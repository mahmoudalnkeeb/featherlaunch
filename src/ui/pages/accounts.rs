// SPDX-License-Identifier: GPL-3.0-only
//! Accounts page, Microsoft device-code sign-in and skin changes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context as _;
use eframe::egui::{self, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, vec2};

use crate::minecraft::auth::{self, Account, AccountKind, Cape};
use crate::minecraft::logs;
use crate::paths;
use crate::ui::dialogs::Confirm;
use crate::ui::glyph::{self, Glyph};
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, Event, theme};

#[derive(Default)]
pub struct AccountsUi {
    slim: bool,
    /// The account whose capes were last asked for, and what came back.
    capes_asked: Option<String>,
    capes: Option<(String, Vec<Cape>)>,
}

impl AccountsUi {
    pub fn on_capes(&mut self, account: String, capes: Vec<Cape>) {
        self.capes = Some((account, capes));
    }
}

const SKIN_WIDTH: f32 = 200.0;
const SKIN_BODY: egui::Vec2 = egui::vec2(108.0, 204.0);

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let mut select = None;
    let mut login = false;
    widgets::page(ui, "Accounts", "Sign in with the Microsoft account that owns Minecraft.", |ui| {
        if app.accounts.accounts.is_empty() {
            widgets::empty_state(ui, "No accounts yet", "Sign in with Microsoft to play and to change your skin.", |ui| {
                login = widgets::pill(ui, Kind::Primary, None, "Sign in with Microsoft").clicked();
            });
            return;
        }
        ui.horizontal_top(|ui| {
            let left = ui.available_width() - SKIN_WIDTH - theme::S6;
            ui.vertical(|ui| {
                ui.set_width(left);
                ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
                for a in &app.accounts.accounts {
                    let selected = app.accounts.selected.as_deref() == Some(a.id.as_str());
                    let (row, r) = widgets::list_row(ui, 52.0, Sense::click(), selected);
                    let head = Rect::from_min_size(row.min + vec2(10.0, 10.0), vec2(32.0, 32.0));
                    match app.skins.get(&a.id) {
                        Some(tex) => {
                            egui::Image::new(egui::load::SizedTexture::new(tex.head.id(), head.size()))
                                .corner_radius(CornerRadius::same(theme::R_SM))
                                .paint_at(ui, head);
                        }
                        None => {
                            ui.painter().rect_filled(head, CornerRadius::same(theme::R_SM), theme::RAISED);
                            glyph::paint(ui.painter(), glyph::centered(head, 16.0), Glyph::User, theme::TEXT_MUTED);
                        }
                    }
                    let remove_rect = Rect::from_center_size(Pos2::new(row.right() - 26.0, row.center().y), vec2(theme::H_MD, theme::H_MD));
                    let x = head.right() + theme::S3;
                    let active_width = if selected { 60.0 } else { 0.0 };
                    let text_w = remove_rect.left() - active_width - theme::S2 - x;
                    widgets::line(ui, Pos2::new(x, row.top() + 8.0), &a.name, theme::body_strong(), theme::TEXT, text_w);
                    let kind = match a.kind {
                        AccountKind::Microsoft if a.has_valid_token() => "Microsoft  ·  signed in",
                        AccountKind::Microsoft => "Microsoft",
                        AccountKind::Offline => "Offline",
                    };
                    widgets::line(ui, Pos2::new(x, row.top() + 27.0), kind, theme::label(), theme::TEXT_MUTED, text_w);
                    if selected {
                        widgets::badge(ui, Pos2::new(remove_rect.left() - active_width, row.center().y - 9.0), "Active", theme::ACCENT);
                    }
                    let mut button = ui.new_child(UiBuilder::new().max_rect(remove_rect));
                    if widgets::icon_button(&mut button, Glyph::Trash, "Remove").clicked() {
                        app.confirm = Some(Confirm::RemoveAccount(a.id.clone()));
                    } else if r.clicked() {
                        select = Some(a.id.clone());
                    }
                }
                ui.add_space(theme::S3);
                login = widgets::pill(ui, Kind::Secondary, None, "Add another account").clicked();
            });
            ui.add_space(theme::S6);
            ui.vertical(|ui| {
                ui.set_width(SKIN_WIDTH);
                skin(app, ui);
            });
        });
    });
    if login {
        start_login(app);
    }
    if let Some(id) = select {
        app.accounts.selected = Some(id);
        app.save_accounts();
    }
}

fn skin(app: &mut App, ui: &mut egui::Ui) {
    let (well, _) = ui.allocate_exact_size(vec2(SKIN_WIDTH, SKIN_BODY.y + theme::S6 * 2.0), Sense::hover());
    let radius = CornerRadius::same(theme::R_LG);
    ui.painter().rect_filled(well, radius, theme::SURFACE);
    ui.painter().rect_stroke(well, radius, Stroke::new(1.0_f32, theme::BORDER), StrokeKind::Inside);
    match app.accounts.selected.as_ref().and_then(|id| app.skins.get(id)) {
        Some(tex) => {
            let body = Rect::from_center_size(well.center(), SKIN_BODY);
            egui::Image::new(egui::load::SizedTexture::new(tex.front.id(), body.size())).paint_at(ui, body);
        }
        None => glyph::paint(ui.painter(), glyph::centered(well, 48.0), Glyph::User, theme::TEXT_FAINT),
    }
    if let Some(account) = app.accounts.selected().cloned().filter(|a| a.kind == AccountKind::Microsoft) {
        ui.add_space(theme::S2);
        widgets::toggle_labeled(ui, &mut app.accounts_ui.slim, "Slim arms");
        ui.add_space(theme::S1);
        if widgets::pill(ui, Kind::Secondary, None, "Change skin…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Skin", &["png"]).pick_file()
        {
            change_skin(app, account.clone(), path, app.accounts_ui.slim);
        }
        saved_skins(app, ui, &account);
        capes(app, ui, &account);
    }
}

fn saved_skins(app: &mut App, ui: &mut egui::Ui, account: &Account) {
    ui.add_space(theme::S1);
    let mut chosen = None;
    widgets::combo("saved-skins", "Saved skins", SKIN_WIDTH - widgets::INPUT_MARGIN).show_ui(ui, |ui| {
        let saved = logs::files_in(&paths::skins(), |n| n.ends_with(".png"));
        if saved.is_empty() {
            ui.label(RichText::new("None yet").color(theme::TEXT_MUTED));
        }
        for skin in saved {
            if ui.selectable_label(false, skin.name.trim_end_matches(".png")).clicked() {
                chosen = Some(skin.path);
            }
        }
    });
    if let Some(path) = chosen {
        change_skin(app, account.clone(), path, app.accounts_ui.slim);
    }
    ui.add_space(theme::S1);
    if widgets::pill(ui, Kind::Ghost, None, "Add to saved…").clicked()
        && let Some(path) = rfd::FileDialog::new().add_filter("Skin", &["png"]).pick_file()
        && let Some(name) = path.file_name()
    {
        let saved = std::fs::create_dir_all(paths::skins()).and_then(|()| std::fs::copy(&path, paths::skins().join(name)));
        if let Err(e) = saved {
            app.errors.push(format!("Could not save the skin: {e}"));
        }
    }
}

fn capes(app: &mut App, ui: &mut egui::Ui, account: &Account) {
    if app.accounts_ui.capes_asked.as_deref() != Some(&account.id) {
        app.accounts_ui.capes_asked = Some(account.id.clone());
        let (account, bus) = (account.clone(), app.bus.clone());
        // Quietly, because this page opens fine offline; the menu just stays hidden.
        app.rt.spawn(async move {
            match auth::capes(account).await {
                Ok((account, capes)) => bus.send(Event::Capes { account: account.id, capes }),
                Err(e) => crate::log::error(&format!("Loading capes failed: {e:#}")),
            }
        });
    }
    let Some((_, capes)) = app.accounts_ui.capes.clone().filter(|(id, list)| *id == account.id && !list.is_empty()) else { return };
    ui.add_space(theme::S1);
    let current = capes.iter().find(|c| c.state == "ACTIVE");
    let mut picked = None;
    widgets::combo("cape", current.map_or("No cape", |c| c.alias.as_str()), SKIN_WIDTH - widgets::INPUT_MARGIN).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), "No cape").clicked() {
            picked = Some(None);
        }
        for cape in &capes {
            if ui.selectable_label(cape.state == "ACTIVE", &cape.alias).clicked() {
                picked = Some(Some(cape.id.clone()));
            }
        }
    });
    if let Some(cape) = picked {
        let account = account.clone();
        app.task("Changing cape", move |_, bus| async move {
            let account = auth::ensure_token(account).await?;
            auth::set_cape(&account, cape.as_deref()).await?;
            let (account, capes) = auth::capes(account).await?;
            bus.send(Event::Capes { account: account.id, capes });
            Ok(())
        });
    }
}

fn start_login(app: &mut App) {
    // Closing the code dialog sets this flag, which stops the background polling.
    let cancel = Arc::new(AtomicBool::new(false));
    app.login_cancel = Some(cancel.clone());
    app.task("Signing in with Microsoft", move |_, bus| async move {
        let code = auth::start_device_login().await?;
        bus.send(Event::DeviceCode(code.clone()));
        let result = auth::finish_device_login(&code, &cancel).await;
        // A cancelled attempt already cleared its dialog; don't clear a newer one.
        if !cancel.load(Ordering::Relaxed) {
            bus.send(Event::LoginFinished);
        }
        if let Some(account) = result? {
            bus.send(Event::Info(format!("Signed in as {}", account.name)));
            bus.send(Event::Account(account));
        }
        Ok(())
    });
}

fn change_skin(app: &mut App, account: Account, path: std::path::PathBuf, slim: bool) {
    app.task("Uploading skin", move |_, bus| async move {
        let png = tokio::fs::read(&path).await.with_context(|| format!("reading {}", path.display()))?;
        crate::minecraft::skin::compose(&png, slim)?; // reject non-skin images before uploading
        let account = auth::ensure_token(account).await?;
        let updated = auth::upload_skin(&account, png, slim).await?;
        bus.send(Event::Info("Skin updated".into()));
        bus.send(Event::Account(updated));
        Ok(())
    });
}

pub fn device_code_window(app: &mut App, ctx: &egui::Context) {
    let Some(code) = app.device_code.clone() else { return };
    let (_, closed) = widgets::dialog(ctx, "device-code", "Sign in with Microsoft", 420.0, |ui| {
        ui.label(RichText::new("Open the Microsoft page in your browser and enter this code:").color(theme::TEXT_MUTED));
        ui.add_space(12.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(theme::R_LG), theme::BG);
        let font = egui::FontId::monospace(30.0);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &code.user_code, font, theme::ACCENT);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::pill(ui, Kind::Primary, None, "Copy code and open browser").clicked() {
                ui.ctx().copy_text(code.user_code.clone());
                app.open(&code.verification_uri);
            }
        });
        ui.add_space(4.0);
        ui.hyperlink_to(RichText::new(&code.verification_uri).font(theme::label()), &code.verification_uri);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
            ui.label(RichText::new("Waiting for you to finish signing in…").color(theme::TEXT_MUTED));
        });
    });
    if closed {
        app.device_code = None;
        if let Some(cancel) = app.login_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
