// SPDX-License-Identifier: GPL-3.0-only
//! `launcher.log` in the data folder. Release builds on Windows have no console, so this is
//! where startup problems, task errors and crashes end up for bug reports.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths;

/// The log starts over past this size, so it can't grow without bound.
const MAX_BYTES: u64 = 512 * 1024;

static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    paths::root().join("launcher.log")
}

pub fn error(message: &str) {
    eprintln!("{message}");
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = path();
    let append = std::fs::metadata(&path).is_ok_and(|m| m.len() < MAX_BYTES);
    let file = OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path);
    if let Ok(mut file) = file {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(file, "[{secs}] {message}");
    }
}

/// Records panics before the process aborts (release builds use `panic = "abort"`).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error(&format!("crash: {info}"));
        default(info);
    }));
}
