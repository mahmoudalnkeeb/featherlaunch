// SPDX-License-Identifier: GPL-3.0-only
//! Starting and stopping the game for an instance.

use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use tokio::sync::oneshot;

use super::{App, Bus, Event, Page, console, editor};
use crate::config::OnLaunch;
use crate::minecraft::auth::{self, AccountKind};
use crate::minecraft::launch::{self, Level, LineSink, LogLine};

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
        if self.running.contains_key(id) {
            return;
        }
        let Some(inst) = self.instance(id).cloned() else { return };
        let Some(account) = self.accounts.selected().cloned() else {
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
            let (log_bus, log_id) = (bus.clone(), inst.id.clone());
            let sink: LineSink = Arc::new(move |line| log_bus.send_batched(Event::Log { inst: log_id.clone(), line }));
            let (exit_bus, exit_id) = (bus.clone(), inst.id.clone());
            let kill = launch::spawn(prepared, sink, move |code| exit_bus.send(Event::GameExited { inst: exit_id, code }))?;
            bus.send(Event::GameStarted { inst: inst.id.clone(), kill });
            slot.disarm();
            Ok(())
        });
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
