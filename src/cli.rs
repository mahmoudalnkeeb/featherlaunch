// SPDX-License-Identifier: GPL-3.0-only
//! Command-line arguments, for shortcuts and scripts.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// An instance's folder name or display name, to start right away.
    pub launch: Option<String>,
    pub server: Option<String>,
    pub world: Option<String>,
    /// The account to use, by player name.
    pub profile: Option<String>,
    pub offline: bool,
    /// Keeps all data in this folder.
    pub dir: Option<PathBuf>,
}

/// Options that need a value and don't get one are ignored, like unknown ones, so a broken
/// shortcut still opens the launcher.
pub fn parse(mut args: impl Iterator<Item = String>) -> Args {
    let mut out = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--launch" => out.launch = args.next(),
            "--server" => out.server = args.next(),
            "--world" => out.world = args.next(),
            "--profile" => out.profile = args.next(),
            "--dir" => out.dir = args.next().map(PathBuf::from),
            "--offline" => out.offline = true,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Args {
        parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn reads_every_option() {
        let args = parsed(&[
            "--launch",
            "My Pack",
            "--server",
            "play.example:25566",
            "--world",
            "Home",
            "--profile",
            "Sam",
            "--offline",
            "--dir",
            "D:/data",
        ]);
        assert_eq!(args.launch.as_deref(), Some("My Pack"));
        assert_eq!(args.server.as_deref(), Some("play.example:25566"));
        assert_eq!(args.world.as_deref(), Some("Home"));
        assert_eq!(args.profile.as_deref(), Some("Sam"));
        assert!(args.offline);
        assert_eq!(args.dir, Some(PathBuf::from("D:/data")));
    }

    #[test]
    fn ignores_what_it_does_not_know() {
        assert_eq!(parsed(&["--nope", "x"]), Args::default());
        assert_eq!(parsed(&["--launch"]), Args::default());
    }
}
