// SPDX-License-Identifier: GPL-3.0-only
//! Mod loaders. Fabric and Quilt publish ready-made profiles; Forge and NeoForge run their
//! official installer headless into the shared data folder, which uses the vanilla layout.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::mojang::{self, VersionJson};
use crate::minecraft::instance::Loader;
use crate::net::{self, Download};
use crate::paths;
use crate::tasks::Progress;

#[derive(Deserialize)]
struct FabricEntry {
    loader: FabricLoader,
}

#[derive(Deserialize)]
struct FabricLoader {
    version: String,
}

fn profile_url(loader: Loader, mc: &str, version: &str) -> Option<String> {
    match loader {
        Loader::Fabric => Some(format!("https://meta.fabricmc.net/v2/versions/loader/{mc}/{version}/profile/json")),
        Loader::Quilt => Some(format!("https://meta.quiltmc.org/v3/versions/loader/{mc}/{version}/profile/json")),
        _ => None,
    }
}

pub async fn versions(loader: Loader, mc: &str) -> Result<Vec<String>> {
    match loader {
        Loader::Vanilla => Ok(Vec::new()),
        Loader::Fabric | Loader::Quilt => {
            let url = match loader {
                Loader::Fabric => format!("https://meta.fabricmc.net/v2/versions/loader/{mc}"),
                _ => format!("https://meta.quiltmc.org/v3/versions/loader/{mc}"),
            };
            let entries: Vec<FabricEntry> = net::get_json(&url).await?;
            Ok(entries.into_iter().map(|e| e.loader.version).collect())
        }
        Loader::Forge => {
            let all: HashMap<String, Vec<String>> =
                net::get_json("https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json").await?;
            let prefix = format!("{mc}-");
            let mut list: Vec<String> =
                all.get(mc).map(|v| v.iter().filter_map(|s| s.strip_prefix(&prefix).map(String::from)).collect()).unwrap_or_default();
            list.reverse();
            Ok(list)
        }
        Loader::NeoForge => {
            #[derive(Deserialize)]
            struct List {
                versions: Vec<String>,
            }
            let list: List = net::get_json("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge").await?;
            let Some(prefix) = neoforge_prefix(mc) else { return Ok(Vec::new()) };
            let mut out: Vec<String> = list.versions.into_iter().filter(|v| v.starts_with(&prefix)).collect();
            out.reverse();
            Ok(out)
        }
    }
}

/// NeoForge versions drop the leading "1.": MC 1.21.1 is 21.1.x and MC 1.21 is 21.0.x.
fn neoforge_prefix(mc: &str) -> Option<String> {
    let rest = mc.strip_prefix("1.")?;
    let mut parts = rest.split('.');
    let major = parts.next()?;
    let minor = parts.next().unwrap_or("0");
    Some(format!("{major}.{minor}."))
}

/// Forge installers on a slow PC take a few minutes; one that runs far longer has hung.
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(20 * 60);

static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn ensure(loader: Loader, mc: &str, version: Option<&str>, java: &Path, progress: &Progress) -> Result<String> {
    if loader == Loader::Vanilla {
        return Ok(mc.to_string());
    }
    // Installers write shared version and library folders; two at once would clash. The checks
    // below then find what an earlier install created.
    let _install = INSTALL.lock().await;
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            versions(loader, mc).await?.into_iter().next().with_context(|| format!("no {} versions for Minecraft {mc}", loader.name()))?
        }
    };
    match loader {
        Loader::Fabric | Loader::Quilt => {
            let id = match loader {
                Loader::Fabric => format!("fabric-loader-{version}-{mc}"),
                _ => format!("quilt-loader-{version}-{mc}"),
            };
            if mojang::load_local(&id).is_err() {
                progress.set_label(format!("Installing {} {version}", loader.name()));
                let url = profile_url(loader, mc, &version).unwrap();
                let mut profile: VersionJson = net::get_json(&url).await?;
                profile.id = id.clone();
                let path = mojang::version_json_path(&id);
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, serde_json::to_vec_pretty(&profile)?)?;
            }
            Ok(id)
        }
        Loader::Forge | Loader::NeoForge => install_forge_like(loader, mc, &version, java, progress).await,
        Loader::Vanilla => unreachable!(),
    }
}

async fn install_forge_like(loader: Loader, mc: &str, version: &str, java: &Path, progress: &Progress) -> Result<String> {
    let (id, url) = match loader {
        Loader::Forge => (
            format!("{mc}-forge-{version}"),
            format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{version}/forge-{mc}-{version}-installer.jar"),
        ),
        _ => (
            format!("neoforge-{version}"),
            format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"),
        ),
    };
    if let Some(found) = find_installed(&id, version) {
        return Ok(found);
    }

    progress.set_label(format!("Downloading {} {version} installer", loader.name()));
    let installer = paths::cache().join(format!("{}-{mc}-{version}-installer.jar", loader.name().to_lowercase()));
    net::download_tracked(&Download::new(url, &installer), Some(progress)).await?;

    // The installers refuse to run without a launcher profile file in the target folder.
    let root = paths::root();
    let profiles = root.join("launcher_profiles.json");
    if !profiles.exists() {
        std::fs::write(&profiles, r#"{"profiles":{}}"#)?;
    }

    progress.set_label(format!("Running {} installer (this can take a few minutes)", loader.name()));
    // A cancel would drop the wait but leave the installer process running.
    let _critical = progress.critical();
    let mut ok = false;
    let mut last_output = String::new();
    for flag in ["--installClient", "--install-client"] {
        let mut cmd = tokio::process::Command::new(java);
        // Killed when the timeout drops it, instead of running on in the background.
        cmd.arg("-jar").arg(&installer).arg(flag).arg(root).current_dir(root).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let out = tokio::time::timeout(INSTALLER_TIMEOUT, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("{} installer did not finish within 20 minutes", loader.name()))?
            .context("starting the installer")?;
        if out.status.success() {
            ok = true;
            break;
        }
        // Forge prints errors on stderr, so keep the tail of both streams, in order.
        let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        last_output = lines[lines.len().saturating_sub(15)..].join("\n");
    }
    if !ok {
        bail!("{} installer failed:\n{last_output}", loader.name());
    }
    find_installed(&id, version).with_context(|| format!("installer finished but {id} was not created"))
}

/// The installer names its profile slightly differently across Forge generations.
fn find_installed(expected: &str, version: &str) -> Option<String> {
    if mojang::load_local(expected).is_ok() {
        return Some(expected.to_string());
    }
    let entries = std::fs::read_dir(paths::versions()).ok()?;
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .find(|name| is_profile_of(name, version) && mojang::load_local(name).is_ok())
}

/// Whether a profile folder is this Forge-like version. The version must stand alone in the name:
/// 47.1 must not match `1.20.1-forge-47.10` or `1.20.1-forge-47.1.10`.
fn is_profile_of(name: &str, version: &str) -> bool {
    let version_char = |c: char| c.is_ascii_digit() || c == '.';
    name.to_lowercase().contains("forge")
        && name
            .match_indices(version)
            .any(|(at, _)| !name[..at].ends_with(version_char) && !name[at + version.len()..].starts_with(version_char))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_prefixes() {
        assert_eq!(neoforge_prefix("1.21.1").unwrap(), "21.1.");
        assert_eq!(neoforge_prefix("1.21").unwrap(), "21.0.");
        assert_eq!(neoforge_prefix("1.20.4").unwrap(), "20.4.");
        assert!(neoforge_prefix("24w10a").is_none());
    }

    #[test]
    fn forge_profiles_match_the_exact_version() {
        assert!(is_profile_of("1.20.1-forge-47.1.10", "47.1.10"));
        assert!(is_profile_of("neoforge-21.1.77", "21.1.77"));
        // Older Forge names run the version into a word and add a suffix.
        assert!(is_profile_of("1.7.10-Forge10.13.4.1614-1.7.10", "10.13.4.1614"));

        assert!(!is_profile_of("1.20.1-forge-47.10", "47.1"));
        assert!(!is_profile_of("1.20.1-forge-47.1.10", "47.1"));
        assert!(!is_profile_of("1.20.1-forge-47.1.10", "47.1.1"));
        assert!(!is_profile_of("1.20.1-forge-147.1", "47.1"));
        assert!(!is_profile_of("fabric-loader-47.1-1.20.1", "47.1"));
    }
}
