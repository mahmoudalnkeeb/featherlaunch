// SPDX-License-Identifier: GPL-3.0-only
//! Modrinth modpack (`.mrpack`) import.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::minecraft::instance::{Instance, Loader, PackSource};
use crate::modplatform::modrinth::{self, ContentIndex, ContentKind, Version};
use crate::net::{self, Download};
use crate::tasks::Progress;
use crate::{icons, log, paths};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackIndex {
    pub name: String,
    #[serde(default)]
    pub version_id: String,
    pub files: Vec<PackFile>,
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    pub path: String,
    pub hashes: HashMap<String, String>,
    #[serde(default)]
    pub env: Option<PackEnv>,
    pub downloads: Vec<String>,
    #[serde(default)]
    pub file_size: u64,
}

#[derive(Debug, Deserialize)]
pub struct PackEnv {
    #[serde(default)]
    pub client: String,
}

/// The only hosts the Modrinth pack format lets files be downloaded from.
const ALLOWED_HOSTS: [&str; 4] = ["cdn.modrinth.com", "github.com", "raw.githubusercontent.com", "gitlab.com"];

fn allowed_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h)))
}

/// Rejects absolute paths and `..` so a pack cannot write outside the instance.
pub fn safe_rel_path(p: &str) -> Option<PathBuf> {
    // Pack paths use '/'. Backslashes and drive letters only appear in hostile or broken packs,
    // and on Linux they would not be caught by the component check below.
    if p.contains(['\\', ':']) {
        return None;
    }
    let path = Path::new(p);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// How a pack's `dependencies` name each loader.
pub const LOADER_KEYS: [(&str, Loader); 4] =
    [("fabric-loader", Loader::Fabric), ("quilt-loader", Loader::Quilt), ("neoforge", Loader::NeoForge), ("forge", Loader::Forge)];

pub fn loader_of(deps: &HashMap<String, String>) -> Result<(String, Loader, Option<String>)> {
    let mc = deps.get("minecraft").context("modpack does not name a Minecraft version")?.clone();
    for (key, loader) in LOADER_KEYS {
        if let Some(v) = deps.get(key) {
            return Ok((mc, loader, Some(v.clone())));
        }
    }
    Ok((mc, Loader::Vanilla, None))
}

pub fn read_index(pack: &Path) -> Result<PackIndex> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    let mut entry = archive.by_name("modrinth.index.json").context("not a Modrinth modpack (no modrinth.index.json)")?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

/// Files in a pack from outside Modrinth that Modrinth hasn't reviewed: ones hosted elsewhere, and
/// mods bundled in its overrides. Mods run as code on the player's PC.
pub fn unchecked_files(pack: &Path) -> Result<Vec<String>> {
    let index = read_index(pack)?;
    let hosted_elsewhere =
        |f: &&PackFile| f.downloads.iter().any(|u| reqwest::Url::parse(u).map_or(true, |u| u.host_str() != Some("cdn.modrinth.com")));
    let mut files: Vec<String> = index.files.iter().filter(hosted_elsewhere).map(|f| f.path.clone()).collect();
    let archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    files.extend(
        archive
            .file_names()
            .filter(|name| !name.ends_with('/'))
            .filter(|name| ["overrides/mods/", "client-overrides/mods/"].iter().any(|p| name.starts_with(p)))
            .map(str::to_string),
    );
    Ok(files)
}

/// Everything a pack puts in the game folder, as paths with `/`.
fn pack_paths(index: &PackIndex, pack: &Path) -> Result<Vec<String>> {
    let archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    let overrides = archive
        .file_names()
        .filter(|name| !name.ends_with('/'))
        .filter_map(|name| name.strip_prefix("overrides/").or_else(|| name.strip_prefix("client-overrides/")))
        .filter(|rel| safe_rel_path(rel).is_some())
        .map(str::to_string);
    let mut paths: Vec<String> = index.files.iter().filter(|f| wanted(f)).map(|f| f.path.clone()).chain(overrides).collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn wanted(file: &PackFile) -> bool {
    !file.env.as_ref().is_some_and(|e| e.client == "unsupported")
}

/// The files to download into `game_dir`.
fn downloads(index: &PackIndex, game_dir: &Path) -> Result<Vec<Download>> {
    let mut items = Vec::new();
    for f in index.files.iter().filter(|f| wanted(f)) {
        let Some(rel) = safe_rel_path(&f.path) else { bail!("modpack contains an unsafe path: {}", f.path) };
        // Packs are untrusted. Like the official launcher, allow only the hosts the format
        // permits and require a hash, so a pack can't download unverified files from any server.
        let url = f
            .downloads
            .iter()
            .find(|u| allowed_url(u))
            .with_context(|| format!("modpack file {} is not hosted on Modrinth, GitHub or GitLab", f.path))?;
        let sha1 = f.hashes.get("sha1").with_context(|| format!("modpack file {} has no SHA-1 hash", f.path))?;
        items.push(Download { url: url.clone(), path: game_dir.join(rel), sha1: Some(sha1.clone()), size: Some(f.file_size) });
    }
    Ok(items)
}

/// Whether an update must leave this file alone when the player already has it.
fn player_owned(rel: &Path) -> bool {
    rel.starts_with("saves") || rel == Path::new("options.txt") || rel == Path::new("servers.dat")
}

fn extract_overrides(pack: &Path, game_dir: &Path, updating: bool) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    // client-overrides are applied after overrides so they win.
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            let Some(rel) = name.strip_prefix(prefix) else { continue };
            let Some(rel) = safe_rel_path(rel) else { continue };
            let target = game_dir.join(&rel);
            if updating && player_owned(&rel) && target.exists() {
                continue;
            }
            if file.is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::io::copy(&mut file, &mut std::fs::File::create(&target)?)?;
        }
    }
    Ok(())
}

/// Downloads a pack to the cache. The guard deletes it however the task ends: done, failed or cancelled.
async fn download_pack(version: &Version, title: &str, progress: &Progress) -> Result<(PathBuf, net::RemoveOnDrop)> {
    let file = version.primary_file().context("modpack version has no file")?;
    let path = paths::temp_pack_path(modrinth::safe_file_name(&file.filename)?);
    let guard = net::RemoveOnDrop(path.clone());
    let mut d = Download::new(&file.url, &path);
    d.sha1 = file.hashes.get("sha1").cloned();
    progress.set_label(format!("Downloading {title}"));
    net::download_tracked(&d, Some(progress)).await?;
    Ok((path, guard))
}

/// Downloads a modpack version from Modrinth and creates a new instance from it. The pack's icon
/// becomes the instance icon; failing to get it just keeps the default.
pub async fn install_from_modrinth(
    version: &Version,
    title: &str,
    icon_url: Option<&str>,
    group: String,
    parallel: usize,
    progress: &Progress,
) -> Result<Instance> {
    let (path, _pack_file) = download_pack(version, title, progress).await?;
    let mut inst = import(path, group, parallel, progress).await?;
    if let Some(pack) = &mut inst.pack {
        pack.title = title.to_string();
        pack.project_id = Some(version.project_id.clone());
        pack.version_id = Some(version.id.clone());
    }
    inst.save()?;
    if let Some(url) = icon_url.filter(|u| !u.is_empty())
        && let Err(e) = icons::download_instance_icon(url, inst.dir()).await
    {
        log::error(&format!("Downloading the icon of {title} failed: {e:#}"));
    }
    Ok(inst)
}

pub async fn import(pack: PathBuf, group: String, parallel: usize, progress: &Progress) -> Result<Instance> {
    progress.set_label("Reading modpack");
    let (index, files) = read_pack(&pack).await?;
    let (mc, loader, loader_version) = loader_of(&index.dependencies)?;
    let name = if index.version_id.is_empty() { index.name.clone() } else { format!("{} {}", index.name, index.version_id) };
    let source = PackSource { title: index.name.clone(), version: index.version_id.clone(), files, ..Default::default() };
    let inst = Instance { name, group, mc_version: mc, loader, loader_version, pack: Some(source), ..Default::default() }.create()?;
    let game_dir = inst.game_dir();
    // Deletes the half-made instance on any error, or when the task is cancelled.
    let partial = PartialInstance(Some(inst));

    let items = downloads(&index, &game_dir)?;
    progress.set_label(format!("Downloading {} modpack files", items.len()));
    net::download_all(items, parallel, progress).await?;

    progress.set_label("Applying modpack overrides");
    // Extraction runs on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    tokio::task::spawn_blocking(move || extract_overrides(&pack, &game_dir, false)).await??;
    Ok(partial.keep())
}

async fn read_pack(pack: &Path) -> Result<(PackIndex, Vec<String>)> {
    let pack = pack.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let index = read_index(&pack)?;
        let paths = pack_paths(&index, &pack)?;
        Ok((index, paths))
    })
    .await?
}

/// Files of the old version to delete. Only content folders: anything else may hold the player's
/// changes, and is harmless left behind.
fn stale_files(old: &[String], new: &[String]) -> Vec<PathBuf> {
    old.iter()
        .filter(|path| !new.contains(path))
        .filter(|path| {
            [ContentKind::Mod, ContentKind::ResourcePack, ContentKind::ShaderPack]
                .iter()
                .any(|k| path.starts_with(&format!("{}/", k.folder())))
        })
        .filter_map(|path| safe_rel_path(path))
        .collect()
}

/// A file the player disabled stays disabled, rather than coming back beside its `.disabled` copy.
fn is_disabled(path: &Path) -> bool {
    let mut disabled = path.as_os_str().to_owned();
    disabled.push(".disabled");
    Path::new(&disabled).exists()
}

/// Moves an instance to another version of its pack, keeping worlds and the player's own files.
/// The old files stay until the new ones are all on disk, so a failed download changes nothing.
pub async fn update(
    mut inst: Instance,
    pack: PathBuf,
    version_id: Option<String>,
    parallel: usize,
    progress: &Progress,
) -> Result<Instance> {
    progress.set_label("Reading modpack");
    let (index, files) = read_pack(&pack).await?;
    let (mc, loader, loader_version) = loader_of(&index.dependencies)?;
    let game_dir = inst.game_dir();
    let items: Vec<Download> = downloads(&index, &game_dir)?.into_iter().filter(|d| !is_disabled(&d.path)).collect();
    progress.set_label(format!("Downloading {} modpack files", items.len()));
    net::download_all(items, parallel, progress).await?;

    progress.set_label("Applying modpack overrides");
    // Extraction runs on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    let source = inst.pack.take().unwrap_or_default();
    let stale = stale_files(&source.files, &files);
    let (dir, instance) = (game_dir.clone(), inst.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        ContentIndex::update(&instance, |content| {
            for rel in &stale {
                content.files.remove(&rel.to_string_lossy().replace('\\', "/"));
            }
        })?;
        for rel in &stale {
            modrinth::remove_file_variants(&dir.join(rel));
        }
        extract_overrides(&pack, &dir, true)
    })
    .await??;

    let title = if source.title.is_empty() { index.name } else { source.title };
    inst.pack = Some(PackSource { title, version: index.version_id, project_id: source.project_id, version_id, files });
    inst.mc_version = mc;
    inst.loader = loader;
    inst.loader_version = loader_version;
    inst.save()?;
    Ok(inst)
}

/// Like [`update`], with a version from Modrinth.
pub async fn update_from_modrinth(inst: Instance, version: &Version, parallel: usize, progress: &Progress) -> Result<Instance> {
    let title = inst.pack.as_ref().map_or("modpack".to_string(), |p| p.title.clone());
    let (path, _pack_file) = download_pack(version, &title, progress).await?;
    update(inst, path, Some(version.id.clone()), parallel, progress).await
}

/// Deletes a half-made instance unless told to keep it.
pub struct PartialInstance(Option<Instance>);

impl PartialInstance {
    pub fn new(inst: Instance) -> Self {
        Self(Some(inst))
    }

    pub fn get(&mut self) -> &mut Instance {
        self.0.as_mut().expect("not kept yet")
    }

    pub fn keep(mut self) -> Instance {
        self.0.take().expect("kept once")
    }
}

impl Drop for PartialInstance {
    fn drop(&mut self) {
        if let Some(inst) = self.0.take() {
            let _ = inst.delete();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths() {
        assert_eq!(safe_rel_path("mods/sodium.jar").unwrap(), PathBuf::from("mods").join("sodium.jar"));
        assert!(safe_rel_path("../evil.jar").is_none());
        assert!(safe_rel_path("C:\\Windows\\evil.dll").is_none());
        assert!(safe_rel_path("/etc/passwd").is_none());
        assert!(safe_rel_path("").is_none());
    }

    #[test]
    fn only_allowed_download_hosts() {
        assert!(allowed_url("https://cdn.modrinth.com/data/a/b.jar"));
        assert!(allowed_url("https://github.com/x/y/releases/download/1/a.jar"));
        assert!(!allowed_url("http://cdn.modrinth.com/a.jar"));
        assert!(!allowed_url("https://cdn.modrinth.com.evil.example/a.jar"));
        assert!(!allowed_url("https://example.com/a.jar"));
        assert!(!allowed_url("not a url"));
    }

    #[test]
    fn reads_loader_dependencies() {
        let deps = HashMap::from([("minecraft".to_string(), "1.20.1".to_string()), ("fabric-loader".to_string(), "0.16.9".to_string())]);
        let (mc, loader, v) = loader_of(&deps).unwrap();
        assert_eq!((mc.as_str(), loader, v.as_deref()), ("1.20.1", Loader::Fabric, Some("0.16.9")));
    }

    #[test]
    fn finds_files_modrinth_has_not_reviewed() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("featherlaunch-test-{}.mrpack", uuid::Uuid::new_v4().simple()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("modrinth.index.json", options).unwrap();
        let index = r#"{"name":"Pack","dependencies":{"minecraft":"1.21"},"files":[
            {"path":"mods/safe.jar","hashes":{},"downloads":["https://cdn.modrinth.com/a.jar"]},
            {"path":"mods/github.jar","hashes":{},"downloads":["https://github.com/a.jar"]}]}"#;
        zip.write_all(index.as_bytes()).unwrap();
        for name in ["overrides/config/a.toml", "overrides/mods/bundled.jar", "client-overrides/mods/client.jar"] {
            zip.start_file(name, options).unwrap();
        }
        zip.finish().unwrap();

        let files = unchecked_files(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files, ["mods/github.jar", "overrides/mods/bundled.jar", "client-overrides/mods/client.jar"]);
    }

    #[test]
    fn updates_remove_only_old_content() {
        let list = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let old = list(&["mods/old.jar", "mods/kept.jar", "config/old.toml", "resourcepacks/old.zip", "../evil.jar"]);
        let new = list(&["mods/kept.jar", "mods/new.jar"]);
        assert_eq!(stale_files(&old, &new), [Path::new("mods/old.jar"), Path::new("resourcepacks/old.zip")]);
    }

    #[test]
    fn updates_keep_worlds_and_settings() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        let game = dir.join("game");
        std::fs::create_dir_all(game.join("saves/w")).unwrap();
        std::fs::write(game.join("options.txt"), "mine").unwrap();
        std::fs::write(game.join("saves/w/level.dat"), "mine").unwrap();
        let pack = dir.join("p.mrpack");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&pack).unwrap());
        for name in ["overrides/options.txt", "overrides/saves/w/level.dat", "overrides/saves/new/level.dat", "overrides/config/a.toml"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"pack").unwrap();
        }
        zip.finish().unwrap();

        extract_overrides(&pack, &game, true).unwrap();
        let read = |rel: &str| std::fs::read_to_string(game.join(rel)).unwrap();
        assert_eq!(read("options.txt"), "mine");
        assert_eq!(read("saves/w/level.dat"), "mine");
        assert_eq!(read("saves/new/level.dat"), "pack");
        assert_eq!(read("config/a.toml"), "pack");

        extract_overrides(&pack, &game, false).unwrap();
        assert_eq!(read("options.txt"), "pack", "a new install takes the pack's files");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_index() {
        let json = r#"{"formatVersion":1,"game":"minecraft","versionId":"5.0","name":"Pack",
            "files":[{"path":"mods/a.jar","hashes":{"sha1":"abc","sha512":"def"},"env":{"client":"required","server":"required"},
            "downloads":["https://cdn.modrinth.com/a.jar"],"fileSize":10}],
            "dependencies":{"minecraft":"1.21.1","neoforge":"21.1.77"}}"#;
        let idx: PackIndex = serde_json::from_str(json).unwrap();
        assert_eq!(idx.files.len(), 1);
        assert_eq!(loader_of(&idx.dependencies).unwrap().1, Loader::NeoForge);
    }
}
