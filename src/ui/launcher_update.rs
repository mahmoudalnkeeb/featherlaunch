// SPDX-License-Identifier: GPL-3.0-only
//! Updating the launcher itself, from the sidebar row.

use std::path::PathBuf;

use eframe::egui;

use super::{App, Bus, Event};
use crate::updater::{self, Release};

#[derive(Default)]
pub enum LauncherUpdate {
    #[default]
    None,
    Available(Release),
    Installing(Release),
    Ready(PathBuf),
}

impl LauncherUpdate {
    pub fn row_label(&self) -> Option<String> {
        match self {
            Self::Available(release) => Some(format!("Update to {}", release.version)),
            Self::Ready(_) => Some("Restart to update".to_string()),
            Self::None | Self::Installing(_) => None,
        }
    }

    pub fn release_page(&self) -> Option<&str> {
        match self {
            Self::Available(release) | Self::Installing(release) => Some(&release.page),
            Self::None | Self::Ready(_) => None,
        }
    }

    /// A new check would only find what is already being installed.
    pub fn can_check(&self) -> bool {
        matches!(self, Self::None | Self::Available(_))
    }
}

/// A launcher update in progress. Dropping it before the install finished offers the update
/// again; the error, if any, is reported by the task itself.
struct UpdateSlot {
    bus: Bus,
    armed: bool,
}

impl UpdateSlot {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UpdateSlot {
    fn drop(&mut self) {
        if self.armed {
            self.bus.send(Event::LauncherUpdateStopped);
        }
    }
}

impl App {
    /// Looks for a new launcher version. Only a `manual` check reports that there is none or that
    /// it failed: being offline at startup isn't worth a message, and the next start tries again.
    pub(super) fn check_launcher_update(&self, manual: bool) {
        let bus = self.bus.clone();
        self.rt.spawn(async move {
            match updater::check().await {
                Ok(Some(release)) => bus.send(Event::LauncherUpdateFound(release)),
                Ok(None) if manual => bus.send(Event::Info("FeatherLaunch is up to date".to_string())),
                Err(e) if manual => {
                    let message = format!("Checking for updates failed: {e:#}");
                    crate::log::error(&message);
                    bus.send(Event::Warning(message));
                }
                _ => {}
            }
        });
    }

    pub(super) fn launcher_update_clicked(&mut self, ctx: &egui::Context) {
        match &self.launcher_update {
            LauncherUpdate::Available(release) => self.install_launcher_update(release.clone()),
            LauncherUpdate::Ready(exe) => {
                let exe = exe.clone();
                self.restart_into_update(ctx, &exe);
            }
            LauncherUpdate::None | LauncherUpdate::Installing(_) => {}
        }
    }

    pub(super) fn install_launcher_update(&mut self, release: Release) {
        self.launcher_update = LauncherUpdate::Installing(release.clone());
        // Moved into the task, so it is dropped (offering the update again) on failure or cancel.
        let mut slot = UpdateSlot { bus: self.bus.clone(), armed: true };
        self.cancellable_task(format!("Updating FeatherLaunch to {}", release.version), move |progress, bus| async move {
            let exe = updater::install(&release, &progress).await?;
            slot.disarm();
            bus.send(Event::LauncherUpdateInstalled(exe));
            Ok(())
        });
    }

    /// Closing would orphan a running game's console and play time, and cut off running tasks, so
    /// both are finished first. The new version also starts by itself on the next launch.
    pub(super) fn restart_into_update(&mut self, ctx: &egui::Context, exe: &std::path::Path) {
        if !self.running.is_empty() {
            self.notify("Close the game first, then restart to update");
        } else if self.tasks.iter().any(|t| !t.snapshot().finished()) {
            self.notify("Wait for running tasks to finish, then restart to update");
        } else {
            match updater::restart(exe) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => self.errors.push(format!("Restarting failed: {e}. The new version starts the next time you open FeatherLaunch.")),
            }
        }
    }
}
