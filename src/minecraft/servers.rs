// SPDX-License-Identifier: GPL-3.0-only
//! The game's server list, `servers.dat`, and asking a server for its status.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::minecraft::world::{child, entry, skip};

const TIMEOUT: Duration = Duration::from_secs(3);
/// Far beyond any real status reply, so a hostile server can't fill the memory.
const MAX_REPLY: usize = 1 << 20;
const DEFAULT_PORT: u16 = 25565;

#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub name: String,
    pub ip: String,
    /// The server's whole entry, so details the launcher doesn't know, like its icon, survive.
    raw: Vec<u8>,
}

fn nbt_string(tag: &mut Vec<u8>, name: &str, value: &str) {
    tag.push(8);
    tag.extend((name.len() as u16).to_be_bytes());
    tag.extend(name.as_bytes());
    tag.extend((value.len() as u16).to_be_bytes());
    tag.extend(value.as_bytes());
}

impl Server {
    pub fn new(name: &str, ip: &str) -> Self {
        let mut raw = Vec::new();
        nbt_string(&mut raw, "name", name);
        nbt_string(&mut raw, "ip", ip);
        raw.push(0);
        Self { name: name.to_string(), ip: ip.to_string(), raw }
    }
}

fn text(raw: &[u8], key: &[u8]) -> Option<String> {
    let (at, 8) = child(raw, 0, key)? else { return None };
    let end = skip(raw, at, 8, 0)?;
    Some(String::from_utf8_lossy(raw.get(at + 2..end)?).into_owned())
}

fn parse(b: &[u8]) -> Option<Vec<Server>> {
    let (root, Some((10, _))) = entry(b, 0)? else { return None };
    let Some((list, 9)) = child(b, root, b"servers") else { return Some(Vec::new()) };
    if *b.get(list)? != 10 {
        return Some(Vec::new());
    }
    let count = i32::from_be_bytes(b.get(list + 1..list + 5)?.try_into().ok()?).max(0);
    let mut at = list + 5;
    let mut out = Vec::new();
    for _ in 0..count {
        let end = skip(b, at, 10, 0)?;
        let raw = b.get(at..end)?.to_vec();
        out.push(Server { name: text(&raw, b"name").unwrap_or_default(), ip: text(&raw, b"ip")?, raw });
        at = end;
    }
    Some(out)
}

fn encode(servers: &[Server]) -> Vec<u8> {
    let mut out = vec![10, 0, 0, 9, 0, 7];
    out.extend(b"servers");
    out.push(10);
    out.extend((servers.len() as i32).to_be_bytes());
    for server in servers {
        out.extend(&server.raw);
    }
    out.push(0);
    out
}

/// A file that can't be read is an error rather than an empty list, so saving can't wipe it.
pub fn load(game_dir: &Path) -> Result<Vec<Server>> {
    let bytes = match std::fs::read(game_dir.join("servers.dat")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading servers.dat"),
    };
    parse(&bytes).context("servers.dat is damaged")
}

pub fn save(game_dir: &Path, servers: &[Server]) -> Result<()> {
    std::fs::create_dir_all(game_dir)?;
    let tmp = game_dir.join("servers.dat.tmp");
    std::fs::write(&tmp, encode(servers))?;
    std::fs::rename(&tmp, game_dir.join("servers.dat"))?;
    Ok(())
}

fn varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(stream: &mut impl Read) -> Result<u32> {
    let mut value = 0;
    for shift in 0..5 {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        value |= u32::from(byte[0] & 0x7f) << (shift * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    anyhow::bail!("the server sent a bad number")
}

fn split_address(address: &str) -> (&str, u16) {
    match address.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => (host, port.parse().unwrap_or(DEFAULT_PORT)),
        _ => (address, DEFAULT_PORT),
    }
}

/// Asks a server how it's doing, as the game's server list does. Returns a line for the player.
/// SRV records aren't looked up, so an address that relies on one shows as offline.
pub fn status(address: &str) -> Result<String> {
    let (host, port) = split_address(address);
    let target = (host, port).to_socket_addrs()?.next().context("no such server")?;
    let mut stream = TcpStream::connect_timeout(&target, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut handshake = vec![0];
    varint(767, &mut handshake);
    varint(host.len() as u32, &mut handshake);
    handshake.extend(host.as_bytes());
    handshake.extend(port.to_be_bytes());
    varint(1, &mut handshake);
    let mut packet = Vec::new();
    varint(handshake.len() as u32, &mut packet);
    packet.extend(handshake);
    packet.extend([1, 0]); // The status request.
    stream.write_all(&packet)?;

    read_varint(&mut stream)?;
    read_varint(&mut stream)?;
    let len = read_varint(&mut stream)? as usize;
    anyhow::ensure!(len <= MAX_REPLY, "the server's reply is too long");
    let mut body = vec![0; len];
    stream.read_exact(&mut body)?;
    let reply: serde_json::Value = serde_json::from_slice(&body)?;
    let players = &reply["players"];
    Ok(format!("{}/{} players  ·  {}", players["online"], players["max"], reply["version"]["name"].as_str().unwrap_or("Unknown version")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_it_does_not_know_about_a_server() {
        let mut raw = Vec::new();
        nbt_string(&mut raw, "icon", "base64data");
        nbt_string(&mut raw, "name", "My Server");
        nbt_string(&mut raw, "ip", "play.example:25566");
        raw.push(0);
        let original = Server { name: "My Server".into(), ip: "play.example:25566".into(), raw };

        let list = vec![original.clone(), Server::new("Second", "two.example")];
        let bytes = encode(&list);
        assert_eq!(parse(&bytes).unwrap(), list);
        assert!(bytes.windows(10).any(|w| w == b"base64data"));
    }

    #[test]
    fn reads_an_empty_or_missing_list() {
        assert_eq!(parse(&encode(&[])), Some(Vec::new()));
        assert_eq!(parse(&[10, 0, 0, 0]), Some(Vec::new()));
        assert_eq!(parse(&[1, 2, 3]), None);
    }

    #[test]
    fn saves_and_loads_a_list() {
        let dir = std::env::temp_dir().join(format!("featherlaunch-test-{}", uuid::Uuid::new_v4().simple()));
        assert!(load(&dir).unwrap().is_empty());
        save(&dir, &[Server::new("A", "a.example")]).unwrap();
        assert_eq!(load(&dir).unwrap()[0].ip, "a.example");
        std::fs::write(dir.join("servers.dat"), [1, 2, 3]).unwrap();
        assert!(load(&dir).is_err(), "a damaged file must not read as an empty list");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn numbers_round_trip() {
        for value in [0, 1, 127, 128, 767, 25565, 2_097_151] {
            let mut bytes = Vec::new();
            varint(value, &mut bytes);
            assert_eq!(read_varint(&mut bytes.as_slice()).unwrap(), value);
        }
        assert!(read_varint(&mut [0xff; 6].as_slice()).is_err());
    }

    #[test]
    fn splits_addresses() {
        assert_eq!(split_address("play.example"), ("play.example", 25565));
        assert_eq!(split_address("play.example:1234"), ("play.example", 1234));
    }

    #[test]
    fn asks_a_server_for_its_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 64];
            let _ = stream.read(&mut request).unwrap();
            let json = br#"{"version":{"name":"1.21.1"},"players":{"online":3,"max":20}}"#;
            let mut body = vec![0];
            varint(json.len() as u32, &mut body);
            body.extend(json);
            let mut reply = Vec::new();
            varint(body.len() as u32, &mut reply);
            reply.extend(body);
            stream.write_all(&reply).unwrap();
        });
        assert_eq!(status(&format!("127.0.0.1:{port}")).unwrap(), "3/20 players  ·  1.21.1");
    }
}
