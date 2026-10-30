// SPDX-License-Identifier: GPL-3.0-only
//! Finds installed Java runtimes and downloads Eclipse Temurin JREs when none fits.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::net::{self, Download};
use crate::paths;
use crate::tasks::Progress;

const JAVA_BIN: &str = if cfg!(windows) { "java.exe" } else { "java" };

#[derive(Clone, Debug)]
pub struct JavaInstall {
    pub path: PathBuf,
    pub version: String,
    pub major: u32,
}

/// "1.8.0_402" → 8, "21.0.5" → 21, "17" → 17.
pub fn parse_major(version: &str) -> Option<u32> {
    let mut parts = version.split(|c: char| !c.is_ascii_digit());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// Reads `JAVA_VERSION` from the runtime's `release` file, which avoids starting a JVM.
fn probe(java_exe: &Path) -> Option<JavaInstall> {
    let home = java_exe.parent()?.parent()?;
    let release = std::fs::read_to_string(home.join("release")).ok()?;
    let line = release.lines().find(|l| l.starts_with("JAVA_VERSION="))?;
    let version = line.trim_start_matches("JAVA_VERSION=").trim_matches('"').to_string();
    Some(JavaInstall { major: parse_major(&version)?, path: java_exe.to_path_buf(), version })
}

fn children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
}

pub fn detect() -> Vec<JavaInstall> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);

    if let Some(home) = env("JAVA_HOME") {
        candidates.push(home.join("bin").join(JAVA_BIN));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(JAVA_BIN));
        }
    }
    let vendors = ["Java", "Eclipse Adoptium", "Eclipse Foundation", "Microsoft", "Zulu", "BellSoft", "Amazon Corretto", "Semeru"];
    for base in [env("ProgramFiles"), env("ProgramFiles(x86)"), env("LOCALAPPDATA").map(|p| p.join("Programs"))].into_iter().flatten() {
        for vendor in vendors {
            for home in children(&base.join(vendor)) {
                candidates.push(home.join("bin").join(JAVA_BIN));
            }
        }
    }
    // Runtimes bundled by the official launcher (classic and Microsoft Store builds).
    let mut runtime_roots = Vec::new();
    if let Some(appdata) = env("APPDATA") {
        runtime_roots.push(appdata.join(".minecraft").join("runtime"));
    }
    if let Some(local) = env("LOCALAPPDATA") {
        runtime_roots.push(local.join(r"Packages\Microsoft.4297127D64EC6_8wekyb3d8bbwe\LocalCache\Local\runtime"));
    }
    for root in runtime_roots {
        for component in children(&root) {
            for platform in children(&component) {
                for home in children(&platform) {
                    candidates.push(home.join("bin").join(JAVA_BIN));
                }
            }
        }
    }
    // Linux: distro packages, SDKMAN, IntelliJ downloads and the official launcher's runtimes.
    #[cfg(not(windows))]
    {
        let home = env("HOME").unwrap_or_default();
        for base in [
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/lib64/jvm"),
            PathBuf::from("/opt"),
            home.join(".sdkman/candidates/java"),
            home.join(".jdks"),
        ] {
            for jvm in children(&base) {
                candidates.push(jvm.join("bin").join(JAVA_BIN));
            }
        }
        for component in children(&home.join(".minecraft/runtime")) {
            for platform in children(&component) {
                for jvm in children(&platform) {
                    candidates.push(jvm.join("bin").join(JAVA_BIN));
                }
            }
        }
    }
    // Downloads land in java/<release>/<archive folder>/, so look one level deeper too.
    for home in children(&paths::java()) {
        candidates.push(home.join("bin").join(JAVA_BIN));
        for nested in children(&home) {
            candidates.push(nested.join("bin").join(JAVA_BIN));
        }
    }

    let mut seen = HashSet::new();
    let mut found: Vec<JavaInstall> = candidates
        .into_iter()
        .filter(|p| p.is_file())
        .filter_map(|p| std::fs::canonicalize(&p).ok().map(|c| (p, c)))
        .filter(|(_, canon)| seen.insert(canon.clone()))
        // The real path, so a PATH symlink like /usr/bin/java leads to the runtime's `release` file.
        .filter_map(|(p, canon)| probe(&canon).map(|j| JavaInstall { path: p, ..j }))
        .collect();
    found.sort_by_key(|j| std::cmp::Reverse(j.major));
    found
}

/// Exact major version first; for Java 16+ requirements a newer runtime also works.
pub fn pick(installs: &[JavaInstall], required: u32) -> Option<&JavaInstall> {
    installs
        .iter()
        .find(|j| j.major == required)
        .or_else(|| if required >= 16 { installs.iter().filter(|j| j.major >= required).min_by_key(|j| j.major) } else { None })
}

#[derive(Deserialize)]
struct AdoptiumAsset {
    binary: AdoptiumBinary,
    release_name: String,
}

#[derive(Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}

#[derive(Deserialize)]
struct AdoptiumPackage {
    link: String,
    checksum: String,
    name: String,
}

const ADOPTIUM_OS: &str = if cfg!(windows) {
    "windows"
} else if cfg!(target_os = "macos") {
    "mac"
} else {
    "linux"
};
/// Temurin JREs are zips on Windows and tarballs elsewhere.
const ARCHIVE_EXT: &str = if cfg!(windows) { ".zip" } else { ".tar.gz" };

#[cfg(windows)]
fn extract(archive: &Path, target: &Path) -> Result<()> {
    zip::ZipArchive::new(std::fs::File::open(archive)?)?.extract(target)?;
    Ok(())
}

/// Unpacks a `.tar.gz`; `tar` keeps the executable bits that `bin/java` needs.
#[cfg(not(windows))]
fn extract(archive: &Path, target: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
    tar::Archive::new(gz).unpack(target)?;
    Ok(())
}

static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn ensure(required: u32, progress: &Progress) -> Result<PathBuf> {
    // Scanning folders is blocking work; the runtime has only two worker threads.
    let installed = || async move {
        let installs = tokio::task::spawn_blocking(detect).await?;
        anyhow::Ok(pick(&installs, required).map(|j| j.path.clone()))
    };
    if let Some(path) = installed().await? {
        return Ok(path);
    }
    // One download at a time: two launches needing the same Java would otherwise unpack into the
    // same folder. The second one finds the first one's runtime after waiting.
    let _install = INSTALL.lock().await;
    if let Some(path) = installed().await? {
        return Ok(path);
    }
    // Temurin never shipped a Java 16 JRE; Minecraft 1.17 runs on 17.
    let major = if required == 16 { 17 } else { required };
    progress.set_label(format!("Downloading Java {major}"));
    let url = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os={ADOPTIUM_OS}&vendor=eclipse"
    );
    let assets: Vec<AdoptiumAsset> = net::get_json(&url).await?;
    let asset = assets
        .into_iter()
        .find(|a| a.binary.package.name.ends_with(ARCHIVE_EXT))
        .with_context(|| format!("no Java {major} download available"))?;
    let zip_path = paths::cache().join(&asset.binary.package.name);
    let _ = std::fs::remove_file(&zip_path);
    net::download_tracked(&Download::new(&asset.binary.package.link, &zip_path), Some(progress)).await?;

    progress.set_label(format!("Installing Java {major}"));
    // Verifying and extracting run on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    let expected = asset.binary.package.checksum.to_lowercase();
    let release = asset.release_name.clone();
    let java = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let mut file = std::fs::File::open(&zip_path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        if hex::encode(hasher.finalize()) != expected {
            bail!("Java download is corrupted (checksum mismatch)");
        }
        let target = paths::java().join(&release);
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target)?;
        extract(&zip_path, &target)?;
        let _ = std::fs::remove_file(&zip_path);
        // The archive contains a single top-level folder such as "jdk-21.0.5+11-jre".
        let home = children(&target).into_iter().next().unwrap_or(target);
        let exe = home.join("bin").join(JAVA_BIN);
        if !exe.is_file() {
            bail!("{JAVA_BIN} not found in the downloaded runtime");
        }
        Ok(exe)
    })
    .await??;
    Ok(java)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majors() {
        assert_eq!(parse_major("1.8.0_402"), Some(8));
        assert_eq!(parse_major("21.0.5"), Some(21));
        assert_eq!(parse_major("17"), Some(17));
        assert_eq!(parse_major(""), None);
    }

    #[test]
    fn picks_compatible_runtime() {
        let j = |m: u32| JavaInstall { path: PathBuf::new(), version: m.to_string(), major: m };
        let installs = [j(21), j(17), j(8)];
        assert_eq!(pick(&installs, 17).unwrap().major, 17);
        assert_eq!(pick(&installs, 8).unwrap().major, 8);
        assert_eq!(pick(&[j(21)], 17).unwrap().major, 21);
        assert!(pick(&[j(21)], 8).is_none());
    }
}
