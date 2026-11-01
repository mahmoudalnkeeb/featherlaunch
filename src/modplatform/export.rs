// SPDX-License-Identifier: GPL-3.0-only
//! Exporting an instance: a zip of the whole instance, or a Modrinth pack.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::minecraft::instance::{ExportInfo, Instance, Loader};
use crate::minecraft::meta::loaders;
use crate::modplatform::modrinth::{self, ContentKind};
use crate::modplatform::mrpack;
use crate::net::{self, RemoveOnDrop};
use crate::tasks::Progress;

/// Folders that may hold files Modrinth also hosts. A pack links to those instead of copying them.
const LINKED_FOLDERS: [ContentKind; 3] = [ContentKind::Mod, ContentKind::ResourcePack, ContentKind::ShaderPack];
/// Beside `.minecraft` in an instance folder.
const INSTANCE_FILES: [&str; 3] = ["instance.json", "content.json", crate::icons::INSTANCE_ICON];

/// What's in the game folder, for choosing what to include.
pub fn top_level(game_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(game_dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Things that only make sense on this PC, or that nobody receiving a copy wants.
pub fn included_by_default(name: &str, pack: bool) -> bool {
    !matches!(name, "logs" | "crash-reports" | ".mixin.out" | ".cache" | "command_history.txt" | "usercache.json")
        && !(pack && matches!(name, "saves" | "screenshots" | "servers.dat"))
}

fn walk(root: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
    let path = root.join(rel);
    if !path.is_dir() {
        out.push(rel.to_string());
        return Ok(());
    }
    for entry in std::fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
        let name = entry?.file_name();
        walk(root, &format!("{rel}/{}", name.to_string_lossy()), out)?;
    }
    Ok(())
}

/// Every file under the chosen top-level names, as paths with `/`.
fn chosen_files(game_dir: &Path, include: &[String]) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for name in include {
        walk(game_dir, name, &mut files)?;
    }
    Ok(files)
}

fn options(name: &str) -> SimpleFileOptions {
    // Jars and zips are compressed already, and old CPUs would spend minutes getting nothing back.
    let packed = [".jar", ".zip", ".gz", ".png", ".mca"].iter().any(|e| name.ends_with(e));
    let method = if packed { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
    SimpleFileOptions::default().compression_method(method).large_file(true)
}

fn add_file(zip: &mut ZipWriter<std::fs::File>, name: &str, file: &Path) -> Result<()> {
    zip.start_file(name, options(name))?;
    std::io::copy(&mut std::fs::File::open(file).with_context(|| format!("reading {}", file.display()))?, zip)?;
    Ok(())
}

/// Writes through a `.part` file, so a failed export leaves nothing that looks finished.
fn write_zip(target: &Path, fill: impl FnOnce(&mut ZipWriter<std::fs::File>) -> Result<()>) -> Result<()> {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let _part = RemoveOnDrop(tmp.clone());
    let mut zip = ZipWriter::new(std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?);
    fill(&mut zip)?;
    zip.finish()?;
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// The instance folder's own files and the chosen part of `.minecraft`.
pub fn export_zip(dir: &Path, include: &[String], target: &Path) -> Result<()> {
    let game_dir = dir.join(".minecraft");
    let files = chosen_files(&game_dir, include)?;
    write_zip(target, |zip| {
        for name in INSTANCE_FILES.iter().filter(|n| dir.join(n).is_file()) {
            add_file(zip, name, &dir.join(name))?;
        }
        for rel in &files {
            add_file(zip, &format!(".minecraft/{rel}"), &game_dir.join(rel))?;
        }
        Ok(())
    })
}

/// The `dependencies` of a pack index. Packs must name an exact loader version.
fn dependencies(inst: &Instance, loader_version: Option<&str>) -> Result<HashMap<String, String>> {
    let mut deps = HashMap::from([("minecraft".to_string(), inst.mc_version.clone())]);
    if let Some((key, _)) = mrpack::LOADER_KEYS.iter().find(|(_, l)| *l == inst.loader) {
        deps.insert(key.to_string(), loader_version.context("no version of the mod loader was found")?.to_string());
    }
    Ok(deps)
}

fn pack_index(inst: &Instance, info: &ExportInfo, files: Vec<Value>, dependencies: HashMap<String, String>) -> Value {
    let version = if info.version.trim().is_empty() { "1.0.0" } else { info.version.trim() };
    let mut index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": version,
        "name": inst.name,
        "files": files,
        "dependencies": dependencies,
    });
    if !info.summary.trim().is_empty() {
        index["summary"] = json!(info.summary.trim());
    }
    index
}

/// A `.mrpack` of the chosen files. Files Modrinth hosts become links; the rest go in `overrides`.
pub async fn export_mrpack(inst: &Instance, include: Vec<String>, info: &ExportInfo, target: PathBuf, progress: &Progress) -> Result<()> {
    progress.set_label("Reading files");
    let game_dir = inst.game_dir();
    let dir = game_dir.clone();
    let hashed: Vec<(String, Option<String>)> = tokio::task::spawn_blocking(move || -> Result<_> {
        let linkable = |rel: &str| LINKED_FOLDERS.iter().any(|k| rel.starts_with(&format!("{}/", k.folder())));
        let files = chosen_files(&dir, &include)?;
        Ok(files
            .into_iter()
            .map(|rel| {
                let sha1 = linkable(&rel).then(|| net::sha1_file(&dir.join(&rel)).ok()).flatten();
                (rel, sha1)
            })
            .collect())
    })
    .await??;

    progress.set_label("Looking up files on Modrinth");
    let hashes: Vec<&String> = hashed.iter().filter_map(|(_, h)| h.as_ref()).collect();
    let found = if hashes.is_empty() { HashMap::new() } else { modrinth::versions_by_hash(&hashes).await? };
    let loader_version = match (&inst.loader_version, inst.loader) {
        (_, Loader::Vanilla) => None,
        (Some(v), _) => Some(v.clone()),
        (None, loader) => loaders::versions(loader, &inst.mc_version).await?.into_iter().next(),
    };
    let deps = dependencies(inst, loader_version.as_deref())?;

    let mut files = Vec::new();
    let mut overrides = Vec::new();
    for (rel, hash) in hashed {
        let file = hash
            .as_ref()
            .and_then(|h| Some((h, found.get(h)?)))
            .and_then(|(h, v)| v.files.iter().find(|f| f.hashes.get("sha1") == Some(h)));
        match file {
            Some(f) => files.push(json!({
                "path": rel,
                "hashes": { "sha1": f.hashes.get("sha1"), "sha512": f.hashes.get("sha512") },
                "downloads": [f.url],
                "fileSize": f.size,
            })),
            None => overrides.push(rel),
        }
    }
    let index = serde_json::to_vec_pretty(&pack_index(inst, info, files, deps))?;

    progress.set_label("Writing the pack");
    tokio::task::spawn_blocking(move || {
        write_zip(&target, |zip| {
            zip.start_file("modrinth.index.json", options("modrinth.index.json"))?;
            zip.write_all(&index)?;
            for rel in &overrides {
                add_file(zip, &format!("overrides/{rel}"), &game_dir.join(rel))?;
            }
            Ok(())
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_zip_holds_the_instance_and_the_chosen_folders() {
        let dir = temp_dir();
        let game = dir.join(".minecraft");
        std::fs::create_dir_all(game.join("mods/nested")).unwrap();
        std::fs::create_dir_all(game.join("logs")).unwrap();
        std::fs::write(dir.join("instance.json"), "{}").unwrap();
        std::fs::write(game.join("options.txt"), "fov:1").unwrap();
        std::fs::write(game.join("mods/nested/a.jar"), "jar").unwrap();
        std::fs::write(game.join("logs/latest.log"), "log").unwrap();

        let target = dir.join("out.zip");
        export_zip(&dir, &["mods".to_string(), "options.txt".to_string()], &target).unwrap();
        let archive = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
        let mut names: Vec<&str> = archive.file_names().collect();
        names.sort();
        assert_eq!(names, [".minecraft/mods/nested/a.jar", ".minecraft/options.txt", "instance.json"]);
        assert!(!dir.join("out.zip.part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_export_leaves_nothing_behind() {
        let dir = temp_dir();
        let target = dir.join("out.zip");
        assert!(export_zip(&dir, &["missing".to_string()], &target).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pack_index_round_trips() {
        let inst = Instance { name: "Mine".into(), mc_version: "1.21.1".into(), loader: Loader::Fabric, ..Default::default() };
        let info = ExportInfo { version: "".into(), summary: " My pack ".into() };
        let index = pack_index(&inst, &info, Vec::new(), dependencies(&inst, Some("0.16.9")).unwrap());
        assert_eq!(index["summary"], "My pack");
        let parsed: mrpack::PackIndex = serde_json::from_value(index).unwrap();
        assert_eq!((parsed.name.as_str(), parsed.version_id.as_str()), ("Mine", "1.0.0"));
        let (mc, loader, version) = mrpack::loader_of(&parsed.dependencies).unwrap();
        assert_eq!((mc.as_str(), loader, version.as_deref()), ("1.21.1", Loader::Fabric, Some("0.16.9")));
        assert!(dependencies(&inst, None).is_err(), "a pack can't leave the loader version open");
    }

    #[test]
    fn saves_stay_out_of_packs_by_default() {
        assert!(included_by_default("saves", false));
        assert!(!included_by_default("saves", true));
        assert!(!included_by_default("logs", false));
        assert!(included_by_default("mods", true));
    }
}
