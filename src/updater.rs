// SPDX-License-Identifier: GPL-3.0-only
//! Finds newer launcher releases and replaces the running executable with them.
//!
//! Releases come from this repository, the same place the website downloads from.
//! Every file is signed with minisign in CI, and nothing is installed without a good signature.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

use crate::net::{self, Download, RemoveOnDrop};
use crate::tasks::Progress;

const LATEST: &str = "https://api.github.com/repos/mahmoudalnkeeb/featherlaunch/releases/latest";
const PUBLIC_KEY: &str = "RWRoJyw/nrod7+416zXGtB4nU6iCbeALkv7LKpTEHxIh9pIqRAYbMTO7";
const ASSET: &str = if cfg!(windows) { "featherlaunch.exe" } else { "featherlaunch-linux-x64.tar.gz" };
#[cfg(not(windows))]
const ARCHIVE_BIN: &str = "featherlaunch/featherlaunch";

/// Suffix of the previous executable, which stays locked on Windows until its process exits.
const OLD: &str = ".old";
const RETRIES: u32 = 10;
const RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page: String,
    asset_url: String,
    asset_size: u64,
    signature_url: String,
}

fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.').map(|p| p.parse().ok());
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

/// Unparseable versions are never newer, so a malformed tag can't offer the same update forever.
fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_version(tag), parse_version(current)), (Some(tag), Some(current)) if tag > current)
}

/// A release still uploading lacks a file or its signature, and counts as no update yet.
fn select(release: GithubRelease, current: &str) -> Option<Release> {
    if !is_newer(&release.tag_name, current) {
        return None;
    }
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let asset = find(ASSET)?;
    let signature = find(&format!("{ASSET}.minisig"))?;
    Some(Release {
        version: release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name).to_string(),
        page: release.html_url.clone(),
        asset_url: asset.browser_download_url.clone(),
        asset_size: asset.size,
        signature_url: signature.browser_download_url.clone(),
    })
}

/// The newest release if it is newer than this build. Development builds never update.
pub async fn check() -> Result<Option<Release>> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    let release: GithubRelease = net::get_json(LATEST).await?;
    Ok(select(release, env!("CARGO_PKG_VERSION")))
}

/// Checks the signature, and that it was made for this version and file: its trusted comment is
/// signed too, so an old signed build can't be served under a new tag.
fn verify(bytes: &[u8], signature: &str, expected_comment: &str, public_key: &str) -> Result<()> {
    let key = PublicKey::from_base64(public_key).context("reading the update signing key")?;
    let signature = Signature::decode(signature).context("reading the update's signature")?;
    key.verify(bytes, &signature, false).context("the update's signature is not valid")?;
    if signature.trusted_comment() != expected_comment {
        bail!("the update's signature is for \"{}\", not \"{expected_comment}\"", signature.trusted_comment());
    }
    Ok(())
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Antivirus can briefly hold a fresh executable, and a closing launcher briefly keeps its own
/// locked. Waiting won't help if the file is missing.
fn retry(mut step: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    for _ in 1..RETRIES {
        match step() {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => std::thread::sleep(RETRY_DELAY),
            result => return result,
        }
    }
    step()
}

#[cfg(windows)]
fn unpack(download: &Path, new: &Path) -> Result<()> {
    std::fs::rename(download, new)?;
    Ok(())
}

#[cfg(not(windows))]
fn unpack(download: &Path, new: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let gz = flate2::read::GzDecoder::new(std::fs::File::open(download)?);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(ARCHIVE_BIN) {
            std::io::copy(&mut entry, &mut std::fs::File::create(new)?)?;
            std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
            return Ok(());
        }
    }
    bail!("{ARCHIVE_BIN} is missing from the update");
}

/// Flushes `new` to disk, so a power cut can't leave a truncated launcher, and installs it.
fn swap(exe: &Path, new: &Path) -> Result<()> {
    std::fs::File::options().write(true).open(new)?.sync_all()?;
    replace(exe, new)
}

/// Puts `new` in place of `exe`. A running executable can't be overwritten on Windows but it can
/// be renamed, and both files share a folder, so the renames can't cross drives.
fn replace(exe: &Path, new: &Path) -> Result<()> {
    let old = sibling(exe, OLD);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).with_context(|| format!("moving {} aside", exe.display()))?;
    if let Err(e) = retry(|| std::fs::rename(new, exe)) {
        let _ = std::fs::rename(&old, exe);
        return Err(e).with_context(|| format!("replacing {}", exe.display()));
    }
    Ok(())
}

/// Downloads, verifies and installs `release`, and returns the launcher's path. The new version
/// starts on the next launch. The path is taken here because `current_exe` follows the running
/// file on Linux, which from now on is the `.old` one.
pub async fn install(release: &Release, progress: &Progress) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding the launcher's own path")?;
    let dir = exe.parent().unwrap_or(&exe).to_path_buf();
    let (download, new) = (sibling(&exe, ".download"), sibling(&exe, ".new"));
    // Both are gone after a successful install; these catch a failure or a cancel.
    let _cleanup = (RemoveOnDrop(download.clone()), RemoveOnDrop(new.clone()));
    let _ = std::fs::remove_file(&download);
    let _ = std::fs::remove_file(&new);

    progress.set_label("Downloading");
    let signature = net::client().get(&release.signature_url).send().await?.error_for_status()?.text().await?;
    progress.add_bytes_total(release.asset_size);
    let file = Download { url: release.asset_url.clone(), path: download.clone(), sha1: None, size: Some(release.asset_size) };
    net::download_tracked(&file, Some(progress)).await.with_context(|| format!("saving the update in {}", dir.display()))?;

    progress.set_label("Installing");
    let _critical = progress.critical();
    let comment = format!("featherlaunch {} {ASSET}", release.version);
    let target = exe.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        verify(&std::fs::read(&download)?, &signature, &comment, PUBLIC_KEY)?;
        unpack(&download, &new)?;
        swap(&target, &new)
    })
    .await??;
    Ok(exe)
}

pub fn restart(exe: &Path) -> std::io::Result<()> {
    std::process::Command::new(exe).spawn().map(drop)
}

pub fn clean_old() {
    if let Ok(exe) = std::env::current_exe() {
        let old = sibling(&exe, OLD);
        let _ = retry(|| std::fs::remove_file(&old));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "RWR7oY1vE3SB5YCHUOYiZIphwsa/mbH6+OLQq5BIFP1OuBQ+kqeMhMfu";
    const PAYLOAD: &[u8] = b"a new launcher";
    const COMMENT: &str = "featherlaunch 9.9.9 featherlaunch.exe";
    const SIGNATURE: &str = "untrusted comment: signature from rsign secret key
RUR7oY1vE3SB5ej256im1PYkDVdurF1ezKh2Bv88hNRwhrGypuHpfgYI8Wz9n0RrcMx2/0vW75zj2RJk6GhRT8i8VYV89jCnTw0=
trusted comment: featherlaunch 9.9.9 featherlaunch.exe
PR13SKdrBYk+Bxh0BXI9+Hg1WVg0jjJYusQcH/4Z81JGnkTGEoVX6Pg8cSQu0j0Jnx/Nk/o9FwevnibFnC7LCg==
";

    fn asset(name: &str) -> GithubAsset {
        GithubAsset { name: name.to_string(), size: 7, browser_download_url: format!("https://example.com/{name}") }
    }

    fn github(tag: &str, assets: Vec<GithubAsset>) -> GithubRelease {
        GithubRelease { tag_name: tag.to_string(), html_url: "https://example.com/release".to_string(), assets }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn compares_versions_as_numbers() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
    }

    #[test]
    fn odd_tags_are_never_newer() {
        for tag in ["latest", "v1.2", "v1.2.3.4", "v1.2.3-beta", ""] {
            assert!(!is_newer(tag, "0.1.0"), "{tag}");
        }
    }

    #[test]
    fn selects_this_platforms_file_and_signature() {
        let release = select(github("v0.2.0", vec![asset("other.zip"), asset(ASSET), asset(&format!("{ASSET}.minisig"))]), "0.1.0");
        let release = release.unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.page, "https://example.com/release");
        assert_eq!(release.asset_url, format!("https://example.com/{ASSET}"));
        assert_eq!(release.signature_url, format!("https://example.com/{ASSET}.minisig"));
    }

    #[test]
    fn a_release_still_uploading_is_not_offered() {
        assert_eq!(select(github("v0.2.0", vec![asset(ASSET)]), "0.1.0"), None);
        assert_eq!(select(github("v0.2.0", vec![asset(&format!("{ASSET}.minisig"))]), "0.1.0"), None);
    }

    #[test]
    fn the_current_version_is_not_offered() {
        assert_eq!(select(github("v0.1.0", vec![asset(ASSET), asset(&format!("{ASSET}.minisig"))]), "0.1.0"), None);
    }

    #[test]
    fn accepts_a_good_signature() {
        verify(PAYLOAD, SIGNATURE, COMMENT, TEST_KEY).unwrap();
    }

    #[test]
    fn rejects_changed_bytes() {
        assert!(verify(b"a bad launcher", SIGNATURE, COMMENT, TEST_KEY).is_err());
    }

    #[test]
    fn rejects_a_signature_for_another_version() {
        assert!(verify(PAYLOAD, SIGNATURE, "featherlaunch 9.9.10 featherlaunch.exe", TEST_KEY).is_err());
    }

    #[test]
    fn rejects_an_edited_trusted_comment() {
        let edited = SIGNATURE.replace("9.9.9", "9.9.10");
        assert!(verify(PAYLOAD, &edited, "featherlaunch 9.9.10 featherlaunch.exe", TEST_KEY).is_err());
    }

    #[test]
    fn rejects_another_key() {
        let other = "RWSgzfeTrNXNk6RBUu0esUz27vTX7HT21D5bMj6Ntz7V3VE2F3pa9q34";
        assert!(verify(PAYLOAD, SIGNATURE, COMMENT, other).is_err());
    }

    #[test]
    fn swap_keeps_the_old_file_until_the_next_start() {
        let dir = temp_dir();
        let (exe, new) = (dir.join("launcher"), dir.join("launcher.new"));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("launcher.old")).unwrap(), "old");
        assert!(!new.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn swap_replaces_a_leftover_old_file() {
        let dir = temp_dir();
        let (exe, new) = (dir.join("launcher"), dir.join("launcher.new"));
        std::fs::write(&exe, "current").unwrap();
        std::fs::write(&new, "new").unwrap();
        std::fs::write(dir.join("launcher.old"), "ancient").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("launcher.old")).unwrap(), "current");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn swap_puts_the_old_file_back_when_the_new_one_cant_move() {
        let dir = temp_dir();
        let exe = dir.join("launcher");
        std::fs::write(&exe, "old").unwrap();
        // With no new file, the executable is moved aside and then has nothing to replace it.
        assert!(replace(&exe, &dir.join("launcher.new")).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(!dir.join("launcher.old").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn unpacks_the_launcher_as_an_executable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir();
        let (archive, new) = (dir.join("update.tar.gz"), dir.join("launcher.new"));
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut add = |path: &str, data: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, data).unwrap();
        };
        add("featherlaunch/featherlaunch.png", b"icon");
        add(ARCHIVE_BIN, b"launcher");
        tar.into_inner().unwrap().finish().unwrap();
        unpack(&archive, &new).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), b"launcher");
        assert_eq!(std::fs::metadata(&new).unwrap().permissions().mode() & 0o777, 0o755);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
