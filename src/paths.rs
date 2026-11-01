// SPDX-License-Identifier: GPL-3.0-only
//! On-disk layout. Libraries, assets and versions are shared by every instance so nothing
//! is downloaded twice. A `portable.txt` next to the executable keeps all data beside it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use serde::Serialize;
use serde::de::DeserializeOwned;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Must run before anything reads [`root`].
pub fn set_root(dir: PathBuf) {
    let _ = ROOT.set(dir);
}

pub fn root() -> &'static PathBuf {
    ROOT.get_or_init(|| {
        if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from))
            && dir.join("portable.txt").exists()
        {
            return dir.join("data");
        }
        directories::BaseDirs::new().map(|b| b.config_dir().join("featherlaunch")).unwrap_or_else(|| PathBuf::from("featherlaunch-data"))
    })
}

/// Where the player moved their instances to, if they did.
static INSTANCES: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_instances(dir: Option<PathBuf>) {
    *INSTANCES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = dir;
}

pub fn instances() -> PathBuf {
    let moved = INSTANCES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    moved.unwrap_or_else(|| root().join("instances"))
}
pub fn libraries() -> PathBuf {
    root().join("libraries")
}
pub fn assets() -> PathBuf {
    root().join("assets")
}
pub fn versions() -> PathBuf {
    root().join("versions")
}
pub fn natives() -> PathBuf {
    root().join("natives")
}
pub fn java() -> PathBuf {
    root().join("java")
}
pub fn cache() -> PathBuf {
    root().join("cache")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    for dir in [instances(), libraries(), assets(), versions(), natives(), java(), cache()] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Where to download a modpack to. Unique, so installing the same pack twice at once doesn't
/// share one file; [`clean_cache`] recognizes the name if the launcher closes before it's deleted.
pub fn temp_pack_path(file_name: &str) -> PathBuf {
    cache().join(format!("{}-{file_name}", uuid::Uuid::new_v4().simple()))
}

fn is_temp_pack(name: &str) -> bool {
    name.ends_with(".mrpack")
        && name.split_once('-').is_some_and(|(prefix, _)| prefix.len() == 32 && prefix.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Removes modpack downloads left in the cache when the launcher closed or crashed during an
/// install. Ones under an hour old are kept in case another copy of the launcher is using them.
pub fn clean_cache() {
    let Ok(entries) = std::fs::read_dir(cache()) else { return };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if stale && is_temp_pack(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Reads a JSON file. A missing file is `Ok(None)`; anything else that stops it loading is an
/// error naming the path.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_slice(&bytes).map(Some).with_context(|| format!("parsing {}", path.display()))
}

/// Problems found while loading saved files, waiting for the UI to show them.
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn take_problems() -> Vec<String> {
    std::mem::take(&mut PROBLEMS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// Loads a file the launcher saves over later, or the default when it doesn't exist. A file that
/// won't load is moved aside first, so that later save can't destroy what is left in it.
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    match read_json(path) {
        Ok(value) => value.unwrap_or_default(),
        Err(e) => {
            let message = match set_aside(path) {
                Ok(backup) => format!("{e:#}. It was kept as {} and defaults were loaded.", backup.display()),
                // Another thread hit the same file first and has reported it.
                Err(moved) if moved.kind() == std::io::ErrorKind::NotFound => return T::default(),
                Err(moved) => format!("{e:#}. It could not be kept as a backup ({moved}), so the next save may replace it."),
            };
            crate::log::error(&message);
            PROBLEMS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(message);
            T::default()
        }
    }
}

/// Renames a file to a free `.bak` name beside it, keeping any earlier backup.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let mut n = 0;
    loop {
        let mut backup = path.as_os_str().to_owned();
        backup.push(if n == 0 { ".bak".to_string() } else { format!(".bak.{n}") });
        let backup = PathBuf::from(backup);
        if !backup.exists() {
            return std::fs::rename(path, &backup).map(|()| backup);
        }
        n += 1;
    }
}

/// Writes JSON to a temporary file, flushes it to disk and renames it over `path`. A crash or
/// power cut mid-write then leaves the previous file, not a truncated one that loads as empty.
pub fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Moves a folder's contents into `to`, which must be empty or new. A failure leaves `from` whole.
pub fn move_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    if to.starts_with(from) {
        anyhow::bail!("that folder is inside the current one");
    }
    if to.exists() && std::fs::read_dir(to)?.next().is_some() {
        anyhow::bail!("choose an empty folder");
    }
    // On the same drive this is instant, and it only works with `to` gone.
    let _ = std::fs::remove_dir(to);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = copy_dir(from, to) {
        let _ = std::fs::remove_dir_all(to);
        return Err(e.into());
    }
    std::fs::remove_dir_all(from)?;
    Ok(())
}

pub fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_a_folder_only_into_an_empty_place() {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        let from = dir.join("from");
        std::fs::create_dir_all(from.join("One")).unwrap();
        std::fs::write(from.join("One/instance.json"), "{}").unwrap();

        assert!(move_dir(&from, &from.join("inside")).is_err());
        let full = dir.join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("x"), "").unwrap();
        assert!(move_dir(&from, &full).is_err());
        assert!(from.join("One/instance.json").exists(), "a refused move changes nothing");

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        move_dir(&from, &empty).unwrap();
        assert!(empty.join("One/instance.json").exists());
        assert!(!from.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recognizes_only_temporary_pack_downloads() {
        assert!(is_temp_pack("0123456789abcdef0123456789abcdef-Fabulously Optimized-6.2.mrpack"));
        assert!(!is_temp_pack("Fabulously-Optimized.mrpack"));
        assert!(!is_temp_pack("0123456789abcdef0123456789abcdef-x.jar"));
        assert!(!is_temp_pack("version_manifest_v2.json"));
    }

    #[test]
    fn temporary_pack_paths_are_unique_and_recognized() {
        let (a, b) = (temp_pack_path("Pack-1.0.mrpack"), temp_pack_path("Pack-1.0.mrpack"));
        assert_ne!(a, b);
        assert!(is_temp_pack(&a.file_name().unwrap().to_string_lossy()));
    }

    #[test]
    fn saved_files_load_or_are_kept_as_backups() {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let backup = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();

        // Missing: defaults, and nothing to report.
        assert_eq!(read_json::<Vec<u32>>(&path).unwrap(), None);
        assert_eq!(load_json::<Vec<u32>>(&path), Vec::<u32>::new());
        assert!(take_problems().is_empty());

        // Valid: loaded, and left alone.
        std::fs::write(&path, "[1,2]").unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path), vec![1, 2]);
        assert!(path.exists());

        // Corrupt: defaults, the file moved aside whole, and the path in the report.
        std::fs::write(&path, "[1,").unwrap();
        assert!(read_json::<Vec<u32>>(&path).unwrap_err().to_string().contains("accounts.json"));
        assert_eq!(load_json::<Vec<u32>>(&path), Vec::<u32>::new());
        assert!(!path.exists());
        assert_eq!(backup("accounts.json.bak"), "[1,");
        let problems = take_problems();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("accounts.json.bak"), "{}", problems[0]);

        // A second bad file doesn't replace the first backup.
        std::fs::write(&path, "second").unwrap();
        let _ = load_json::<Vec<u32>>(&path);
        assert_eq!(backup("accounts.json.bak"), "[1,");
        assert_eq!(backup("accounts.json.bak.1"), "second");
        take_problems();

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
