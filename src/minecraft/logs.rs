// SPDX-License-Identifier: GPL-3.0-only
//! Logs the game left on disk, and sharing a log as a mclo.gs link.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;

use crate::net;

const UPLOAD_URL: &str = "https://api.mclo.gs/1/log";
/// mclo.gs keeps this much of a log and cuts the rest; the end is where crashes are.
const UPLOAD_LINES: usize = 25_000;

#[derive(Clone, Debug)]
pub struct SavedLog {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

/// The files in `dir` whose names pass `wanted`, newest first.
pub fn files_in(dir: &Path, wanted: fn(&str) -> bool) -> Vec<SavedLog> {
    let mut files: Vec<SavedLog> = named_files(dir, wanted).collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.modified));
    files
}

fn named_files(dir: &Path, wanted: fn(&str) -> bool) -> impl Iterator<Item = SavedLog> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(move |e| {
        let name = e.file_name().into_string().ok().filter(|n| wanted(n))?;
        Some(SavedLog { name, path: e.path(), modified: e.metadata().and_then(|m| m.modified()).ok() })
    })
}

/// The game logs and crash reports in a game folder, newest first.
pub fn list(game: &Path) -> Vec<SavedLog> {
    let mut logs: Vec<SavedLog> = named_files(&game.join("logs"), |n| n.ends_with(".log") || n.ends_with(".log.gz"))
        .chain(named_files(&game.join("crash-reports"), |n| n.ends_with(".txt")))
        .collect();
    logs.sort_by_key(|l| std::cmp::Reverse(l.modified));
    logs
}

pub fn read(path: &Path) -> Result<String> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = if path.extension().is_some_and(|e| e == "gz") {
        let mut out = Vec::new();
        GzDecoder::new(raw.as_slice()).read_to_end(&mut out).with_context(|| format!("unpacking {}", path.display()))?;
        out
    } else {
        raw
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Removes what shouldn't leave the PC: known tokens, anything shaped like an access token, and
/// the home folder, which usually holds the person's real name.
pub fn redact(text: &str, tokens: &[String], home: Option<&str>) -> String {
    let mut out = text.to_string();
    for token in tokens.iter().filter(|t| !t.is_empty()) {
        out = out.replace(token.as_str(), "<hidden>");
    }
    if let Some(home) = home.filter(|h| !h.is_empty()) {
        out = out.replace(home, "~").replace(&home.replace('\\', "/"), "~");
    }
    let mut hide_next = false;
    let lines: Vec<String> = out
        .lines()
        .map(|line| {
            let mut words = Vec::new();
            for word in line.split(' ') {
                let jwt = word.starts_with("eyJ") && word.matches('.').count() == 2;
                words.push(if hide_next || jwt { "<hidden>" } else { word });
                hide_next = word == "--accessToken" || word == "--session";
            }
            hide_next = false;
            words.join(" ")
        })
        .collect();
    lines.join("\n")
}

fn tail(text: &str, lines: usize) -> &str {
    match text.rmatch_indices('\n').nth(lines - 1) {
        Some((at, _)) => &text[at + 1..],
        None => text,
    }
}

#[derive(Deserialize)]
struct UploadResponse {
    url: Option<String>,
    error: Option<String>,
}

/// Uploads a log that has been through [`redact`] and returns its link.
pub async fn upload(text: &str) -> Result<String> {
    let response: UploadResponse =
        net::client().post(UPLOAD_URL).form(&[("content", tail(text, UPLOAD_LINES))]).send().await?.error_for_status()?.json().await?;
    response.url.with_context(|| response.error.unwrap_or_else(|| "mclo.gs sent no link".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_tokens_and_the_home_folder() {
        let text = "java --accessToken abc --version 1\nopened C:\\Users\\Sam\\x and C:/Users/Sam/y\nsecret-token here\nkey eyJhbGci.eyJzdWIi.c2ln end";
        let out = redact(text, &["secret-token".to_string()], Some("C:\\Users\\Sam"));
        assert_eq!(out, "java --accessToken <hidden> --version 1\nopened ~\\x and ~/y\n<hidden> here\nkey <hidden> end");
    }

    #[test]
    fn keeps_a_short_log_whole_and_the_end_of_a_long_one() {
        assert_eq!(tail("a\nb", 5), "a\nb");
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
    }

    #[test]
    fn reads_plain_and_compressed_logs() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.log"), "plain").unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"packed").unwrap();
        std::fs::write(dir.join("old.log.gz"), gz.finish().unwrap()).unwrap();
        assert_eq!(read(&dir.join("latest.log")).unwrap(), "plain");
        assert_eq!(read(&dir.join("old.log.gz")).unwrap(), "packed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_logs_and_crash_reports_only() {
        let game = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(game.join("logs")).unwrap();
        std::fs::create_dir_all(game.join("crash-reports")).unwrap();
        for file in ["logs/latest.log", "logs/a.log.gz", "logs/notes.txt", "crash-reports/crash.txt"] {
            std::fs::write(game.join(file), "x").unwrap();
        }
        let mut names: Vec<String> = list(&game).into_iter().map(|l| l.name).collect();
        names.sort();
        std::fs::remove_dir_all(&game).unwrap();
        assert_eq!(names, ["a.log.gz", "crash.txt", "latest.log"]);
    }
}
