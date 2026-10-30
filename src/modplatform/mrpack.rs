// SPDX-License-Identifier: GPL-3.0-only
//! Modrinth modpack (`.mrpack`) import.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::minecraft::instance::{Instance, Loader};
use crate::modplatform::modrinth::{self, Version};
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

pub fn loader_of(deps: &HashMap<String, String>) -> Result<(String, Loader, Option<String>)> {
    let mc = deps.get("minecraft").context("modpack does not name a Minecraft version")?.clone();
    for (key, loader) in
        [("fabric-loader", Loader::Fabric), ("quilt-loader", Loader::Quilt), ("neoforge", Loader::NeoForge), ("forge", Loader::Forge)]
    {
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

fn extract_overrides(pack: &Path, game_dir: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    // client-overrides are applied after overrides so they win.
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            let Some(rel) = name.strip_prefix(prefix) else { continue };
            let Some(rel) = safe_rel_path(rel) else { continue };
            let target = game_dir.join(rel);
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
    let file = version.primary_file().context("modpack version has no file")?;
    let path = paths::temp_pack_path(modrinth::safe_file_name(&file.filename)?);
    // Deleted however the task ends: done, failed or cancelled.
    let _pack_file = net::RemoveOnDrop(path.clone());
    let mut d = Download::new(&file.url, &path);
    d.sha1 = file.hashes.get("sha1").cloned();
    progress.set_label(format!("Downloading {title}"));
    net::download_tracked(&d, Some(progress)).await?;
    let inst = import(path.clone(), group, parallel, progress).await?;
    if let Some(url) = icon_url.filter(|u| !u.is_empty())
        && let Err(e) = icons::download_instance_icon(url, inst.dir()).await
    {
        log::error(&format!("Downloading the icon of {title} failed: {e:#}"));
    }
    Ok(inst)
}

pub async fn import(pack: PathBuf, group: String, parallel: usize, progress: &Progress) -> Result<Instance> {
    progress.set_label("Reading modpack");
    let p = pack.clone();
    let index = tokio::task::spawn_blocking(move || read_index(&p)).await??;
    let (mc, loader, loader_version) = loader_of(&index.dependencies)?;
    let name = if index.version_id.is_empty() { index.name.clone() } else { format!("{} {}", index.name, index.version_id) };
    let inst = Instance { name, group, mc_version: mc, loader, loader_version, ..Default::default() }.create()?;
    let game_dir = inst.game_dir();
    // Deletes the half-made instance on any error, or when the task is cancelled.
    let partial = PartialInstance(Some(inst));

    let mut items = Vec::new();
    for f in &index.files {
        if f.env.as_ref().is_some_and(|e| e.client == "unsupported") {
            continue;
        }
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
    progress.set_label(format!("Downloading {} modpack files", items.len()));
    net::download_all(items, parallel, progress).await?;

    progress.set_label("Applying modpack overrides");
    // Extraction runs on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    tokio::task::spawn_blocking(move || extract_overrides(&pack, &game_dir)).await??;
    Ok(partial.keep())
}

struct PartialInstance(Option<Instance>);

impl PartialInstance {
    fn keep(mut self) -> Instance {
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
