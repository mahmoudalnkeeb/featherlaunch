// SPDX-License-Identifier: GPL-3.0-only
//! Global launcher settings.

use eframe::egui::{self, RichText};

use crate::config::{Config, LOW_END_JVM_ARGS, OnLaunch};
use crate::paths;
use crate::ui::glyph::Glyph;
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, theme};

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let mut draft = app.settings_draft.take().unwrap_or_else(|| app.cfg.clone());
    // The sidebar toggle saves on its own; don't undo a change made after the draft.
    draft.sidebar_collapsed = app.cfg.sidebar_collapsed;
    if draft != app.cfg {
        save_bar(app, ui, &mut draft);
    }
    widgets::page(ui, "Settings", "Defaults for every instance. An instance can override them on its own Settings tab.", |ui| {
        widgets::form(ui, |ui| {
            widgets::form_section(ui, "Game", true);
            widgets::form_row(ui, "Default memory", |ui| {
                widgets::slider(ui, &mut draft.memory_mb, 512..=16384, 256.0, " MB");
                widgets::hint(ui, "3–4 GB suits most modpacks on a 16 GB PC.");
            });
            widgets::form_row(ui, "When the game starts", |ui| {
                let mut choice = draft.on_launch;
                let items = [(OnLaunch::Minimize, "Minimize"), (OnLaunch::Close, "Close"), (OnLaunch::KeepOpen, "Keep open")];
                widgets::segmented(ui, &mut choice, &items);
                draft.on_launch = choice;
            });
            widgets::form_row(ui, "Game log", |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::H_MD);
                    widgets::toggle_labeled(ui, &mut draft.open_console_on_launch, "Show the log when the game starts");
                });
            });

            widgets::form_section(ui, "Java", false);
            widgets::form_row(ui, "Java runtime", |ui| {
                ui.horizontal(|ui| {
                    let text = draft.java_path.clone().unwrap_or_else(|| "Automatic (recommended)".into());
                    let width = (ui.available_width() - 2.0 * (theme::H_MD + theme::S2)).min(320.0);
                    widgets::combo_cut(ui, "java-global", text, width, |ui| {
                        ui.selectable_value(&mut draft.java_path, None, "Automatic (recommended)");
                        for j in &app.java_list {
                            let path = j.path.to_string_lossy().into_owned();
                            ui.selectable_value(&mut draft.java_path, Some(path.clone()), format!("Java {} — {path}", j.version));
                        }
                    });
                    let browse = if cfg!(windows) { "Browse for java.exe" } else { "Browse for java" };
                    if widgets::icon_button(ui, Glyph::Folder, browse).clicked() {
                        // The Linux binary has no extension, so a filter would hide it.
                        let dialog = rfd::FileDialog::new();
                        let dialog = if cfg!(windows) { dialog.add_filter("java.exe", &["exe"]) } else { dialog };
                        if let Some(p) = dialog.pick_file() {
                            draft.java_path = Some(p.to_string_lossy().into_owned());
                        }
                    }
                    if widgets::icon_button(ui, Glyph::Refresh, "Rescan installed Java").clicked() {
                        app.refresh_java();
                    }
                });
            });
            widgets::form_row(ui, "JVM arguments", |ui| {
                widgets::multiline(ui, &mut draft.jvm_args);
                if widgets::pill_sized(ui, Kind::Secondary, None, "Reset to low-end defaults", theme::H_SM).clicked() {
                    draft.jvm_args = LOW_END_JVM_ARGS.to_string();
                }
            });

            widgets::form_section(ui, "Downloads", false);
            widgets::form_row(ui, "Parallel downloads", |ui| {
                widgets::slider(ui, &mut draft.parallel_downloads, 1..=16, 1.0, "");
            });

            widgets::form_section(ui, "Storage", false);
            widgets::form_row(ui, "Data folder", |ui| {
                let path = paths::root().to_string_lossy().into_owned();
                widgets::line_tip(ui, &path, theme::label(), theme::TEXT, theme::H_MD);
                ui.horizontal(|ui| {
                    if widgets::pill_sized(ui, Kind::Secondary, None, "Open", theme::H_SM).clicked() {
                        app.open(paths::root());
                    }
                    if widgets::pill_sized(ui, Kind::Ghost, None, "Instances", theme::H_SM).clicked() {
                        app.open(paths::instances());
                    }
                    if widgets::pill_sized(ui, Kind::Ghost, None, "Java runtimes", theme::H_SM).clicked() {
                        app.open(paths::java());
                    }
                });
            });

            widgets::form_section(ui, "Updates", false);
            widgets::form_row(ui, "Version", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(env!("CARGO_PKG_VERSION")).color(theme::TEXT));
                    ui.add_space(theme::S2);
                    if app.launcher_update.can_check()
                        && widgets::pill_sized(ui, Kind::Secondary, None, "Check for updates", theme::H_SM).clicked()
                    {
                        app.check_launcher_update(true);
                    }
                    if let Some(page) = app.launcher_update.release_page().map(str::to_string)
                        && widgets::pill_sized(ui, Kind::Ghost, None, "Open release page", theme::H_SM).clicked()
                    {
                        app.open(page);
                    }
                });
            });
            widgets::form_row(ui, "Automatic check", |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::H_MD);
                    widgets::toggle_labeled(ui, &mut draft.check_updates, "Look for a new version when the launcher starts");
                });
            });
        });
    });
    app.settings_draft = Some(draft);
}

fn save_bar(app: &mut App, ui: &mut egui::Ui, draft: &mut Config) {
    widgets::save_bar(ui, "settings-save", "Unsaved changes", |ui| {
        if widgets::pill(ui, Kind::Primary, None, "Save").clicked() {
            match draft.save() {
                Ok(()) => {
                    app.cfg = draft.clone();
                    app.notify("Settings saved");
                }
                Err(e) => app.errors.push(format!("Saving settings failed: {e:#}")),
            }
        }
        if widgets::pill(ui, Kind::Secondary, None, "Discard").clicked() {
            *draft = app.cfg.clone();
        }
    });
}
