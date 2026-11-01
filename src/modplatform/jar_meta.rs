// SPDX-License-Identifier: GPL-3.0-only
//! Names and versions read from inside mod jars, for mods Modrinth doesn't know.

use std::io::Read;
use std::path::Path;

use serde_json::Value;

/// Far beyond any real metadata file. A jar can hold one that unpacks to gigabytes.
const MAX_METADATA: u64 = 4 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct JarInfo {
    pub title: String,
    pub version: Option<String>,
}

pub fn read(path: &Path) -> Option<JarInfo> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut text = |name: &str| {
        let mut out = String::new();
        archive.by_name(name).ok()?.take(MAX_METADATA).read_to_string(&mut out).ok()?;
        Some(out)
    };
    if let Some(json) = text("fabric.mod.json") {
        return fabric(&json);
    }
    if let Some(json) = text("quilt.mod.json") {
        return quilt(&json);
    }
    if let Some(toml) = text("META-INF/neoforge.mods.toml").or_else(|| text("META-INF/mods.toml")) {
        return forge(&toml, text("META-INF/MANIFEST.MF").as_deref());
    }
    legacy_forge(&text("mcmod.info")?)
}

/// JSON files in mods are often hand-written, so a bad one just means no name.
fn fabric(json: &str) -> Option<JarInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let title = v["name"].as_str().or(v["id"].as_str())?.to_string();
    Some(JarInfo { title, version: v["version"].as_str().map(String::from) })
}

fn quilt(json: &str) -> Option<JarInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let loader = &v["quilt_loader"];
    let title = loader["metadata"]["name"].as_str().or(loader["id"].as_str())?.to_string();
    Some(JarInfo { title, version: loader["version"].as_str().map(String::from) })
}

/// The first `[[mods]]` table of a mods.toml. A jar can hold several mods, and the first is the
/// main one. A few key lookups don't justify a TOML parser in the binary.
fn forge(toml: &str, manifest: Option<&str>) -> Option<JarInfo> {
    let mut section = toml.lines().map(str::trim).skip_while(|l| *l != "[[mods]]").skip(1).take_while(|l| !l.starts_with('['));
    let (mut title, mut version) = (None, None);
    for line in section.by_ref() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_start_matches(['"', '\'']);
        let value = value.split(['"', '\'']).next().unwrap_or(value).to_string();
        match key.trim() {
            "displayName" => title = Some(value),
            "version" => version = Some(value),
            _ => {}
        }
    }
    // Forge fills this in from the jar's manifest when it builds the mod list.
    if version.as_deref() == Some("${file.jarVersion}") {
        version = manifest.and_then(|m| m.lines().find_map(|l| l.strip_prefix("Implementation-Version:"))).map(|v| v.trim().to_string());
    }
    Some(JarInfo { title: title?, version })
}

fn legacy_forge(json: &str) -> Option<JarInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let first = v.get(0).or(v["modList"].get(0))?;
    let version = first["version"].as_str().filter(|v| !v.contains("${")).map(String::from);
    Some(JarInfo { title: first["name"].as_str()?.to_string(), version })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str, version: Option<&str>) -> Option<JarInfo> {
        Some(JarInfo { title: title.into(), version: version.map(String::from) })
    }

    #[test]
    fn reads_fabric_and_quilt() {
        assert_eq!(fabric(r#"{"id":"sodium","name":"Sodium","version":"0.6"}"#), info("Sodium", Some("0.6")));
        assert_eq!(fabric(r#"{"id":"sodium"}"#), info("sodium", None));
        assert_eq!(quilt(r#"{"quilt_loader":{"id":"a","version":"1","metadata":{"name":"A Mod"}}}"#), info("A Mod", Some("1")));
        assert_eq!(fabric("not json"), None);
    }

    #[test]
    fn reads_the_first_forge_mod() {
        let toml = "modLoader=\"javafml\"\nversion=\"wrong\"\n[[mods]]\nmodId=\"a\"\nversion='1.2' # comment\ndisplayName = \"A Mod\"\n\
                    [[dependencies.a]]\nmodId=\"forge\"\n[[mods]]\ndisplayName=\"Second\"\n";
        assert_eq!(forge(toml, None), info("A Mod", Some("1.2")));
    }

    #[test]
    fn takes_a_forge_version_from_the_manifest() {
        let toml = "[[mods]]\nversion=\"${file.jarVersion}\"\ndisplayName=\"A\"\n";
        assert_eq!(forge(toml, Some("Manifest-Version: 1.0\nImplementation-Version: 3.1\n")), info("A", Some("3.1")));
        assert_eq!(forge(toml, None), info("A", None));
    }

    #[test]
    fn reads_both_mcmod_info_shapes() {
        assert_eq!(legacy_forge(r#"[{"name":"Old","version":"1"}]"#), info("Old", Some("1")));
        assert_eq!(legacy_forge(r#"{"modList":[{"name":"Old","version":"${version}"}]}"#), info("Old", None));
    }

    #[test]
    fn reads_a_real_jar() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("featherlaunch-test-{}.jar", uuid::Uuid::new_v4().simple()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(br#"{"name":"Zipped","version":"2"}"#).unwrap();
        zip.finish().unwrap();
        let found = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(found, info("Zipped", Some("2")));
    }
}
