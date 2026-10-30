// SPDX-License-Identifier: GPL-3.0-only
//! Instances: one folder each under `instances/`, with `instance.json` and a `.minecraft` game dir.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Loader {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub const ALL: [Loader; 5] = [Loader::Vanilla, Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge];

    pub fn name(self) -> &'static str {
        match self {
            Loader::Vanilla => "Vanilla",
            Loader::Fabric => "Fabric",
            Loader::Quilt => "Quilt",
            Loader::Forge => "Forge",
            Loader::NeoForge => "NeoForge",
        }
    }

    /// Modrinth loader names whose mods run on this loader (Quilt also runs Fabric mods).
    pub fn modrinth_names(self) -> &'static [&'static str] {
        match self {
            Loader::Vanilla => &[],
            Loader::Fabric => &["fabric"],
            Loader::Quilt => &["quilt", "fabric"],
            Loader::Forge => &["forge"],
            Loader::NeoForge => &["neoforge"],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Instance {
    /// Folder name; stable for the instance's lifetime.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub group: String,
    pub mc_version: String,
    pub loader: Loader,
    /// `None` uses the newest loader version available at install time.
    pub loader_version: Option<String>,
    pub memory_mb: Option<u32>,
    pub java_path: Option<String>,
    pub jvm_args: Option<String>,
    pub last_played: u64,
    pub play_seconds: u64,
    /// Modification time of `icon.png`, so a changed icon gets a new cache key.
    #[serde(skip)]
    pub icon_stamp: Option<u128>,
}

impl Default for Instance {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "New instance".into(),
            group: String::new(),
            mc_version: String::new(),
            loader: Loader::Vanilla,
            loader_version: None,
            memory_mb: None,
            java_path: None,
            jvm_args: None,
            last_played: 0,
            play_seconds: 0,
            icon_stamp: None,
        }
    }
}

impl Instance {
    pub fn dir(&self) -> PathBuf {
        paths::instances().join(&self.id)
    }

    pub fn game_dir(&self) -> PathBuf {
        self.dir().join(".minecraft")
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(self.game_dir())?;
        paths::write_json(&self.dir().join("instance.json"), self)
    }

    pub fn record_launch(&mut self) -> Result<()> {
        self.last_played = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.save()
    }

    pub fn record_play(&mut self, seconds: u64) -> Result<()> {
        self.play_seconds += seconds;
        self.save()
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = paths::instances().join(id).join("instance.json");
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut inst: Instance = serde_json::from_slice(&bytes)?;
        inst.id = id.to_string();
        inst.icon_stamp = std::fs::metadata(inst.icon_path())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos());
        Ok(inst)
    }

    pub fn icon_path(&self) -> PathBuf {
        self.dir().join(crate::icons::INSTANCE_ICON)
    }

    /// Goes back to the default icon. Having no custom icon already counts as done.
    pub fn reset_icon(&self) -> Result<()> {
        match std::fs::remove_file(self.icon_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing {}", self.icon_path().display()))
            }
            _ => Ok(()),
        }
    }

    pub fn icon_key(&self) -> Option<String> {
        self.icon_stamp.map(|s| format!("file:{}|{s}", self.icon_path().display()))
    }

    pub fn create(mut self) -> Result<Self> {
        self.id = reserve_id(&self.name)?;
        if let Err(e) = self.save() {
            let _ = std::fs::remove_dir_all(self.dir());
            return Err(e);
        }
        Ok(self)
    }

    pub fn duplicate(&self) -> Result<Self> {
        let mut copy = self.clone();
        copy.name = format!("{} (copy)", self.name);
        copy.id = reserve_id(&copy.name)?;
        copy.last_played = 0;
        copy.play_seconds = 0;
        // A failed copy (a full disk, say) would otherwise leave a hidden half-copied folder.
        let copied = paths::copy_dir(&self.dir(), &copy.dir()).map_err(anyhow::Error::from).and_then(|()| copy.save());
        if let Err(e) = copied {
            let _ = std::fs::remove_dir_all(copy.dir());
            return Err(e);
        }
        Ok(copy)
    }

    pub fn delete(&self) -> Result<()> {
        std::fs::remove_dir_all(self.dir())?;
        Ok(())
    }

    pub fn latest_crash_report(&self) -> Option<PathBuf> {
        newest_file(&self.game_dir().join("crash-reports"))
    }

    pub fn worlds(&self) -> Vec<World> {
        list_worlds(&self.game_dir().join("saves"))
    }

    /// Takes the settings the instance page edits from `edited`. Destructured without `..`, so a
    /// new field doesn't compile until it is placed here on purpose: play time and last played
    /// must stay, because they may have changed since the page opened.
    pub fn apply_settings(&mut self, edited: &Instance) {
        let Instance {
            id: _,
            name,
            group,
            mc_version,
            loader,
            loader_version,
            memory_mb,
            java_path,
            jvm_args,
            last_played: _,
            play_seconds: _,
            icon_stamp: _,
        } = edited.clone();
        self.name = name;
        self.group = group;
        self.mc_version = mc_version;
        self.loader = loader;
        self.loader_version = loader_version;
        self.memory_mb = memory_mb;
        self.java_path = java_path;
        self.jvm_args = jvm_args;
    }

    pub fn summary(&self) -> String {
        match (self.loader, &self.loader_version) {
            (Loader::Vanilla, _) => self.mc_version.clone(),
            (l, Some(v)) => format!("{} · {} {v}", self.mc_version, l.name()),
            (l, None) => format!("{} · {}", self.mc_version, l.name()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct World {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

fn list_worlds(saves: &Path) -> Vec<World> {
    let Ok(entries) = std::fs::read_dir(saves) else { return Vec::new() };
    let worlds = entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| {
        let modified = e.metadata().and_then(|m| m.modified()).ok();
        Some(World { name: e.file_name().into_string().ok()?, path: e.path(), modified })
    });
    newest_first(worlds.collect())
}

fn newest_first(mut worlds: Vec<World>) -> Vec<World> {
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}

fn newest_file(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries.flatten().max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok()).map(|e| e.path())
}

pub fn list() -> Vec<Instance> {
    let Ok(entries) = std::fs::read_dir(paths::instances()) else { return Vec::new() };
    let mut out: Vec<Instance> =
        entries.flatten().filter_map(|e| e.file_name().into_string().ok()).filter_map(|id| Instance::load(&id).ok()).collect();
    out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.name.cmp(&b.name)));
    out
}

pub fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    let s = s.trim_matches(|c| c == '_' || c == '.').to_string();
    if s.is_empty() { "instance".into() } else { s }
}

/// Picks a free folder name and creates the folder in the same step, so two instances created at
/// once (say, two imports of the same pack) can't both claim it.
fn reserve_id(name: &str) -> Result<String> {
    let base = slug(name);
    let mut id = base.clone();
    let mut n = 1;
    loop {
        match std::fs::create_dir(paths::instances().join(&id)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                n += 1;
                id = format!("{base}-{n}");
            }
            Err(e) => return Err(e).with_context(|| format!("creating the instance folder {id}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(slug("My Pack 1.20!"), "My_Pack_1.20");
        assert_eq!(slug("..\\evil"), "evil");
        assert_eq!(slug("???"), "instance");
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, seconds_ago: u64) {
        let time = SystemTime::now() - std::time::Duration::from_secs(seconds_ago);
        std::fs::File::options().write(true).create(true).truncate(false).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn newest_file_finds_the_latest_crash_report() {
        let dir = temp_dir();
        assert_eq!(newest_file(&dir.join("missing")), None);
        assert_eq!(newest_file(&dir), None);
        touch(&dir.join("old.txt"), 500);
        touch(&dir.join("latest.txt"), 5);
        touch(&dir.join("older.txt"), 900);
        assert_eq!(newest_file(&dir), Some(dir.join("latest.txt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn worlds_are_the_folders_in_saves() {
        let dir = temp_dir();
        assert!(list_worlds(&dir.join("missing")).is_empty());
        std::fs::create_dir(dir.join("My world")).unwrap();
        touch(&dir.join("not-a-world.txt"), 1);
        let worlds = list_worlds(&dir);
        assert_eq!(worlds.len(), 1);
        assert_eq!((worlds[0].name.as_str(), worlds[0].path.clone()), ("My world", dir.join("My world")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn worlds_sort_newest_first() {
        let world = |name: &str, age: u64| World {
            name: name.into(),
            path: PathBuf::new(),
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 - age)),
        };
        let sorted = newest_first(vec![world("Old", 500), world("New", 5), world("Older", 900)]);
        assert_eq!(sorted.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["New", "Old", "Older"]);
    }

    #[test]
    fn saving_settings_keeps_play_time() {
        let mut saved = Instance { id: "a".into(), play_seconds: 3600, last_played: 99, icon_stamp: Some(7), ..Default::default() };
        let edited = Instance {
            id: "other".into(),
            name: "Renamed".into(),
            group: "Group".into(),
            mc_version: "1.21".into(),
            loader: Loader::Fabric,
            loader_version: Some("0.16".into()),
            memory_mb: Some(4096),
            java_path: Some("java".into()),
            jvm_args: Some("-Xss1M".into()),
            last_played: 1,
            play_seconds: 1,
            icon_stamp: None,
        };
        saved.apply_settings(&edited);
        assert_eq!((saved.name.as_str(), saved.group.as_str(), saved.mc_version.as_str()), ("Renamed", "Group", "1.21"));
        assert_eq!(saved.loader, Loader::Fabric);
        assert_eq!(saved.loader_version.as_deref(), Some("0.16"));
        assert_eq!((saved.memory_mb, saved.java_path.as_deref(), saved.jvm_args.as_deref()), (Some(4096), Some("java"), Some("-Xss1M")));
        assert_eq!((saved.id.as_str(), saved.play_seconds, saved.last_played, saved.icon_stamp), ("a", 3600, 99, Some(7)));
    }
}
