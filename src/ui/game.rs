// SPDX-License-Identifier: GPL-3.0-only
//! Starting and stopping the game for an instance.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use tokio::sync::oneshot;

use super::dialogs::Confirm;
use super::{App, Bus, Event, Page, console, editor};
use crate::config::OnLaunch;
use crate::discord::Activity;
use crate::minecraft::auth::{self, AccountKind};
use crate::minecraft::instance::Loader;
use crate::minecraft::launch::{self, Level, LineSink, LogLine};
use crate::sysmem;

/// An instance's reserved "running" slot. Dropping it before the game starts frees the slot
/// again; the error, if any, is reported by the task itself.
struct LaunchSlot {
    bus: Bus,
    inst: Option<String>,
}

impl LaunchSlot {
    fn disarm(&mut self) {
        self.inst = None;
    }
}

impl Drop for LaunchSlot {
    fn drop(&mut self) {
        if let Some(inst) = self.inst.take() {
            self.bus.send(Event::GameExited { inst, code: Some(0) });
        }
    }
}

pub(super) struct Running {
    pub(super) kill: Option<oneshot::Sender<()>>,
    pub(super) started: Instant,
    pub(super) killed: bool,
}

impl App {
    pub fn launch(&mut self, id: &str, offline: bool) {
        self.start(id, offline, self.cfg.low_memory_warning);
    }

    pub fn launch_despite_low_memory(&mut self, id: &str, offline: bool) {
        self.start(id, offline, false);
    }

    fn start(&mut self, id: &str, offline: bool, check_memory: bool) {
        if self.running.contains_key(id) {
            return;
        }
        let Some(mut inst) = self.instance(id).cloned() else { return };
        let chosen = inst.account_id.as_ref().and_then(|id| self.accounts.accounts.iter().find(|a| a.id == *id));
        let Some(account) = chosen.or(self.accounts.selected()).cloned() else {
            self.warn("Sign in with a Microsoft account before launching.");
            self.go(Page::Accounts);
            return;
        };
        if (offline || account.kind == AccountKind::Offline) && !self.accounts.owns_game() {
            self.warn("Offline play requires signing in with a Microsoft account that owns Minecraft: Java Edition.");
            self.go(Page::Accounts);
            return;
        }
        let cfg = self.cfg.clone();
        let memory_mb = inst.memory_mb.unwrap_or(cfg.memory_mb);
        if check_memory
            && let Some(ram) = sysmem::read()
            && sysmem::is_low(memory_mb, ram.available_mb)
        {
            self.confirm = Some(Confirm::LowMemory { id: id.to_string(), offline, memory_mb, free_mb: ram.available_mb });
            return;
        }
        if let Some((server, world)) = self.join_once.take() {
            inst.join_server = server.or(inst.join_server);
            inst.join_world = world;
        }
        self.consoles.insert(id.to_string(), console::Console::default());
        if cfg.open_console_on_launch {
            self.open_editor(id, editor::Tab::Log);
        }
        // Reserve the slot so a double click cannot start two copies.
        self.running.insert(id.to_string(), Running { kill: None, started: Instant::now(), killed: false });
        // Moved into the task, so it is dropped (freeing the slot) on failure or cancel alike.
        let mut slot = LaunchSlot { bus: self.bus.clone(), inst: Some(id.to_string()) };
        self.cancellable_task(format!("Launching {}", inst.name), move |progress, bus| async move {
            let account = if offline || account.kind == AccountKind::Offline {
                account
            } else {
                let a = auth::ensure_token(account).await?;
                bus.send(Event::Account(a.clone()));
                a
            };
            let prepared = launch::prepare(&inst, &account, offline, &cfg, &progress).await?;
            for note in &prepared.notes {
                bus.send(Event::Warning(note.clone()));
                bus.send(Event::Log { inst: inst.id.clone(), line: LogLine { level: Level::Warn, text: note.clone() } });
            }
            let (log_bus, log_id) = (bus.clone(), inst.id.clone());
            let sink: LineSink = Arc::new(move |line| log_bus.send_batched(Event::Log { inst: log_id.clone(), line }));
            let (exit_bus, exit_id) = (bus.clone(), inst.id.clone());
            let kill = launch::spawn(prepared, sink, move |code| exit_bus.send(Event::GameExited { inst: exit_id, code }))?;
            bus.send(Event::GameStarted { inst: inst.id.clone(), kill });
            slot.disarm();
            Ok(())
        });
    }

    /// Shows `inst` on the player's Discord profile, if they turned that on. The instance's own
    /// name stays private: it can be anything, and only the game and pack are public.
    pub(super) fn show_on_discord(&mut self, inst: &str) {
        if !self.cfg.discord_presence {
            return;
        }
        let Some(inst) = self.instance(inst) else { return };
        let details = match inst.loader {
            Loader::Vanilla => format!("Minecraft {}", inst.mc_version),
            loader => format!("Minecraft {} with {}", inst.mc_version, loader.name()),
        };
        let state = inst.pack.as_ref().map(|p| p.title.clone());
        let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.discord.set(Some(Activity { details, state, started }));
    }

    pub fn kill(&mut self, id: &str) {
        if let Some(r) = self.running.get_mut(id) {
            r.killed = true;
            if let Some(tx) = r.kill.take() {
                let _ = tx.send(());
            }
        }
    }

    pub(super) fn on_game_exit(&mut self, ctx: &egui::Context, inst: String, code: Option<i32>) {
        let Some(run) = self.running.remove(&inst) else { return };
        if self.running.is_empty() {
            self.discord.set(None);
        }
        if run.kill.is_none() {
            return; // launch failed before the game started
        }
        if let Some(i) = self.instances.iter_mut().find(|i| i.id == inst)
            && let Err(e) = i.record_play(run.started.elapsed().as_secs())
        {
            self.errors.push(format!("Saving the play time of {} failed: {e:#}", i.name));
        }
        // A game session may have added worlds or changed files.
        if let Some(e) = &mut self.editor {
            e.invalidate();
        }
        let crashed = !run.killed && code != Some(0);
        let text = match code {
            Some(c) => format!("Game exited with code {c}"),
            None => "Game was stopped".to_string(),
        };
        self.consoles.entry(inst.clone()).or_default().push(LogLine { level: if crashed { Level::Error } else { Level::Info }, text });
        if crashed {
            self.open_editor(&inst, editor::Tab::Log);
        }
        if self.cfg.on_launch == OnLaunch::Minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }
}
