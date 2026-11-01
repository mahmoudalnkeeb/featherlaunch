// SPDX-License-Identifier: GPL-3.0-only
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnLaunch {
    KeepOpen,
    Minimize,
    Close,
}

/// G1 flags that keep pauses short on slow CPUs. Applies to Java 8 through 21+.
pub const LOW_END_JVM_ARGS: &str = "-XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:+ParallelRefProcEnabled \
-XX:MaxGCPauseMillis=200 -XX:+DisableExplicitGC -XX:G1NewSizePercent=30 -XX:G1MaxNewSizePercent=40 \
-XX:G1HeapRegionSize=8M -XX:G1ReservePercent=20 -XX:InitiatingHeapOccupancyPercent=15";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameWindow {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
}

impl Default for GameWindow {
    /// Minecraft's own defaults.
    fn default() -> Self {
        Self { width: 854, height: 480, fullscreen: false }
    }
}

/// Commands around the game. Empty means none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchHooks {
    /// Starts the game through another program, like `mangohud`.
    pub wrapper: String,
    pub pre_launch: String,
    pub post_exit: String,
    /// `NAME=value` lines.
    pub env: String,
}

/// Which kinds of version a mod can update to. A stricter choice can stop updates for a mod that
/// only publishes betas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateChannel {
    #[default]
    All,
    Beta,
    Release,
}

impl UpdateChannel {
    pub fn version_types(self) -> Option<&'static [&'static str]> {
        match self {
            UpdateChannel::All => None,
            UpdateChannel::Beta => Some(&["release", "beta"]),
            UpdateChannel::Release => Some(&["release"]),
        }
    }
}

/// Options that only exist on Linux.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxOptions {
    pub gamemode: bool,
    pub mangohud: bool,
    pub discrete_gpu: bool,
    pub system_glfw: bool,
    pub system_openal: bool,
}

impl LinuxOptions {
    /// Programs that start the game, outermost first.
    pub fn wrappers(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.gamemode {
            out.push("gamemoderun");
        }
        if self.mangohud {
            out.push("mangohud");
        }
        out
    }

    pub fn env(self) -> Vec<(String, String)> {
        let vars = [("DRI_PRIME", "1"), ("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")];
        vars.iter().filter(|_| self.discrete_gpu).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    pub fn jvm_args(self) -> Vec<String> {
        let mut out = Vec::new();
        if self.system_glfw {
            out.push("-Dorg.lwjgl.glfw.libname=libglfw.so.3".to_string());
        }
        if self.system_openal {
            out.push("-Dorg.lwjgl.openal.libname=libopenal.so.1".to_string());
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory_mb: u32,
    pub min_memory_mb: u32,
    pub window: GameWindow,
    pub hooks: LaunchHooks,
    pub linux: LinuxOptions,
    pub low_memory_warning: bool,
    /// Off by default, because it shows on the player's public profile.
    pub discord_presence: bool,
    pub jvm_args: String,
    /// Global Java override; `None` picks or downloads the right Java automatically.
    pub java_path: Option<String>,
    /// Use a chosen Java even when it doesn't suit the Minecraft version.
    pub ignore_java_check: bool,
    pub on_launch: OnLaunch,
    pub parallel_downloads: usize,
    pub update_channel: UpdateChannel,
    pub open_console_on_launch: bool,
    pub sidebar_collapsed: bool,
    pub check_updates: bool,
    /// `http://host:port`, for the launcher's own downloads. Empty means none.
    pub proxy: String,
    /// `None` keeps instances in the data folder.
    pub instances_dir: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory_mb: crate::sysmem::read().map_or(3072, |ram| crate::sysmem::default_memory_mb(ram.total_mb)),
            min_memory_mb: 512,
            window: GameWindow::default(),
            hooks: LaunchHooks::default(),
            linux: LinuxOptions::default(),
            low_memory_warning: true,
            discord_presence: false,
            jvm_args: LOW_END_JVM_ARGS.to_string(),
            java_path: None,
            ignore_java_check: false,
            on_launch: OnLaunch::Minimize,
            parallel_downloads: 8,
            update_channel: UpdateChannel::All,
            open_console_on_launch: false,
            sidebar_collapsed: false,
            check_updates: true,
            proxy: String::new(),
            instances_dir: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        paths::load_json(&paths::root().join("config.json"))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::write_json(&paths::root().join("config.json"), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_options_turn_into_launch_settings() {
        assert!(LinuxOptions::default().wrappers().is_empty());
        assert!(LinuxOptions::default().env().is_empty());
        assert!(LinuxOptions::default().jvm_args().is_empty());
        let on = LinuxOptions { gamemode: true, mangohud: true, discrete_gpu: true, system_glfw: true, system_openal: false };
        assert_eq!(on.wrappers(), ["gamemoderun", "mangohud"]);
        assert!(on.env().iter().any(|(k, v)| k == "DRI_PRIME" && v == "1"));
        assert_eq!(on.jvm_args(), ["-Dorg.lwjgl.glfw.libname=libglfw.so.3"]);
    }
}
