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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory_mb: u32,
    pub jvm_args: String,
    /// Global Java override; `None` picks or downloads the right Java automatically.
    pub java_path: Option<String>,
    pub on_launch: OnLaunch,
    pub parallel_downloads: usize,
    pub open_console_on_launch: bool,
    pub sidebar_collapsed: bool,
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory_mb: 3072,
            jvm_args: LOW_END_JVM_ARGS.to_string(),
            java_path: None,
            on_launch: OnLaunch::Minimize,
            parallel_downloads: 8,
            open_console_on_launch: false,
            sidebar_collapsed: false,
            check_updates: true,
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
