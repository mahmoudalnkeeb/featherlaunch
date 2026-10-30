// SPDX-License-Identifier: GPL-3.0-only
//! Turns an instance into a running game: downloads, natives, arguments, process and logs.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, PipeReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::oneshot;

use crate::config::Config;
use crate::java;
use crate::minecraft::auth::{Account, AccountKind};
use crate::minecraft::instance::Instance;
use crate::minecraft::meta::{loaders, mojang};
use crate::net::{self, Download};
use crate::paths;
use crate::tasks::Progress;

pub struct Prepared {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub game_dir: PathBuf,
}

pub async fn prepare(inst: &Instance, account: &Account, offline: bool, cfg: &Config, progress: &Progress) -> Result<Prepared> {
    let parallel = cfg.parallel_downloads;
    progress.set_label(format!("Preparing Minecraft {}", inst.mc_version));
    let vanilla = mojang::ensure_vanilla(&inst.mc_version).await?;

    let required = vanilla.java_version.as_ref().map_or(8, |j| j.major_version);
    let java = match inst.java_path.as_ref().or(cfg.java_path.as_ref()) {
        Some(p) => PathBuf::from(p),
        None => java::ensure(required, progress).await?,
    };

    // Vanilla files first: Forge's installer expects the client jar and libraries present.
    download_version_files(&vanilla, &inst.mc_version, parallel, progress).await?;
    let version_id = loaders::ensure(inst.loader, &inst.mc_version, inst.loader_version.as_deref(), &java, progress).await?;
    let version = mojang::resolve(&version_id).await?;
    if version_id != inst.mc_version {
        download_version_files(&version, &inst.mc_version, parallel, progress).await?;
    }

    progress.set_label("Checking assets");
    let game_dir = inst.game_dir();
    std::fs::create_dir_all(&game_dir)?;
    let (assets_root, game_assets, index_id) = prepare_assets(&version, &game_dir, parallel, progress).await?;

    let natives_dir = paths::natives().join(&version_id);
    let libraries: Vec<mojang::LibraryFile> = version.libraries.iter().flat_map(|l| l.files()).collect();
    progress.set_label("Extracting natives");
    let native_jars: Vec<PathBuf> = libraries.iter().filter(|l| l.native).map(|l| l.download.path.clone()).collect();
    let nd = natives_dir.clone();
    tokio::task::spawn_blocking(move || extract_natives(&native_jars, &nd)).await??;

    let mut seen = HashSet::new();
    let mut classpath: Vec<String> = libraries
        .iter()
        .filter(|l| !l.native || l.download.path.to_string_lossy().contains("lwjgl"))
        .map(|l| l.download.path.to_string_lossy().into_owned())
        .filter(|p| seen.insert(p.clone()))
        .collect();
    classpath.push(client_jar(&inst.mc_version).to_string_lossy().into_owned());

    let (name, uuid, token, user_type) = match (account.kind, offline) {
        (AccountKind::Microsoft, false) => {
            (account.name.clone(), account.id.clone(), account.token.clone().context("account is not signed in")?, "msa")
        }
        _ => (account.name.clone(), account.id.clone(), "0".to_string(), "legacy"),
    };

    let vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", name),
        ("version_name", version_id.clone()),
        ("game_directory", game_dir.to_string_lossy().into_owned()),
        ("assets_root", assets_root.to_string_lossy().into_owned()),
        ("game_assets", game_assets.to_string_lossy().into_owned()),
        ("assets_index_name", index_id),
        ("auth_uuid", uuid.clone()),
        ("auth_access_token", token.clone()),
        ("auth_session", format!("token:{token}:{uuid}")),
        ("clientid", String::new()),
        ("auth_xuid", account.xuid.clone()),
        ("user_type", user_type.to_string()),
        ("user_properties", "{}".to_string()),
        ("version_type", version.kind.clone().unwrap_or_else(|| "release".into())),
        ("natives_directory", natives_dir.to_string_lossy().into_owned()),
        ("launcher_name", "featherlaunch".to_string()),
        ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
        ("classpath", classpath.join(CLASSPATH_SEPARATOR)),
        ("classpath_separator", CLASSPATH_SEPARATOR.to_string()),
        ("library_directory", paths::libraries().to_string_lossy().into_owned()),
    ]);

    let memory = inst.memory_mb.unwrap_or(cfg.memory_mb);
    let mut args = vec![format!("-Xmx{memory}m"), "-Xms512m".to_string()];
    let extra = inst.jvm_args.as_deref().unwrap_or(&cfg.jvm_args);
    args.extend(extra.split_whitespace().map(String::from));
    args.push("-Dlog4j2.formatMsgNoLookups=true".into());

    match &version.arguments {
        Some(a) if !a.jvm.is_empty() => {
            args.extend(a.jvm.iter().flat_map(|v| v.values()).map(|s| substitute(&s, &vars)));
        }
        _ => {
            args.push(format!("-Djava.library.path={}", vars["natives_directory"]));
            // Vanilla's workaround that makes Intel's Windows drivers use their fast path.
            if cfg!(windows) {
                args.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".into());
            }
            args.push("-cp".into());
            args.push(vars["classpath"].clone());
        }
    }

    if let Some(log) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let path = paths::assets().join("log_configs").join(&log.file.id);
        let mut d = Download::new(&log.file.url, &path);
        d.sha1 = log.file.sha1.clone();
        d.size = log.file.size;
        if net::download(&d).await.is_ok() {
            args.push(log.argument.replace("${path}", &path.to_string_lossy()));
        }
    }

    args.push(version.main_class.clone().context("version has no main class")?);
    match (&version.arguments, &version.minecraft_arguments) {
        (_, Some(legacy)) => args.extend(legacy.split_whitespace().map(|s| substitute(s, &vars))),
        (Some(a), None) => args.extend(a.game.iter().flat_map(|v| v.values()).map(|s| substitute(&s, &vars))),
        (None, None) => {}
    }

    Ok(Prepared { java, args, game_dir })
}

fn client_jar(mc: &str) -> PathBuf {
    paths::versions().join(mc).join(format!("{mc}.jar"))
}

async fn download_version_files(v: &mojang::VersionJson, mc: &str, parallel: usize, progress: &Progress) -> Result<()> {
    progress.set_label("Downloading libraries");
    let mut items: Vec<Download> = v.libraries.iter().flat_map(|l| l.files()).map(|f| f.download).collect();
    if let Some(client) = v.downloads.as_ref().and_then(|d| d.client.as_ref()) {
        let mut d = Download::new(&client.url, client_jar(mc));
        d.sha1 = client.sha1.clone();
        d.size = client.size;
        items.push(d);
    }
    // Forge lists files its installer generates locally; they have no URL to fetch.
    items.retain(|d| !d.url.is_empty() || !d.path.exists());
    net::download_all(items, parallel, progress).await
}

/// Returns (assets root, legacy game assets dir, index id).
async fn prepare_assets(
    v: &mojang::VersionJson,
    game_dir: &Path,
    parallel: usize,
    progress: &Progress,
) -> Result<(PathBuf, PathBuf, String)> {
    let root = paths::assets();
    let Some(index_ref) = &v.asset_index else {
        return Ok((root.clone(), root, "legacy".into()));
    };
    let index_path = root.join("indexes").join(format!("{}.json", index_ref.id));
    let mut d = Download::new(&index_ref.url, &index_path);
    d.sha1 = index_ref.sha1.clone();
    d.size = index_ref.size;
    net::download(&d).await?;
    let index: mojang::AssetIndex = serde_json::from_slice(&std::fs::read(&index_path)?)?;

    progress.set_label("Downloading assets");
    let objects = root.join("objects");
    let items: Vec<Download> = index
        .objects
        .values()
        .map(|o| Download {
            url: format!("{}{}", mojang::RESOURCES_URL, o.rel_path()),
            path: objects.join(o.rel_path()),
            sha1: Some(o.hash.clone()),
            size: Some(o.size),
        })
        .collect();
    net::download_all(items, parallel, progress).await?;

    // Pre-1.7 versions read assets by name from a plain folder tree.
    let game_assets = if index.map_to_resources {
        game_dir.join("resources")
    } else if index.is_virtual {
        root.join("virtual").join(&index_ref.id)
    } else {
        root.join("virtual").join("legacy")
    };
    if index.is_virtual || index.map_to_resources {
        // Thousands of small copies, so run them off the two async workers.
        let target_root = game_assets.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            for (name, obj) in &index.objects {
                let target = target_root.join(name);
                if !target.exists() {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(objects.join(obj.rel_path()), &target)?;
                }
            }
            Ok(())
        })
        .await??;
    }
    Ok((root, game_assets, index_ref.id.clone()))
}

const CLASSPATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

const NATIVE_EXTENSIONS: &[&str] = if cfg!(windows) {
    &[".dll"]
} else if cfg!(target_os = "macos") {
    &[".dylib", ".jnilib"]
} else {
    &[".so"]
};

fn extract_natives(jars: &[PathBuf], dir: &Path) -> Result<()> {
    if dir.join(".done").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for jar in jars {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(jar).with_context(|| format!("opening {}", jar.display()))?)?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let Some(name) = file.enclosed_name() else { continue };
            let lower = name.to_string_lossy().to_lowercase();
            if file.is_dir() || lower.starts_with("meta-inf") || !NATIVE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
                continue;
            }
            // Flatten: LWJGL 3 jars keep libraries under <os>/x64/org/lwjgl/.
            let Some(file_name) = name.file_name() else { continue };
            let mut out = std::fs::File::create(dir.join(file_name))?;
            std::io::copy(&mut file, &mut out)?;
        }
    }
    std::fs::write(dir.join(".done"), b"")?;
    Ok(())
}

pub fn substitute(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 3 + end]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: Level,
    pub text: String,
}

/// Turns stdout into log lines. Versions with a log4j config print XML events; others print text.
#[derive(Default)]
pub struct LogParser {
    event: Option<String>,
}

fn guess_level(line: &str) -> Level {
    if line.contains("ERROR") || line.contains("FATAL") || line.contains("Exception") || line.starts_with("\tat ") {
        Level::Error
    } else if line.contains("WARN") {
        Level::Warn
    } else if line.contains("DEBUG") {
        Level::Debug
    } else {
        Level::Info
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

impl LogParser {
    pub fn feed(&mut self, line: &str) -> Option<LogLine> {
        if self.event.is_none() && line.trim_start().starts_with("<log4j:Event") {
            self.event = Some(String::new());
        }
        let Some(buf) = self.event.as_mut() else {
            return Some(LogLine { level: guess_level(line), text: line.to_string() });
        };
        buf.push_str(line);
        buf.push('\n');
        if !line.contains("</log4j:Event>") {
            return None;
        }
        let xml = self.event.take().unwrap();
        let level = match attr(&xml, "level").unwrap_or("INFO") {
            "WARN" => Level::Warn,
            "ERROR" | "FATAL" => Level::Error,
            "DEBUG" | "TRACE" => Level::Debug,
            _ => Level::Info,
        };
        let thread = attr(&xml, "thread").unwrap_or("main");
        let mut parts = Vec::new();
        let mut rest = xml.as_str();
        while let Some(s) = rest.find("<![CDATA[") {
            let body = &rest[s + 9..];
            let e = body.find("]]>").unwrap_or(body.len());
            parts.push(body[..e].trim_end().to_string());
            rest = &body[e..];
        }
        let level_name = match level {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Debug => "DEBUG",
        };
        Some(LogLine { level, text: format!("[{thread}/{level_name}] {}", parts.join("\n")) })
    }
}

pub type LineSink = Arc<dyn Fn(LogLine) + Send + Sync>;

/// Starts the game and returns a sender that kills it when sent `()`.
pub fn spawn(prep: Prepared, on_line: LineSink, on_exit: impl FnOnce(Option<i32>) + Send + 'static) -> Result<oneshot::Sender<()>> {
    let (stdout, stdout_writer) = std::io::pipe().context("creating the game's stdout pipe")?;
    let (stderr, stderr_writer) = std::io::pipe().context("creating the game's stderr pipe")?;
    let mut cmd = tokio::process::Command::new(&prep.java);
    cmd.args(&prep.args).current_dir(&prep.game_dir).stdin(Stdio::null()).stdout(stdout_writer).stderr(stderr_writer);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().with_context(|| format!("starting {}", prep.java.display()))?;

    on_line(LogLine { level: Level::Debug, text: format!("{} {}", prep.java.display(), redact(&prep.args)) });

    let sink = on_line.clone();
    let mut parser = LogParser::default();
    forward_lines(stdout, move |line| {
        if let Some(l) = parser.feed(&line) {
            sink(l);
        }
    });
    let sink = on_line.clone();
    forward_lines(stderr, move |line| sink(LogLine { level: Level::Error, text: line }));

    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        // A dropped sender (launcher closing) must not kill the game; only an explicit send does.
        let code = tokio::select! {
            status = child.wait() => status.ok().and_then(|s| s.code()),
            Ok(()) = kill_rx => {
                let _ = child.kill().await;
                None
            }
        };
        on_exit(code);
    });
    Ok(kill_tx)
}

/// Passes on each line of `pipe` until it closes. This is a plain thread because tokio reads child
/// pipes on its blocking pool on Windows, and dropping the runtime waits for that pool. A read of
/// the game's stderr would hang the launcher on close until the game exits.
fn forward_lines(pipe: PipeReader, mut on_line: impl FnMut(String) + Send + 'static) {
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            on_line(line);
        }
    });
}

/// The command line is shown in the console, so the access token is masked.
fn redact(args: &[String]) -> String {
    let mut out = Vec::with_capacity(args.len());
    let mut hide_next = false;
    for a in args {
        if hide_next {
            out.push("<hidden>".to_string());
            hide_next = false;
        } else {
            hide_next = a == "--accessToken" || a == "--session";
            out.push(if a.len() > 300 { format!("{}…", shorten(a, 120)) } else { a.clone() });
        }
    }
    out.join(" ")
}

/// The first `chars` characters. Cutting by bytes could split a multi-byte character in a path.
fn shorten(text: &str, chars: usize) -> &str {
    &text[..text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_shortens_arguments_in_any_language() {
        // Multi-byte characters straddle byte 120 at every offset.
        for pad in 0..4 {
            let path = format!("{}{}", "a".repeat(pad), "مجلد/".repeat(100));
            let shown = redact(&["-cp".to_string(), path.clone()]);
            assert_eq!(shown, format!("-cp {}…", path.chars().take(120).collect::<String>()));
        }
    }

    #[test]
    fn redact_keeps_short_arguments_and_hides_tokens() {
        let args = ["--accessToken", "secret", "--session", "secret", "--username", "Steve"].map(String::from);
        assert_eq!(redact(&args), "--accessToken <hidden> --session <hidden> --username Steve");
    }

    #[test]
    fn substitutes_known_and_keeps_unknown() {
        let vars = HashMap::from([("a", "1".to_string()), ("b", "two".to_string())]);
        assert_eq!(substitute("${a}-${b}", &vars), "1-two");
        assert_eq!(substitute("x${missing}y", &vars), "x${missing}y");
        assert_eq!(substitute("-Djava.library.path=${a}", &vars), "-Djava.library.path=1");
        assert_eq!(substitute("plain", &vars), "plain");
        assert_eq!(substitute("broken${a", &vars), "broken${a");
    }

    #[test]
    fn parses_log4j_xml_events() {
        let mut p = LogParser::default();
        assert!(p.feed(r#"<log4j:Event logger="x" timestamp="1" level="WARN" thread="Render thread">"#).is_none());
        assert!(p.feed("<log4j:Message><![CDATA[Something odd]]></log4j:Message>").is_none());
        let line = p.feed("</log4j:Event>").unwrap();
        assert_eq!(line.level, Level::Warn);
        assert_eq!(line.text, "[Render thread/WARN] Something odd");
        let plain = p.feed("[12:00:00] [main/ERROR]: boom").unwrap();
        assert_eq!(plain.level, Level::Error);
    }

    #[test]
    fn hides_access_token() {
        let args: Vec<String> = ["--username", "Steve", "--accessToken", "secret"].map(String::from).to_vec();
        assert_eq!(redact(&args), "--username Steve --accessToken <hidden>");
    }

    #[test]
    fn a_running_game_does_not_hold_up_closing_the_launcher() {
        // Outlives the wait below and, like the game, writes nothing to stderr.
        #[cfg(windows)]
        let (java, args) = ("ping", ["-n", "11", "127.0.0.1"].map(String::from).to_vec());
        #[cfg(not(windows))]
        let (java, args) = ("sleep", vec!["10".to_string()]);
        let prep = Prepared { java: java.into(), args, game_dir: std::env::temp_dir() };

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _kill = runtime.block_on(async { spawn(prep, Arc::new(|_| {}), |_| {}) }).unwrap();

        let (closed_tx, closed) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(runtime);
            let _ = closed_tx.send(());
        });
        closed.recv_timeout(std::time::Duration::from_secs(5)).expect("shutting down waited for the game");
    }

    #[test]
    fn forwards_both_output_pipes_until_the_game_exits() {
        #[cfg(windows)]
        let (java, args) = ("cmd", ["/c", "echo", "out&", "echo", "err", "1>&2"].map(String::from).to_vec());
        #[cfg(not(windows))]
        let (java, args) = ("sh", ["-c", "echo out; echo err >&2"].map(String::from).to_vec());
        let prep = Prepared { java: java.into(), args, game_dir: std::env::temp_dir() };

        let (line_tx, lines) = std::sync::mpsc::channel();
        let sink: LineSink = Arc::new(move |line| {
            let _ = line_tx.send(line);
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _kill = runtime.block_on(async { spawn(prep, sink, |_| {}) }).unwrap();

        let mut seen = Vec::new();
        loop {
            match lines.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(line) => seen.push(line.text.trim().to_string()),
                // All readers dropped the sink, so the pipes reached EOF.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(e) => panic!("the pipes stayed open: {e}, seen {seen:?}"),
            }
        }
        assert!(seen.iter().any(|l| l == "out"), "{seen:?}");
        assert!(seen.iter().any(|l| l == "err"), "{seen:?}");
    }
}
