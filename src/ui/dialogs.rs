// SPDX-License-Identifier: GPL-3.0-only
//! Yes-or-no questions before something can't be undone.

use eframe::egui::{self, Color32, RichText};

use super::widgets::{self, Kind};
use super::{App, Page, theme};
use crate::minecraft::instance::Instance;
use crate::modplatform::modrinth::{self, ContentKind, LocalFile};

pub enum Confirm {
    DeleteInstance(String),
    DeleteFile { inst: Box<Instance>, kind: ContentKind, file: Box<LocalFile> },
    RemoveAccount(String),
    Discard(Page),
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
