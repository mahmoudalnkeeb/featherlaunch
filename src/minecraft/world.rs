// SPDX-License-Identifier: GPL-3.0-only
//! World folders under `saves/`: reading and changing their name, copying and importing them.

use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::minecraft::instance::World;
use crate::modplatform::import::extract_under;
use crate::modplatform::mrpack::safe_rel_path;
use crate::paths;

/// The next byte offset after a named binary tag payload, or `None` if the data is cut short.
fn skip(b: &[u8], p: usize, tag: u8) -> Option<usize> {
    let int = |at: usize| Some(i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?).max(0) as usize);
    match tag {
        1 => Some(p + 1),
        2 => Some(p + 2),
        3 | 5 => Some(p + 4),
        4 | 6 => Some(p + 8),
        7 => Some(p + 4 + int(p)?),
        8 => Some(p + 2 + u16::from_be_bytes(b.get(p..p + 2)?.try_into().ok()?) as usize),
        9 => {
            let item = *b.get(p)?;
            let mut at = p + 5;
            for _ in 0..int(p + 1)? {
                at = skip(b, at, item)?;
            }
            Some(at)
        }
        10 => {
            let mut at = p;
            loop {
                let (next, entry) = entry(b, at)?;
                let Some((tag, _)) = entry else { return Some(next) };
                at = skip(b, next, tag)?;
            }
        }
        11 => Some(p + 4 + 4 * int(p)?),
        12 => Some(p + 4 + 8 * int(p)?),
        _ => None,
    }
}

/// A tag's type and name.
type Tag<'a> = (u8, &'a [u8]);

/// Reads a tag's header at `p`: where its payload starts, and the tag. An end tag has neither.
fn entry(b: &[u8], p: usize) -> Option<(usize, Option<Tag<'_>>)> {
    let tag = *b.get(p)?;
    if tag == 0 {
        return Some((p + 1, None));
    }
    let len = u16::from_be_bytes(b.get(p + 1..p + 3)?.try_into().ok()?) as usize;
    Some((p + 3 + len, Some((tag, b.get(p + 3..p + 3 + len)?))))
}

/// A compound's child by name, as its payload's offset and type.
fn child(b: &[u8], compound: usize, name: &[u8]) -> Option<(usize, u8)> {
    let mut at = compound;
    loop {
        let (payload, entry) = entry(b, at)?;
        let (tag, found) = entry?;
        if found == name {
            return Some((payload, tag));
        }
        at = skip(b, payload, tag)?;
    }
}

/// Where `Data.LevelName` is in an uncompressed level.dat, including its two-byte length.
fn level_name_range(b: &[u8]) -> Option<Range<usize>> {
    let (root, Some((10, _))) = entry(b, 0)? else { return None };
    let (data, 10) = child(b, root, b"Data")? else { return None };
    let (name, 8) = child(b, data, b"LevelName")? else { return None };
    Some(name..skip(b, name, 8)?)
}

fn read_level_dat(world: &Path) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(std::fs::File::open(world.join("level.dat"))?).read_to_end(&mut out)?;
    Ok(out)
}

pub fn level_name(world: &Path) -> Option<String> {
    let data = read_level_dat(world).ok()?;
    let range = level_name_range(&data)?;
    Some(String::from_utf8_lossy(&data[range.start + 2..range.end]).into_owned())
}

/// Changes the name shown in the game. The folder keeps its name, so nothing pointing at it breaks.
pub fn set_level_name(world: &Path, name: &str) -> Result<()> {
    let data = read_level_dat(world).context("reading level.dat")?;
    let range = level_name_range(&data).context("level.dat has no world name")?;
    let len = u16::try_from(name.len()).context("that name is too long")?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&data[..range.start])?;
    encoder.write_all(&len.to_be_bytes())?;
    encoder.write_all(name.as_bytes())?;
    encoder.write_all(&data[range.end..])?;
    // Written beside it and renamed, so a failed write can't lose the world's data.
    let tmp = world.join("level.dat.tmp");
    std::fs::write(&tmp, encoder.finish()?)?;
    std::fs::rename(&tmp, world.join("level.dat"))?;
    Ok(())
}

/// A folder name in `saves` that isn't taken, based on `name`.
fn free_folder(saves: &Path, name: &str) -> PathBuf {
    let mut target = saves.join(name);
    let mut n = 2;
    while target.exists() {
        target = saves.join(format!("{name} ({n})"));
        n += 1;
    }
    target
}

pub fn duplicate(world: &World) -> Result<()> {
    let saves = world.path.parent().context("world has no parent folder")?;
    let folder = world.path.file_name().context("world has no folder name")?.to_string_lossy();
    let target = free_folder(saves, &format!("{folder} (copy)"));
    let copied = paths::copy_dir(&world.path, &target).map_err(anyhow::Error::from);
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&target);
        return Err(e);
    }
    // The copy works without a new name, it's just harder to tell apart.
    let _ = set_level_name(&target, &format!("{} (copy)", world.name));
    Ok(())
}

/// Extracts a world from a zip into `saves`. The world can sit at the zip's root or in a folder.
pub fn import_zip(saves: &Path, zip: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip)?).context("that file isn't a zip")?;
    let level = archive
        .file_names()
        .filter(|n| n.rsplit('/').next() == Some("level.dat"))
        .min_by_key(|n| n.matches('/').count())
        .context("that zip doesn't contain a Minecraft world (no level.dat)")?
        .to_string();
    let prefix = level.trim_end_matches("level.dat").to_string();
    let stem = zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "World".to_string());
    let name = prefix.trim_end_matches('/').rsplit('/').next().filter(|n| safe_rel_path(n).is_some()).unwrap_or(&stem);
    std::fs::create_dir_all(saves)?;
    let target = free_folder(saves, name);
    let extracted = extract_under(&mut archive, &prefix, &target);
    if extracted.is_err() {
        let _ = std::fs::remove_dir_all(&target);
    }
    extracted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(tag: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend((name.len() as u16).to_be_bytes());
        out.extend(name.as_bytes());
        out.extend(payload);
        out
    }

    fn string(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u16).to_be_bytes().to_vec();
        out.extend(text.as_bytes());
        out
    }

    /// A level.dat with every kind of tag before the name, so `skip` has to walk past them all.
    fn level_dat() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(named(1, "byte", &[1]));
        data.extend(named(2, "short", &[0, 1]));
        data.extend(named(3, "int", &[0; 4]));
        data.extend(named(4, "long", &[0; 8]));
        data.extend(named(5, "float", &[0; 4]));
        data.extend(named(6, "double", &[0; 8]));
        data.extend(named(7, "bytes", &[0, 0, 0, 2, 9, 9]));
        data.extend(named(9, "list", &[8, 0, 0, 0, 1, 0, 1, b'x']));
        data.extend(named(10, "nested", &[1, 0, 1, b'a', 5, 0]));
        data.extend(named(11, "ints", &[0, 0, 0, 1, 0, 0, 0, 0]));
        data.extend(named(12, "longs", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]));
        data.extend(named(8, "LevelName", &string("Old name")));
        data.extend(named(3, "after", &[0, 0, 0, 7]));
        data.push(0);
        let mut root = named(10, "Data", &data);
        root.push(0);
        named(10, "", &root)
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_level(world: &Path) {
        std::fs::create_dir_all(world).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&level_dat()).unwrap();
        std::fs::write(world.join("level.dat"), encoder.finish().unwrap()).unwrap();
    }

    #[test]
    fn reads_and_changes_the_world_name() {
        let dir = temp_dir();
        write_level(&dir);
        assert_eq!(level_name(&dir).as_deref(), Some("Old name"));
        set_level_name(&dir, "A much longer new name").unwrap();
        assert_eq!(level_name(&dir).as_deref(), Some("A much longer new name"));
        // What follows the name survives the change of length.
        let data = read_level_dat(&dir).unwrap();
        let (root, _) = entry(&data, 0).unwrap();
        let (compound, _) = child(&data, root, b"Data").unwrap();
        let (after, _) = child(&data, compound, b"after").unwrap();
        assert_eq!(&data[after..after + 4], [0, 0, 0, 7]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_level_dat_has_no_name() {
        assert_eq!(level_name_range(&level_dat()[..20]), None);
        assert_eq!(level_name_range(&[]), None);
        assert_eq!(level_name_range(&named(10, "", &[0])), None);
    }

    #[test]
    fn copies_a_world_under_a_new_name() {
        let saves = temp_dir();
        let path = saves.join("New World");
        write_level(&path);
        let world = World { name: "Old name".into(), path, modified: None };
        duplicate(&world).unwrap();
        duplicate(&world).unwrap();
        assert_eq!(level_name(&saves.join("New World (copy)")).as_deref(), Some("Old name (copy)"));
        assert!(saves.join("New World (copy) (2)").join("level.dat").exists());
        std::fs::remove_dir_all(&saves).unwrap();
    }

    fn zip_with(path: &Path, names: &[&str]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for name in names {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"data").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn imports_a_world_from_either_zip_layout() {
        let dir = temp_dir();
        let saves = dir.join("saves");
        let zip = dir.join("Backup.zip");

        zip_with(&zip, &["My World/level.dat", "My World/region/r.0.0.mca", "../evil.txt"]);
        import_zip(&saves, &zip).unwrap();
        assert!(saves.join("My World/region/r.0.0.mca").exists());
        assert!(!dir.join("evil.txt").exists());

        zip_with(&zip, &["level.dat", "region/r.0.0.mca"]);
        import_zip(&saves, &zip).unwrap();
        assert!(saves.join("Backup/level.dat").exists());

        zip_with(&zip, &["readme.txt"]);
        assert!(import_zip(&saves, &zip).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
