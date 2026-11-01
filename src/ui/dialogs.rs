// SPDX-License-Identifier: GPL-3.0-only
//! Yes-or-no questions before something can't be undone.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};

use super::widgets::{self, Kind};
use super::{App, Event, Page, theme};
use crate::minecraft::instance::{Instance, World};
use crate::modplatform::modrinth::{self, ContentKind, LocalFile};

pub enum Confirm {
    DeleteInstance(String),
    DeleteFile { inst: Box<Instance>, kind: ContentKind, file: Box<LocalFile> },
    RemoveAccount(String),
    Discard(Page),
    LowMemory { id: String, offline: bool, memory_mb: u32, free_mb: u64 },
    UncheckedPack { path: PathBuf, group: String, update: Option<String>, files: Vec<String> },
    DeleteWorld(Box<World>),
    UploadLog { text: String },
}

/// How many paths a dialog names before summing up the rest.
const LISTED_FILES: usize = 8;

fn file_list(files: &[String]) -> String {
    let mut lines: Vec<String> = files.iter().take(LISTED_FILES).cloned().collect();
    if files.len() > LISTED_FILES {
        lines.push(format!("and {} more", files.len() - LISTED_FILES));
    }
    lines.join("\n")
}

struct Prompt {
    title: &'static str,
    body: String,
    note: Option<(String, Color32)>,
    cancel: &'static str,
    confirm: &'static str,
    enabled: bool,
}

fn prompt(app: &App, confirm: &Confirm) -> Option<Prompt> {
    let cannot_undo = Some(("This cannot be undone.".to_string(), theme::ERR));
    Some(match confirm {
        Confirm::DeleteInstance(id) => {
            let inst = app.instance(id)?;
            let running = app.running.contains_key(id);
            Prompt {
                title: "Delete instance",
                body: format!("\"{}\" and all of its worlds, mods and settings will be deleted.", inst.name),
                note: if running { Some(("Stop the game before deleting this instance.".to_string(), theme::WARN)) } else { cannot_undo },
                cancel: "Cancel",
                confirm: "Delete",
                enabled: !running,
            }
        }
        Confirm::DeleteFile { file, .. } => Prompt {
            title: "Delete file",
            body: format!("\"{}\" will be deleted from this instance.", file.title.as_deref().unwrap_or(&file.file_name)),
            note: cannot_undo,
            cancel: "Cancel",
            confirm: "Delete",
            enabled: true,
        },
        Confirm::RemoveAccount(id) => {
            let account = app.accounts.accounts.iter().find(|a| a.id == *id)?;
            Prompt {
                title: "Remove account",
                body: format!("{} will be removed from FeatherLaunch. You can sign in again at any time.", account.name),
                note: None,
                cancel: "Cancel",
                confirm: "Remove",
                enabled: true,
            }
        }
        Confirm::Discard(_) => Prompt {
            title: "Unsaved changes",
            body: format!("Your changes to {} haven't been saved.", app.editor.as_ref()?.name()),
            note: None,
            cancel: "Keep editing",
            confirm: "Discard",
            enabled: true,
        },
        Confirm::LowMemory { memory_mb, free_mb, .. } => Prompt {
            title: "Low free memory",
            body: format!("The game can use up to {memory_mb} MB, but only {free_mb} MB is free."),
            note: Some(("Launching may slow down your PC. Closing other apps frees memory.".to_string(), theme::WARN)),
            cancel: "Cancel",
            confirm: "Launch anyway",
            enabled: true,
        },
        Confirm::UncheckedPack { files, .. } => Prompt {
            title: "Unchecked mods in this pack",
            body: format!("This pack adds files Modrinth hasn't reviewed:\n\n{}", file_list(files)),
            note: Some(("Mods can run any code on your PC. Only continue if you trust this pack.".to_string(), theme::WARN)),
            cancel: "Cancel",
            confirm: "Install anyway",
            enabled: true,
        },
        Confirm::DeleteWorld(world) => Prompt {
            title: "Delete world",
            body: format!("\"{}\" will be deleted, along with everything built in it.", world.name),
            note: cannot_undo,
            cancel: "Cancel",
            confirm: "Delete",
            enabled: true,
        },
        Confirm::UploadLog { text } => Prompt {
            title: "Upload log",
            body: format!("{} lines will be sent to mclo.gs, and anyone with the link can read them.", text.lines().count()),
            note: Some((
                "Access tokens and your home folder are hidden first. Check the log for anything else private.".to_string(),
                theme::WARN,
            )),
            cancel: "Cancel",
            confirm: "Upload",
            enabled: true,
        },
    })
}

fn apply(app: &mut App, confirm: Confirm) {
    match confirm {
        Confirm::DeleteInstance(id) => {
            if let Some(inst) = app.instance(&id).cloned() {
                app.delete_instance(inst);
            }
        }
        Confirm::DeleteFile { inst, kind, file } => {
            if let Err(e) = modrinth::delete_local(&inst, kind, &file) {
                app.errors.push(format!("Could not delete {}: {e:#}", file.file_name));
            }
            if let Some(editor) = &mut app.editor {
                editor.invalidate();
            }
        }
        Confirm::RemoveAccount(id) => {
            app.accounts.remove(&id);
            app.skins.remove(&id);
            app.save_accounts();
        }
        Confirm::Discard(page) => app.go_now(page),
        Confirm::LowMemory { id, offline, .. } => app.launch_despite_low_memory(&id, offline),
        Confirm::UncheckedPack { path, group, update, .. } => app.import_pack(path, group, update),
        Confirm::UploadLog { text } => app.upload_log(text),
        Confirm::DeleteWorld(world) => {
            app.task(format!("Deleting {}", world.name), move |_, bus| async move {
                tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&world.path)).await??;
                bus.send(Event::ContentChanged);
                Ok(())
            });
        }
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(confirm) = &app.confirm else { return };
    let Some(prompt) = prompt(app, confirm) else {
        app.confirm = None;
        return;
    };
    let mut yes = false;
    let mut no = false;
    let (_, dismissed) = widgets::dialog(ctx, "confirm", prompt.title, 420.0, |ui| {
        ui.label(RichText::new(&prompt.body).color(theme::TEXT));
        if let Some((text, color)) = &prompt.note {
            ui.label(RichText::new(text).color(*color));
        }
        widgets::dialog_buttons(ui, |ui| {
            yes = ui.add_enabled_ui(prompt.enabled, |ui| widgets::pill(ui, Kind::Danger, None, prompt.confirm)).inner.clicked();
            no = widgets::pill(ui, Kind::Secondary, None, prompt.cancel).clicked();
        });
    });
    if yes && let Some(confirm) = app.confirm.take() {
        apply(app, confirm);
    } else if no || dismissed {
        app.confirm = None;
    }
}
