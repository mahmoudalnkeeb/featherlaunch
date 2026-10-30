// SPDX-License-Identifier: GPL-3.0-only
//! Mojang version metadata: manifest, version JSON, OS rules, libraries and assets.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::net::{self, Download};
use crate::paths;

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const LIBRARIES_URL: &str = "https://libraries.minecraft.net/";
pub const RESOURCES_URL: &str = "https://resources.download.minecraft.net/";

pub const OS_NAME: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "osx"
} else {
    "linux"
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionManifest {
    pub versions: Vec<ManifestEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub release_time: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub logging: Option<Logging>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgValue>,
    #[serde(default)]
    pub jvm: Vec<ArgValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: OneOrMany },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl ArgValue {
    pub fn values(&self) -> Vec<String> {
        match self {
            ArgValue::Plain(s) => vec![s.clone()],
            ArgValue::Ruled { rules, value } if rules_allow(rules) => match value {
                OneOrMany::One(s) => vec![s.clone()],
                OneOrMany::Many(v) => v.clone(),
            },
            ArgValue::Ruled { .. } => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

impl Rule {
    fn matches(&self) -> bool {
        if let Some(os) = &self.os {
            if os.name.as_deref().is_some_and(|n| n != OS_NAME) {
                return false;
            }
            // Mojang uses "x86" for 32-bit only; this launcher targets 64-bit x86.
            if os.arch.as_deref().is_some_and(|a| a != "x86_64" && a != "amd64") {
                return false;
            }
        }
        // No optional features (demo, custom resolution, quick play) are turned on.
        if let Some(features) = &self.features
            && features.values().any(|wanted| *wanted)
        {
            return false;
        }
        true
    }
}

/// Mojang rule semantics: with rules present, the last matching rule decides; default deny.
pub fn rules_allow(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches() {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub major_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LogFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogFile {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

/// A library file to fetch. `native` jars are extracted; the rest go on the classpath.
#[derive(Clone, Debug)]
pub struct LibraryFile {
    pub download: Download,
    pub native: bool,
}

/// `group:artifact:version[:classifier][@ext]` to a repository-relative path.
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = coords.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    let file = match parts.get(3) {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{}/{artifact}/{version}/{file}", group.replace('.', "/")))
}

/// Identity used to de-duplicate libraries: group, artifact and classifier, without version.
fn library_key(name: &str) -> String {
    let coords = name.split('@').next().unwrap_or(name);
    let parts: Vec<&str> = coords.split(':').collect();
    match parts.as_slice() {
        [g, a, _, c, ..] => format!("{g}:{a}:{c}"),
        [g, a, ..] => format!("{g}:{a}"),
        _ => name.to_string(),
    }
}

fn classifier(name: &str) -> Option<&str> {
    name.split('@').next().unwrap_or(name).split(':').nth(3)
}

/// Native classifiers for other CPU architectures would overwrite the x64 libraries.
fn wrong_arch(classifier: &str) -> bool {
    classifier.contains("arm64") || classifier.ends_with("-x86") || classifier.contains("aarch64")
}

impl Library {
    pub fn files(&self) -> Vec<LibraryFile> {
        if !rules_allow(self.rules.as_deref().unwrap_or(&[])) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let lib_dir = paths::libraries();
        let class = classifier(&self.name);
        if class.is_some_and(wrong_arch) {
            return out;
        }
        let is_native = class.is_some_and(|c| c.starts_with("natives-"));

        let artifact = self.downloads.as_ref().and_then(|d| d.artifact.as_ref());
        if let Some(a) = artifact {
            if let Some(rel) = a.path.clone().or_else(|| maven_path(&self.name)) {
                out.push(LibraryFile {
                    download: Download { url: a.url.clone(), path: lib_dir.join(rel), sha1: a.sha1.clone(), size: a.size },
                    native: is_native,
                });
            }
        } else if self.natives.is_none()
            && let Some(rel) = maven_path(&self.name)
        {
            let base = self.url.as_deref().unwrap_or(LIBRARIES_URL);
            let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
            out.push(LibraryFile {
                download: Download { url: format!("{base}{rel}"), path: lib_dir.join(rel), sha1: self.sha1.clone(), size: self.size },
                native: is_native,
            });
        }

        // Pre-1.19 style natives: a classifier per OS, extracted rather than put on the classpath.
        if let Some(key) = self.natives.as_ref().and_then(|n| n.get(OS_NAME)) {
            let key = key.replace("${arch}", "64");
            let classifiers = self.downloads.as_ref().and_then(|d| d.classifiers.as_ref());
            if let Some(a) = classifiers.and_then(|c| c.get(&key)) {
                let rel = a.path.clone().or_else(|| maven_path(&format!("{}:{key}", self.name)));
                if let Some(rel) = rel {
                    out.push(LibraryFile {
                        download: Download { url: a.url.clone(), path: lib_dir.join(rel), sha1: a.sha1.clone(), size: a.size },
                        native: true,
                    });
                }
            }
        }
        out
    }
}

pub fn merge(parent: VersionJson, child: VersionJson) -> VersionJson {
    let child_keys: HashSet<String> = child.libraries.iter().map(|l| library_key(&l.name)).collect();
    let mut libraries = child.libraries;
    libraries.extend(parent.libraries.into_iter().filter(|l| !child_keys.contains(&library_key(&l.name))));

    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut p), Some(c)) => {
            p.game.extend(c.game);
            p.jvm.extend(c.jvm);
            Some(p)
        }
        (p, c) => c.or(p),
    };

    VersionJson {
        id: child.id,
        inherits_from: None,
        main_class: child.main_class.or(parent.main_class),
        arguments,
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        libraries,
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
        kind: parent.kind.or(child.kind),
    }
}

pub async fn manifest() -> Result<VersionManifest> {
    let cache = paths::cache().join("version_manifest_v2.json");
    match net::get_json::<VersionManifest>(MANIFEST_URL).await {
        Ok(m) => {
            let _ = std::fs::write(&cache, serde_json::to_vec(&m)?);
            Ok(m)
        }
        Err(e) => std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(e),
    }
}

pub fn version_json_path(id: &str) -> PathBuf {
    paths::versions().join(id).join(format!("{id}.json"))
}

pub fn load_local(id: &str) -> Result<VersionJson> {
    read_profile(&version_json_path(id))
}

fn read_profile(path: &std::path::Path) -> Result<VersionJson> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn is_missing(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// Loads a vanilla version JSON, downloading it once. Cached copies keep offline play working.
pub async fn ensure_vanilla(id: &str) -> Result<VersionJson> {
    if let Ok(v) = load_local(id) {
        return Ok(v);
    }
    let manifest = manifest().await?;
    let entry = manifest.versions.iter().find(|v| v.id == id).ok_or_else(|| anyhow!("Minecraft version {id} not found"))?;
    let mut d = Download::new(&entry.url, version_json_path(id));
    d.sha1 = entry.sha1.clone();
    net::download(&d).await?;
    load_local(id)
}

pub async fn resolve(id: &str) -> Result<VersionJson> {
    let mut chain = Vec::new();
    let mut next = Some(id.to_string());
    while let Some(id) = next {
        // Only a missing file is worth downloading: a damaged loader profile isn't in Mojang's
        // manifest, and the error from `load_local` names the file.
        let v = match load_local(&id) {
            Ok(v) => v,
            Err(e) if is_missing(&e) => ensure_vanilla(&id).await?,
            Err(e) => return Err(e),
        };
        next = v.inherits_from.clone();
        chain.push(v);
        if chain.len() > 8 {
            anyhow::bail!("version inheritance loop at {id}");
        }
    }
    let mut merged = chain.pop().unwrap();
    while let Some(child) = chain.pop() {
        merged = merge(merged, child);
    }
    Ok(merged)
}

#[derive(Clone, Debug, Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
    #[serde(default)]
    pub map_to_resources: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

impl AssetObject {
    pub fn rel_path(&self) -> String {
        format!("{}/{}", &self.hash[..2], self.hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(maven_path("net.fabricmc:fabric-loader:0.16.9").unwrap(), "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
        assert_eq!(maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows").unwrap(), "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar");
        assert_eq!(maven_path("a.b:c:1@zip").unwrap(), "a/b/c/1/c-1.zip");
        assert!(maven_path("broken").is_none());
    }

    fn rule(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn os_rules() {
        assert!(rules_allow(&[]));
        assert!(rules_allow(&rule(r#"[{"action":"allow"}]"#)));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","os":{"name":"osx"}}]"#)));
        assert!(rules_allow(&rule(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#)));
        let disallow_here = format!(r#"[{{"action":"allow"}},{{"action":"disallow","os":{{"name":"{OS_NAME}"}}}}]"#);
        assert!(!rules_allow(&rule(&disallow_here)));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","os":{"arch":"x86"}}]"#)));
        assert!(!rules_allow(&rule(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#)));
    }

    #[test]
    fn only_a_missing_profile_counts_as_missing() {
        let path = std::env::temp_dir().join(format!("featherlaunch-test-{}.json", uuid::Uuid::new_v4().simple()));
        assert!(is_missing(&read_profile(&path).unwrap_err()));

        std::fs::write(&path, r#"{"id":"#).unwrap();
        let error = read_profile(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(!is_missing(&error));
        assert!(error.to_string().contains(&path.display().to_string()), "{error:#}");
    }

    #[test]
    fn merge_prefers_child_libraries() {
        let lib = |n: &str| Library { name: n.into(), downloads: None, url: None, sha1: None, size: None, natives: None, rules: None };
        let parent = VersionJson {
            id: "1.20.1".into(),
            main_class: Some("net.minecraft.client.main.Main".into()),
            libraries: vec![lib("org.ow2.asm:asm:9.3"), lib("com.google.guava:guava:31")],
            ..Default::default()
        };
        let child = VersionJson {
            id: "fabric".into(),
            inherits_from: Some("1.20.1".into()),
            main_class: Some("net.fabricmc.loader.impl.launch.knot.KnotClient".into()),
            libraries: vec![lib("org.ow2.asm:asm:9.7")],
            ..Default::default()
        };
        let m = merge(parent, child);
        assert_eq!(m.id, "fabric");
        assert_eq!(m.main_class.unwrap(), "net.fabricmc.loader.impl.launch.knot.KnotClient");
        let names: Vec<_> = m.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["org.ow2.asm:asm:9.7", "com.google.guava:guava:31"]);
    }

    #[test]
    fn skips_other_arch_natives() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl:3.3.3:natives-windows-arm64",
                "downloads":{"artifact":{"path":"x.jar","url":"u","sha1":"s","size":1}}}"#,
        )
        .unwrap();
        assert!(lib.files().is_empty());
    }
}
