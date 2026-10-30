// SPDX-License-Identifier: GPL-3.0-only
//! HTTP client and hash-verified downloads with retries.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

use crate::tasks::Progress;

pub const USER_AGENT: &str = concat!("mahmoudalnkeeb/featherlaunch/", env!("CARGO_PKG_VERSION"));

pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP client")
    })
}

pub async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let resp = client().get(url).send().await.with_context(|| format!("GET {url}"))?;
    let resp = resp.error_for_status().with_context(|| format!("GET {url}"))?;
    resp.json().await.with_context(|| format!("parsing {url}"))
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub fn sha1_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

#[derive(Clone, Debug)]
pub struct Download {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

impl Download {
    pub fn new(url: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self { url: url.into(), path: path.into(), sha1: None, size: None }
    }

    /// Existing files are trusted when their size matches: hashing every library on every
    /// launch is too slow on old disks. Hashes are checked right after downloading instead.
    fn is_present(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(meta) => self.size.is_none_or(|s| s == 0 || meta.len() == s),
            Err(_) => false,
        }
    }
}

pub async fn download(d: &Download) -> Result<()> {
    download_tracked(d, None).await
}

/// Like [`download`], also reporting the file's name and bytes to `progress`. When `d.size` is
/// set the caller has already counted it in the byte total; otherwise the response's length is.
pub async fn download_tracked(d: &Download, progress: Option<&Progress>) -> Result<()> {
    if d.is_present() {
        return Ok(());
    }
    if d.url.is_empty() {
        bail!("{} is missing and has no download URL", d.path.display());
    }
    if let Some(parent) = d.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(p) = progress {
        p.file_started(&name);
    }
    const ATTEMPTS: u64 = 3;
    let mut attempt = 1;
    let result = loop {
        match try_download(d, progress).await {
            Err(e) if attempt < ATTEMPTS && is_retryable(&e) => {
                tokio::time::sleep(Duration::from_millis(400 * attempt)).await;
                attempt += 1;
            }
            result => break result,
        }
    };
    if let Some(p) = progress {
        p.file_finished(&name);
    }
    result.with_context(|| format!("downloading {}", d.url))
}

/// A file that arrived with the wrong hash. The server would send the same bytes again.
#[derive(Debug)]
struct ChecksumMismatch {
    expected: String,
    got: String,
}

impl std::fmt::Display for ChecksumMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checksum mismatch (expected {}, got {})", self.expected, self.got)
    }
}

impl std::error::Error for ChecksumMismatch {}

/// Errors other than a server refusal or a bad file can be temporary. This includes local errors,
/// because on Windows an antivirus can hold a fresh file for a moment.
fn is_retryable(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<ChecksumMismatch>().is_some() {
        return false;
    }
    let status = error.downcast_ref::<reqwest::Error>().and_then(reqwest::Error::status);
    status.is_none_or(|s| !s.is_client_error() || s == reqwest::StatusCode::TOO_MANY_REQUESTS || s == reqwest::StatusCode::REQUEST_TIMEOUT)
}

async fn try_download(d: &Download, progress: Option<&Progress>) -> Result<()> {
    let resp = client().get(&d.url).send().await?.error_for_status()?;
    // A size the caller didn't know is counted here, and taken back if this attempt fails.
    let added_total = match (progress, d.size) {
        (Some(p), None) => {
            let len = resp.content_length().unwrap_or(0);
            p.add_bytes_total(len);
            len
        }
        _ => 0,
    };
    let mut received = 0u64;
    let result = write_body(d, resp, progress, &mut received).await;
    if result.is_err()
        && let Some(p) = progress
    {
        p.undo_bytes(received, added_total);
    }
    result
}

async fn write_body(d: &Download, resp: reqwest::Response, progress: Option<&Progress>, received: &mut u64) -> Result<()> {
    // Unique per attempt: two tasks may fetch the same shared library at once, and a shared
    // temporary file would mix their bytes.
    let mut tmp = d.path.clone().into_os_string();
    tmp.push(format!(".{}.part", uuid::Uuid::new_v4().simple()));
    let tmp = PathBuf::from(tmp);
    // Removes the partial file on failure or when the task is cancelled; after the rename below
    // there is nothing left to remove.
    let _part = RemoveOnDrop(tmp.clone());
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        *received += chunk.len() as u64;
        if let Some(p) = progress {
            p.add_bytes(chunk.len() as u64);
        }
    }
    file.flush().await?;
    drop(file);
    if let Some(expected) = &d.sha1 {
        let got = hex::encode(hasher.finalize());
        if !got.eq_ignore_ascii_case(expected) {
            return Err(ChecksumMismatch { expected: expected.clone(), got }.into());
        }
    }
    tokio::fs::rename(&tmp, &d.path).await?;
    Ok(())
}

/// Deletes a file when dropped, including when a cancelled task's future is dropped mid-step.
/// Once the file has been moved away there is nothing left to delete.
pub struct RemoveOnDrop(pub PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Downloads everything with at most `parallel` requests in flight. Stops at the first failure,
/// so a dropped connection is reported at once instead of after every file has retried.
pub async fn download_all(items: Vec<Download>, parallel: usize, progress: &Progress) -> Result<()> {
    progress.add_total(items.len() as u64);
    // Count the known sizes of missing files up front, so the byte bar doesn't jump around.
    let known: u64 = items.iter().filter(|d| !d.is_present()).filter_map(|d| d.size).sum();
    progress.add_bytes_total(known);
    stream::iter(items)
        .map(|d| async move {
            download_tracked(&d, Some(progress)).await?;
            progress.inc();
            Ok(())
        })
        .buffer_unordered(parallel.max(1))
        .try_collect()
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    use super::*;

    async fn serve(replies: Vec<(u16, &'static [u8])>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file.jar", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (status, body) = replies[counter.fetch_add(1, Ordering::SeqCst).min(replies.len() - 1)];
                let mut request = [0u8; 2048];
                let _ = socket.read(&mut request).await;
                let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(&[head.as_bytes(), body].concat()).await;
            }
        });
        (url, hits)
    }

    fn target() -> PathBuf {
        std::env::temp_dir().join(format!("featherlaunch-test-{}.jar", uuid::Uuid::new_v4().simple()))
    }

    #[tokio::test]
    async fn gone_files_fail_after_one_attempt() {
        for status in [404, 403] {
            let (url, hits) = serve(vec![(status, b"")]).await;
            let path = target();
            let error = download(&Download::new(url, &path)).await.unwrap_err();
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert!(format!("{error:#}").contains(&status.to_string()), "{error:#}");
            assert!(!path.exists());
        }
    }

    #[tokio::test]
    async fn checksum_mismatch_fails_after_one_attempt() {
        let (url, hits) = serve(vec![(200, b"wrong file")]).await;
        let path = target();
        let mut d = Download::new(url, &path);
        d.sha1 = Some("0".repeat(40));
        let error = download(&d).await.unwrap_err();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(format!("{error:#}").contains("checksum mismatch"), "{error:#}");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn temporary_failures_are_retried() {
        for status in [503, 429] {
            let (url, hits) = serve(vec![(status, b""), (200, b"the file")]).await;
            let path = target();
            download(&Download::new(url, &path)).await.unwrap();
            assert_eq!(hits.load(Ordering::SeqCst), 2);
            assert_eq!(std::fs::read(&path).unwrap(), b"the file");
            std::fs::remove_file(&path).unwrap();
        }
    }

    #[tokio::test]
    async fn gives_up_after_three_attempts() {
        let (url, hits) = serve(vec![(503, b"")]).await;
        assert!(download(&Download::new(url, target())).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn partial_files_are_removed_when_dropped() {
        let path = std::env::temp_dir().join(format!("featherlaunch-test-{}.part", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, b"half a download").unwrap();
        drop(RemoveOnDrop(path.clone()));
        assert!(!path.exists());
    }
}
