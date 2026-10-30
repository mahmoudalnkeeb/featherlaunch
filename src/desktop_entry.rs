// SPDX-License-Identifier: GPL-3.0-only
//! Linux menu entry and icon.
//!
//! Wayland compositors ignore icons a window sets for itself; they look up the `.desktop` file
//! whose name matches the window's app ID (`featherlaunch`). Without one, docks and task
//! switchers show a generic icon. So on startup the launcher writes its own entry and icon into
//! the user's data folder, pointing at wherever the binary currently lives.

use std::path::Path;

use anyhow::{Context, Result};

const ICON: &[u8] = include_bytes!("../assets/icon-256.png");

/// Writes the menu entry and icon when missing or out of date (for example after the binary moved).
pub fn install() -> Result<()> {
    let base = directories::BaseDirs::new().context("no home folder")?;
    let data = base.data_dir();
    let exe = std::env::current_exe().context("finding the launcher's own path")?;

    write_if_changed(&data.join("icons/hicolor/256x256/apps/featherlaunch.png"), ICON)?;
    // Desktop entries need the Exec path quoted when it has spaces.
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=FeatherLaunch\n\
         GenericName=Minecraft Launcher\n\
         Comment=Lightweight Minecraft: Java Edition launcher\n\
         Exec=\"{}\"\n\
         Icon=featherlaunch\n\
         Terminal=false\n\
         Categories=Game;\n\
         Keywords=minecraft;launcher;mods;modrinth;\n\
         StartupWMClass=featherlaunch\n\
         StartupNotify=true\n",
        exe.display()
    );
    write_if_changed(&data.join("applications/featherlaunch.desktop"), entry.as_bytes())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}
