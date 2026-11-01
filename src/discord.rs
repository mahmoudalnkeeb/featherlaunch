// SPDX-License-Identifier: GPL-3.0-only
//! Discord presence: shows what's being played on the player's profile, when they turn it on.
//!
//! This talks to the Discord app on the same PC over its local pipe. Nothing goes over the network
//! from here, and it needs no library, so it costs nothing while it's off.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use serde_json::{Value, json};

/// FeatherLaunch's Discord application. It's public, and modified builds can set their own.
const APP_ID: &str = match option_env!("FEATHERLAUNCH_DISCORD_APP_ID") {
    Some(id) => id,
    None => "1566564246484029602",
};
const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
/// How often to look for Discord while there's something to show and it isn't running.
const RETRY: Duration = Duration::from_secs(30);
/// Replies are small; a longer one means something else is on the pipe.
const MAX_FRAME: u32 = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub details: String,
    pub state: Option<String>,
    /// When the game started, in seconds since 1970.
    pub started: u64,
}

impl Activity {
    fn to_json(&self) -> Value {
        let mut activity = json!({
            "details": self.details,
            "timestamps": { "start": self.started },
            "assets": { "large_image": "logo", "large_text": "FeatherLaunch" },
        });
        if let Some(state) = &self.state {
            activity["state"] = json!(state);
        }
        activity
    }
}

fn write_frame(pipe: &mut impl Write, op: u32, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend(op.to_le_bytes());
    frame.extend((body.len() as u32).to_le_bytes());
    frame.extend(body.as_bytes());
    pipe.write_all(&frame)?;
    pipe.flush()
}

fn read_frame(pipe: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 8];
    pipe.read_exact(&mut header)?;
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if len > MAX_FRAME {
        return Err(std::io::Error::other("Discord's reply is too long"));
    }
    let mut body = vec![0; len as usize];
    pipe.read_exact(&mut body)?;
    Ok(body)
}

/// Says hello on a fresh connection.
fn handshake(pipe: &mut (impl Read + Write)) -> std::io::Result<()> {
    write_frame(pipe, HANDSHAKE, &json!({ "v": 1, "client_id": APP_ID }))?;
    read_frame(pipe).map(drop)
}

fn set_activity(pipe: &mut (impl Read + Write), activity: &Activity) -> std::io::Result<()> {
    let body = json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity.to_json() },
        "nonce": uuid::Uuid::new_v4().to_string(),
    });
    write_frame(pipe, FRAME, &body)?;
    read_frame(pipe).map(drop)
}

#[cfg(windows)]
type Pipe = std::fs::File;
#[cfg(not(windows))]
type Pipe = std::os::unix::net::UnixStream;

/// Discord uses the first free slot, so it can be any of these.
#[cfg(windows)]
fn connect() -> Option<Pipe> {
    (0..10).find_map(|n| std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}")).ok())
}

#[cfg(not(windows))]
fn connect() -> Option<Pipe> {
    let var = |k: &str| std::env::var(k).ok();
    let base = var("XDG_RUNTIME_DIR").or_else(|| var("TMPDIR")).unwrap_or_else(|| "/tmp".to_string());
    // Also where the Flatpak and Snap builds of Discord put it.
    let dirs = [base.clone(), format!("{base}/app/com.discordapp.Discord"), format!("{base}/snap.discord")];
    dirs.iter().flat_map(|dir| (0..10).map(move |n| format!("{dir}/discord-ipc-{n}"))).find_map(|path| Pipe::connect(path).ok())
}

/// Keeps the wanted activity showing. Closing the connection is what clears it.
fn run(updates: Receiver<Option<Activity>>) {
    let mut wanted: Option<Activity> = None;
    let mut pipe: Option<Pipe> = None;
    loop {
        // Only wakes on its own while there's something to show but no Discord to show it on.
        let next = if wanted.is_some() && pipe.is_none() {
            updates.recv_timeout(RETRY)
        } else {
            updates.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(update) => wanted = update,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(activity) = &wanted else {
            pipe = None;
            continue;
        };
        if pipe.is_none() {
            pipe = connect().and_then(|mut p| handshake(&mut p).ok().map(|()| p));
        }
        if let Some(p) = &mut pipe
            && set_activity(p, activity).is_err()
        {
            pipe = None;
        }
    }
}

/// The handle the UI keeps. The worker thread starts with the first update, so it never exists
/// for people who leave this off.
#[derive(Default)]
pub struct Presence {
    updates: Option<Sender<Option<Activity>>>,
}

impl Presence {
    /// Shows `activity`, or clears the presence when given `None`.
    pub fn set(&mut self, activity: Option<Activity>) {
        if self.updates.is_none() && activity.is_none() {
            return;
        }
        let updates = self.updates.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || run(rx));
            tx
        });
        let _ = updates.send(activity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, FRAME, &json!({ "a": 1 })).unwrap();
        assert_eq!(&bytes[..4], &1u32.to_le_bytes());
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), br#"{"a":1}"#);
    }

    #[test]
    fn refuses_a_huge_frame() {
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(u32::MAX.to_le_bytes());
        assert!(read_frame(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn leaves_out_what_is_not_set() {
        let activity = Activity { details: "Minecraft 1.21.1".into(), state: None, started: 7 };
        let value = activity.to_json();
        assert_eq!(value["details"], "Minecraft 1.21.1");
        assert_eq!(value["timestamps"]["start"], 7);
        assert_eq!(value["assets"]["large_image"], "logo");
        assert!(value.get("state").is_none());
        assert_eq!(Activity { state: Some("Pack".into()), ..activity }.to_json()["state"], "Pack");
    }

    #[test]
    fn nothing_starts_until_there_is_something_to_show() {
        let mut presence = Presence::default();
        presence.set(None);
        assert!(presence.updates.is_none());
    }
}
