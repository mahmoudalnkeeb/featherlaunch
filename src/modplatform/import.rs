// SPDX-License-Identifier: GPL-3.0-only
//! New instances from files: any pack or instance zip, and other launchers' instance folders.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use zip::ZipArchive;

use crate::log;
use crate::minecraft::instance::{Instance, Loader, PackSource};
use crate::modplatform::curseforge;
use crate::modplatform::modrinth::ContentKind;
use crate::modplatform::mrpack::{self, PartialInstance, safe_rel_path};
use crate::paths;
use crate::tasks::Progress;

#[derive(Debug, PartialEq)]
pub enum ArchiveKind {
    Modrinth,
    CurseForge,
    /// An export from this launcher.
    FeatherLaunch,
    /// Prism Launcher or MultiMC; `root` is the folder in the zip that holds `instance.cfg`.
    Launcher {
        root: String,
    },
}

pub fn detect(path: &Path) -> Result<ArchiveKind> {
    let archive = ZipArchive::new(File::open(path)?).context("that file isn't a zip")?;
    let has = |name: &str| archive.file_names().any(|n| n == name);
    if has("modrinth.index.json") {
        return Ok(ArchiveKind::Modrinth);
    }
    if has("manifest.json") {
        return Ok(ArchiveKind::CurseForge);
    }
    if has("instance.json") {
        return Ok(ArchiveKind::FeatherLaunch);
    }
    let cfg = archive.file_names().filter(|n| n.rsplit('/').next() == Some("instance.cfg")).min_by_key(|n| n.matches('/').count());
    match cfg {
        Some(cfg) => Ok(ArchiveKind::Launcher { root: cfg.trim_end_matches("instance.cfg").to_string() }),
        None => bail!("that file isn't a modpack or an instance FeatherLaunch can import"),
    }
}

/// What the player should confirm before this file is imported. See [`mrpack::unchecked_files`].
/// Other formats have no review to rely on, so any mod in them is listed.
pub fn unchecked_files(path: &Path) -> Result<Vec<String>> {
    if detect(path)? == ArchiveKind::Modrinth {
        return mrpack::unchecked_files(path);
    }
    let archive = ZipArchive::new(File::open(path)?)?;
    Ok(archive.file_names().filter(|n| n.ends_with(".jar")).map(str::to_string).collect())
}

/// Extracts what's under `prefix` into `target`, skipping paths that would leave it.
pub fn extract_under(archive: &mut ZipArchive<File>, prefix: &str, target: &Path) -> Result<()> {
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel) = file.name().strip_prefix(prefix).and_then(safe_rel_path) else { continue };
        let path = target.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut file, &mut File::create(&path)?)?;
    }
    Ok(())
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let mut text = String::new();
    archive.by_name(name).with_context(|| format!("{name} is missing"))?.read_to_string(&mut text)?;
    Ok(text)
}

/// `key=value` lines of an `instance.cfg`.
fn parse_cfg(text: &str) -> HashMap<&str, &str> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim().trim_matches('"'))).collect()
}

#[derive(Deserialize)]
struct MmcPack {
    components: Vec<MmcComponent>,
}

#[derive(Deserialize)]
struct MmcComponent {
    uid: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "cachedVersion", default)]
    cached_version: Option<String>,
}

impl MmcComponent {
    fn version(&self) -> Option<String> {
        self.version.clone().or_else(|| self.cached_version.clone())
    }
}

const LOADER_UIDS: [(&str, Loader); 4] = [
    ("net.fabricmc.fabric-loader", Loader::Fabric),
    ("org.quiltmc.quilt-loader", Loader::Quilt),
    ("net.neoforged", Loader::NeoForge),
    ("net.minecraftforge", Loader::Forge),
];

/// Builds an instance from another launcher's `instance.cfg` and `mmc-pack.json`.
fn instance_from_launcher(cfg: &str, mmc_pack: &str) -> Result<Instance> {
    let cfg = parse_cfg(cfg);
    let pack: MmcPack = serde_json::from_str(mmc_pack).context("reading mmc-pack.json")?;
    let component = |uid: &str| pack.components.iter().find(|c| c.uid == uid);
    let mc = component("net.minecraft").and_then(MmcComponent::version).context("the instance doesn't name a Minecraft version")?;
    let on = |key: &str| cfg.get(key) == Some(&"true");
    let (loader, loader_version) =
        LOADER_UIDS.iter().find_map(|(uid, l)| Some((*l, component(uid)?.version()))).unwrap_or((Loader::Vanilla, None));
    let text = |key: &str| cfg.get(key).filter(|v| !v.is_empty()).map(|v| v.to_string());
    Ok(Instance {
        name: text("name").unwrap_or_else(|| "Imported instance".to_string()),
        mc_version: mc,
        loader,
        loader_version,
        memory_mb: if on("OverrideMemory") { cfg.get("MaxMemAlloc").and_then(|v| v.parse().ok()) } else { None },
        jvm_args: if on("OverrideJavaArgs") { text("JvmArgs") } else { None },
        play_seconds: cfg.get("totalTimePlayed").and_then(|v| v.parse().ok()).unwrap_or(0),
        pack: (cfg.get("ManagedPackType") == Some(&"modrinth")).then(|| PackSource {
            title: text("ManagedPackName").unwrap_or_default(),
            version: text("ManagedPackVersionName").unwrap_or_default(),
            project_id: text("ManagedPackID"),
            version_id: text("ManagedPackVersionID"),
            files: Vec::new(),
        }),
        ..Default::default()
    })
}

/// Treats what's in the content folders as the pack's, so an imported pack can update. Prism keeps
/// no list that FeatherLaunch can read.
fn adopt_pack_files(inst: &mut Instance) -> Result<()> {
    let game_dir = inst.game_dir();
    let Some(pack) = &mut inst.pack else { return Ok(()) };
    for kind in [ContentKind::Mod, ContentKind::ResourcePack, ContentKind::ShaderPack] {
        for entry in std::fs::read_dir(game_dir.join(kind.folder())).into_iter().flatten().flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                pack.files.push(format!("{}/{}", kind.folder(), name.trim_end_matches(".disabled")));
            }
        }
    }
    inst.save()
}

fn import_launcher_zip(path: &Path, root: &str, group: String) -> Result<Instance> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let cfg = read_entry(&mut archive, &format!("{root}instance.cfg"))?;
    let mmc_pack = read_entry(&mut archive, &format!("{root}mmc-pack.json"))?;
    let inst = Instance { group, ..instance_from_launcher(&cfg, &mmc_pack)? }.create()?;
    let game_dir = inst.game_dir();
    let mut partial = PartialInstance::new(inst);
    for folder in ["minecraft/", ".minecraft/"] {
        extract_under(&mut archive, &format!("{root}{folder}"), &game_dir)?;
    }
    adopt_pack_files(partial.get())?;
    Ok(partial.keep())
}

/// The instance's own files in an export; the rest is `.minecraft`.
fn import_own_zip(path: &Path, group: String) -> Result<Instance> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let saved: Instance = serde_json::from_str(&read_entry(&mut archive, "instance.json")?).context("reading instance.json")?;
    // A path from the PC it was exported on.
    let inst = Instance { group, java_path: None, ..saved }.create()?;
    let dir = inst.dir();
    let partial = PartialInstance::new(inst);
    extract_under(&mut archive, ".minecraft/", &dir.join(".minecraft"))?;
    for name in ["content.json", crate::icons::INSTANCE_ICON] {
        if let Ok(mut file) = archive.by_name(name) {
            std::io::copy(&mut file, &mut File::create(dir.join(name))?)?;
        }
    }
    Ok(partial.keep())
}

/// Makes an instance from a pack or instance file, with anything the player needs to hear about it.
pub async fn import_file(path: PathBuf, group: String, parallel: usize, progress: &Progress) -> Result<(Instance, Option<String>)> {
    progress.set_label("Reading file");
    let file = path.clone();
    let kind = tokio::task::spawn_blocking(move || detect(&file)).await??;
    match kind {
        ArchiveKind::Modrinth => return Ok((mrpack::import(path, group, parallel, progress).await?, None)),
        ArchiveKind::CurseForge => return curseforge::import_pack(&path, group, parallel, progress).await,
        _ => {}
    }
    progress.set_label("Copying files");
    // Extraction runs on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    let inst = tokio::task::spawn_blocking(move || match kind {
        ArchiveKind::Launcher { root } => import_launcher_zip(&path, &root, group),
        _ => import_own_zip(&path, group),
    })
    .await??;
    Ok((inst, None))
}

/// The instance folders at `dir`: it may be one, a launcher's `instances` folder, or its data folder.
fn find_instances(dir: &Path) -> Vec<PathBuf> {
    if dir.join("instance.cfg").is_file() {
        return vec![dir.to_path_buf()];
    }
    let children = |d: &Path| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).collect::<Vec<_>>();
    let found: Vec<PathBuf> = children(dir).into_iter().filter(|p| p.join("instance.cfg").is_file()).collect();
    if found.is_empty() {
        children(&dir.join("instances")).into_iter().filter(|p| p.join("instance.cfg").is_file()).collect()
    } else {
        found
    }
}

#[derive(Deserialize)]
struct GroupFile {
    groups: HashMap<String, GroupEntry>,
}

#[derive(Deserialize)]
struct GroupEntry {
    #[serde(default)]
    instances: Vec<String>,
}

/// Folder name → group, from the launcher's `instgroups.json`.
fn parse_groups(json: &str) -> HashMap<String, String> {
    let Ok(file) = serde_json::from_str::<GroupFile>(json) else { return HashMap::new() };
    file.groups.into_iter().flat_map(|(group, entry)| entry.instances.into_iter().map(move |i| (i, group.clone()))).collect()
}

fn import_folder_instance(dir: &Path, group: String) -> Result<Instance> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).with_context(|| format!("reading {}", dir.join(name).display()));
    let inst = Instance { group, ..instance_from_launcher(&read("instance.cfg")?, &read("mmc-pack.json")?)? }.create()?;
    let game_dir = inst.game_dir();
    let mut partial = PartialInstance::new(inst);
    if let Some(source) = ["minecraft", ".minecraft"].iter().map(|f| dir.join(f)).find(|p| p.is_dir()) {
        paths::copy_dir(&source, &game_dir)?;
    }
    adopt_pack_files(partial.get())?;
    Ok(partial.keep())
}

/// Copies every instance found at `dir` and returns how many. One that fails doesn't stop the rest.
pub fn import_folder(dir: &Path) -> Result<usize> {
    let instances = find_instances(dir);
    if instances.is_empty() {
        bail!("no Prism Launcher or MultiMC instances were found in {}", dir.display());
    }
    let groups = instances
        .first()
        .and_then(|i| i.parent())
        .and_then(|p| std::fs::read_to_string(p.join("instgroups.json")).ok())
        .map(|json| parse_groups(&json))
        .unwrap_or_default();
    let mut imported = 0;
    let mut failure = None;
    for dir in &instances {
        let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match import_folder_instance(dir, groups.get(&folder).cloned().unwrap_or_default()) {
            Ok(_) => imported += 1,
            Err(e) => {
                log::error(&format!("Importing {} failed: {e:#}", dir.display()));
                failure = Some(e);
            }
        }
    }
    match failure {
        Some(e) if imported == 0 => Err(e),
        _ => Ok(imported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const CFG: &str = "[General]\nname=My Pack\nOverrideMemory=true\nMaxMemAlloc=4096\nOverrideJavaArgs=false\nJvmArgs=-Xss1M\n\
                       totalTimePlayed=120\nManagedPack=true\nManagedPackType=modrinth\nManagedPackID=abc\nManagedPackName=Pack\n\
                       ManagedPackVersionID=v1\nManagedPackVersionName=1.0\n";
    const MMC_PACK: &str = r#"{"components":[
        {"uid":"org.lwjgl3","version":"3.3.3"},
        {"uid":"net.minecraft","version":"1.20.1"},
        {"uid":"net.fabricmc.intermediary","cachedVersion":"1.20.1"},
        {"uid":"net.fabricmc.fabric-loader","version":"0.16.9"}],"formatVersion":1}"#;

    fn zip_with(path: &Path, names: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, text) in names {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_an_instance_from_another_launcher() {
        let inst = instance_from_launcher(CFG, MMC_PACK).unwrap();
        assert_eq!((inst.name.as_str(), inst.mc_version.as_str()), ("My Pack", "1.20.1"));
        assert_eq!((inst.loader, inst.loader_version.as_deref()), (Loader::Fabric, Some("0.16.9")));
        assert_eq!((inst.memory_mb, inst.jvm_args.as_deref(), inst.play_seconds), (Some(4096), None, 120));
        let pack = inst.pack.unwrap();
        assert_eq!((pack.project_id.as_deref(), pack.version_id.as_deref()), (Some("abc"), Some("v1")));
    }

    #[test]
    fn a_plain_instance_stays_plain() {
        let inst = instance_from_launcher("name=Vanilla\n", r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#).unwrap();
        assert_eq!((inst.loader, inst.memory_mb), (Loader::Vanilla, None));
        assert!(inst.pack.is_none());
        assert!(instance_from_launcher("name=Broken\n", r#"{"components":[]}"#).is_err());
    }

    #[test]
    fn tells_file_formats_apart() {
        let dir = temp_dir();
        let path = dir.join("a.zip");
        let kind = |names: &[(&str, &str)]| {
            zip_with(&path, names);
            detect(&path)
        };
        assert_eq!(kind(&[("modrinth.index.json", "")]).unwrap(), ArchiveKind::Modrinth);
        assert_eq!(kind(&[("manifest.json", "")]).unwrap(), ArchiveKind::CurseForge);
        assert_eq!(kind(&[("instance.json", "")]).unwrap(), ArchiveKind::FeatherLaunch);
        assert_eq!(kind(&[("instance.cfg", "")]).unwrap(), ArchiveKind::Launcher { root: String::new() });
        assert_eq!(
            kind(&[("Mine/minecraft/saves/x/instance.cfg", ""), ("Mine/instance.cfg", "")]).unwrap(),
            ArchiveKind::Launcher { root: "Mine/".into() }
        );
        assert!(kind(&[("readme.txt", "")]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_mods_in_formats_without_a_review() {
        let dir = temp_dir();
        let path = dir.join("a.zip");
        zip_with(&path, &[("instance.cfg", ""), ("minecraft/mods/a.jar", ""), ("minecraft/options.txt", "")]);
        assert_eq!(unchecked_files(&path).unwrap(), ["minecraft/mods/a.jar"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracts_only_what_is_under_the_prefix() {
        let dir = temp_dir();
        let path = dir.join("a.zip");
        zip_with(&path, &[("Mine/minecraft/mods/a.jar", "x"), ("Mine/instance.cfg", ""), ("Mine/minecraft/../../evil", "x")]);
        let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let out = dir.join("out");
        extract_under(&mut archive, "Mine/minecraft/", &out).unwrap();
        assert!(out.join("mods/a.jar").exists());
        assert!(!out.join("instance.cfg").exists());
        assert!(!dir.join("evil").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_instances_from_any_of_the_launcher_folders() {
        let dir = temp_dir();
        let one = dir.join("instances/One");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(dir.join("instances/not-an-instance")).unwrap();
        std::fs::write(one.join("instance.cfg"), "").unwrap();
        assert_eq!(find_instances(&dir), vec![one.clone()]);
        assert_eq!(find_instances(&dir.join("instances")), vec![one.clone()]);
        assert_eq!(find_instances(&one), vec![one.clone()]);
        assert!(find_instances(&dir.join("instances/not-an-instance")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_instance_groups() {
        let groups = parse_groups(r#"{"formatVersion":"1","groups":{"Modded":{"hidden":false,"instances":["One","Two"]}}}"#);
        assert_eq!(groups.get("Two").map(String::as_str), Some("Modded"));
        assert!(parse_groups("not json").is_empty());
    }
}
