// SPDX-License-Identifier: GPL-3.0-only
//! Library page: header, filters, a bar for the selected instance and the instance grid.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, vec2};

use crate::minecraft::instance::{Instance, Loader};
use crate::ui::glyph::{self, Glyph};
use crate::ui::icons::Icons;
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, Page, accounts, chrome, dialogs, editor, new_instance, settings, theme};

const CARD_MIN_WIDTH: f32 = 156.0;
const CARD_HEIGHT: f32 = 196.0;
const ART_HEIGHT: f32 = 124.0;
const GAP: f32 = theme::S3;
const HERO_HEIGHT: f32 = 96.0;
const HERO_ICON: f32 = 64.0;
const TOAST_TIME: Duration = Duration::from_secs(4);
/// Failures stay longer because the user may need to act on them.
const WARNING_TIME: Duration = Duration::from_secs(8);

#[derive(Clone, PartialEq, Default)]
pub struct Filter {
    pub group: Option<String>,
    pub loader: Option<Loader>,
}

impl Filter {
    fn matches(&self, inst: &Instance) -> bool {
        self.group.as_ref().is_none_or(|g| &inst.group == g) && self.loader.is_none_or(|l| inst.loader == l)
    }
}

pub struct Toast {
    text: String,
    at: Instant,
    failed: bool,
}

impl Toast {
    pub fn new(text: impl Into<String>, failed: bool) -> Self {
        Self { text: text.into(), at: Instant::now(), failed }
    }
}

enum Action {
    Select(String),
    Launch(String, bool),
    Kill(String),
    Edit(String, editor::Tab),
    Folder(String),
    Copy(String),
    Delete(String),
    ClearFilters,
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    chrome::title_bar(app, ctx);
    chrome::rail(app, ctx);
    let mut actions = Vec::new();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ctx, |ui| {
        if app.editor.is_some() {
            editor::page(app, ui);
            return;
        }
        match app.page {
            Page::Library => library(app, ui, &mut actions),
            Page::New => new_instance::page(app, ui),
            Page::Accounts => accounts::page(app, ui),
            Page::Settings => settings::page(app, ui),
        }
    });
    for action in actions {
        apply(app, action);
    }
    toast(app, ctx);
}
pub fn format_playtime(secs: u64) -> String {
    match secs {
        0 => "Never played".into(),
        s if s < 3600 => format!("{}m played", (s / 60).max(1)),
        s => format!("{}h {}m played", s / 3600, (s % 3600) / 60),
    }
}

fn time_ago(epoch_secs: u64) -> Option<String> {
    if epoch_secs == 0 {
        return None;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = now.saturating_sub(epoch_secs) / 86_400;
    Some(match days {
        0 => "Last played today".into(),
        1 => "Last played yesterday".into(),
        2..=13 => format!("Last played {days} days ago"),
        14..=59 => format!("Last played {} weeks ago", days / 7),
        _ => format!("Last played {} months ago", days / 30),
    })
}

pub fn instance_icon(ui: &egui::Ui, icons: &Icons, inst: &Instance, rect: Rect) {
    let tex = icons.get_opt(inst.icon_key().as_deref());
    widgets::icon(ui, rect, tex, &initial(inst), theme::loader_color(inst.loader));
}

/// The first letter, because names like "1.21.10 Fabric" start with a digit.
fn initial(inst: &Instance) -> String {
    let label: String = inst.name.chars().filter(|c| c.is_alphabetic()).take(1).collect();
    if label.is_empty() { inst.loader.name().to_string() } else { label }
}

fn visible(app: &App) -> Vec<&Instance> {
    let query = app.filter.trim().to_lowercase();
    let found = |i: &&Instance| {
        query.is_empty()
            || i.name.to_lowercase().contains(&query)
            || i.mc_version.contains(&query)
            || i.loader.name().to_lowercase().contains(&query)
    };
    app.instances.iter().filter(|i| app.library_filter.matches(i)).filter(found).collect()
}

fn library(app: &mut App, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        widgets::page_frame().show(ui, |ui| {
            header(app, ui);
            if app.instances.is_empty() {
                widgets::empty_state(
                    ui,
                    "No instances yet",
                    "Create one from any Minecraft version, or install a Modrinth modpack.",
                    |ui| {
                        if widgets::pill(ui, Kind::Primary, None, "New instance").clicked() {
                            app.go(Page::New);
                        }
                    },
                );
                return;
            }
            filters(app, ui);
            ui.add_space(theme::S4);
            let list = visible(app);
            if list.is_empty() {
                let text = "Nothing matches your search and filters.";
                widgets::empty_state(ui, "No matching instances", text, |ui| {
                    if widgets::pill(ui, Kind::Secondary, None, "Clear filters").clicked() {
                        actions.push(Action::ClearFilters);
                    }
                });
                return;
            }
            // A hidden instance stays selected but leaves the bar, so the bar can't show an
            // instance the search left out.
            if let Some(inst) = list.iter().find(|i| app.selected.as_deref() == Some(i.id.as_str())) {
                hero(app, ui, inst, actions);
                ui.add_space(theme::S4);
            }
            grid(app, ui, &list, actions);
        });
    });
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    let count = app.instances.len();
    let mut meta = format!("{count} instance{}", if count == 1 { "" } else { "s" });
    let total_play: u64 = app.instances.iter().map(|i| i.play_seconds).sum();
    if total_play >= 60 {
        meta.push_str(&format!("  ·  {}", format_playtime(total_play)));
    }
    widgets::page_header(ui, "Library", &meta, |ui| {
        if count > 0 {
            let width = (ui.available_width() - theme::S4).clamp(140.0, 240.0);
            widgets::search(ui, &mut app.filter, "Search instances", width);
        }
    });
}

fn filters(app: &mut App, ui: &mut egui::Ui) {
    let mut groups: Vec<String> = app.instances.iter().map(|i| i.group.clone()).filter(|g| !g.is_empty()).collect();
    groups.sort_unstable();
    groups.dedup();
    let loaders: Vec<Loader> = Loader::ALL.into_iter().filter(|l| app.instances.iter().any(|i| i.loader == *l)).collect();
    let filter = &mut app.library_filter;
    if filter.group.as_ref().is_some_and(|g| !groups.contains(g)) {
        filter.group = None;
    }
    if filter.loader.is_some_and(|l| !loaders.contains(&l)) {
        filter.loader = None;
    }
    if groups.is_empty() && loaders.len() < 2 {
        return;
    }
    ui.add_space(theme::S3);
    ui.horizontal(|ui| {
        let instances = &app.instances;
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width((ui.available_width() - 168.0).max(0.0));
            ui.spacing_mut().item_spacing = vec2(theme::S2, theme::S2);
            if !groups.is_empty() && widgets::chip(ui, "All", Some(instances.len()), filter.group.is_none()).clicked() {
                filter.group = None;
            }
            for group in &groups {
                let count = instances.iter().filter(|i| i.group == *group).count();
                if widgets::chip(ui, group, Some(count), filter.group.as_ref() == Some(group)).clicked() {
                    filter.group = Some(group.clone());
                }
            }
        });
        if loaders.len() > 1 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let label = filter.loader.map_or("All loaders", Loader::name);
                widgets::combo("library-loader", label, 136.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut filter.loader, None, "All loaders");
                    for loader in &loaders {
                        ui.selectable_value(&mut filter.loader, Some(*loader), loader.name());
                    }
                });
            });
        }
    });
}

fn hero(app: &App, ui: &mut egui::Ui, inst: &Instance, actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let radius = CornerRadius::same(theme::R_LG);
    ui.painter().rect_filled(rect, radius, theme::SURFACE);
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0_f32, theme::BORDER), StrokeKind::Inside);
    let running = app.running.contains_key(&inst.id);
    let can_play_offline = app.accounts.owns_game();

    let icon = Rect::from_center_size(Pos2::new(rect.left() + theme::S4 + HERO_ICON / 2.0, rect.center().y), vec2(HERO_ICON, HERO_ICON));
    instance_icon(ui, &app.icons, inst, icon);

    let id = inst.id.clone();
    let inner = rect.shrink(theme::S4);
    // A child, so laying out the buttons doesn't move this page's cursor back up into the bar.
    let mut buttons = ui.new_child(UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let more = widgets::icon_button_sized(&mut buttons, Glyph::More, "More", theme::H_LG);
    let popup = Id::new("hero-more");
    if more.clicked() {
        buttons.memory_mut(|m| m.toggle_popup(popup));
    }
    egui::popup::popup_below_widget(&buttons, popup, &more, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        instance_menu(ui, inst, running, can_play_offline, actions);
    });
    if widgets::pill_sized(&mut buttons, Kind::Secondary, None, "Manage", theme::H_LG).clicked() {
        actions.push(Action::Edit(id.clone(), editor::Tab::first_for(inst)));
    }
    if running {
        if widgets::pill_sized(&mut buttons, Kind::Danger, Some(Glyph::Stop), "Stop", theme::H_LG).clicked() {
            actions.push(Action::Kill(id.clone()));
        }
    } else if widgets::pill_sized(&mut buttons, Kind::Primary, Some(Glyph::Play), "Play", theme::H_LG).clicked() {
        actions.push(Action::Launch(id.clone(), false));
    }

    let x = icon.right() + theme::S4;
    let width = buttons.min_rect().left() - theme::S4 - x;
    let top = rect.center().y - 26.0;
    let name = Rect::from_min_size(Pos2::new(x, top), vec2(width, 30.0));
    if widgets::line(ui, name.min, &inst.name, theme::display(), theme::TEXT, width) {
        ui.interact(name, Id::new("hero-name"), Sense::hover()).on_hover_text(&inst.name);
    }
    let mut meta_x = x;
    let meta_y = top + 34.0;
    if running {
        ui.painter().circle_filled(Pos2::new(meta_x + 4.0, meta_y + 9.0), 4.0, theme::OK);
        let label = ui.painter().text(Pos2::new(meta_x + 14.0, meta_y), Align2::LEFT_TOP, "Running", theme::label(), theme::OK);
        meta_x = label.right() + theme::S2;
    }
    let mut meta = vec![inst.summary(), format_playtime(inst.play_seconds)];
    meta.extend(time_ago(inst.last_played));
    let meta = meta.join("  ·  ");
    if widgets::line(ui, Pos2::new(meta_x, meta_y), &meta, theme::label(), theme::TEXT_MUTED, x + width - meta_x) {
        let rect = Rect::from_min_size(Pos2::new(meta_x, meta_y), vec2(x + width - meta_x, 18.0));
        ui.interact(rect, Id::new("hero-meta"), Sense::hover()).on_hover_text(&meta);
    }
}

fn instance_menu(ui: &mut egui::Ui, inst: &Instance, running: bool, can_play_offline: bool, actions: &mut Vec<Action>) {
    widgets::popup_width(ui, 184.0);
    let id = &inst.id;
    if !running && can_play_offline && widgets::menu_item(ui, "Play offline").clicked() {
        actions.push(Action::Launch(id.clone(), true));
    }
    if widgets::menu_item(ui, "Settings").clicked() {
        actions.push(Action::Edit(id.clone(), editor::Tab::Settings));
    }
    if widgets::menu_item(ui, "Open folder").clicked() {
        actions.push(Action::Folder(id.clone()));
    }
    if widgets::menu_item(ui, "Duplicate").clicked() {
        actions.push(Action::Copy(id.clone()));
    }
    ui.separator();
    if widgets::menu_danger(ui, "Delete").clicked() {
        actions.push(Action::Delete(id.clone()));
    }
}

fn card(ui: &mut egui::Ui, icons: &Icons, inst: &Instance, width: f32, selected: bool, running: bool) -> (egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, CARD_HEIGHT), Sense::click());
    let mut play = false;
    if !ui.is_rect_visible(rect) {
        return (response, play);
    }
    // contains_pointer, not hovered: the play button on top would otherwise "unhover" the card.
    let hovered = response.contains_pointer();
    let radius = CornerRadius::same(theme::R_LG);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, if hovered { theme::RAISED } else { theme::SURFACE });
    if selected {
        painter.rect_stroke(rect, radius, Stroke::new(1.5_f32, theme::ACCENT), StrokeKind::Inside);
    } else {
        painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, theme::BORDER), StrokeKind::Inside);
    }
    let tint = theme::loader_color(inst.loader);
    let art = Rect::from_min_size(rect.min + vec2(theme::S2, theme::S2), vec2(width - theme::S4, ART_HEIGHT));
    painter.rect_filled(art, CornerRadius::same(theme::R_MD), mix(theme::SURFACE, tint, if hovered { 0.2 } else { 0.14 }));
    let icon = Rect::from_center_size(art.center(), vec2(72.0, 72.0));
    match icons.get_opt(inst.icon_key().as_deref()) {
        Some(tex) => widgets::icon(ui, icon, Some(tex), &inst.name, tint),
        None => widgets::letter(ui, icon, &initial(inst), tint),
    }

    if running {
        widgets::badge(ui, art.left_top() + vec2(theme::S2, theme::S2), "Running", theme::OK);
    }
    if hovered && !running {
        let button = Rect::from_center_size(art.right_bottom() - vec2(24.0, 24.0), vec2(theme::H_MD, theme::H_MD));
        let r = ui.interact(button, response.id.with("play"), Sense::click()).on_hover_text("Play");
        let fill = if r.hovered() { theme::ACCENT_HOVER } else { theme::ACCENT };
        ui.painter().rect_filled(button, CornerRadius::same(theme::R_MD), fill);
        let g = Rect::from_center_size(button.center() + vec2(1.0, 0.0), vec2(14.0, 14.0));
        glyph::paint(ui.painter(), g, Glyph::Play, theme::ON_ACCENT);
        play = r.clicked();
    }

    let text_x = rect.left() + theme::S3;
    let text_width = width - theme::S6;
    let cut = widgets::line(ui, Pos2::new(text_x, art.bottom() + theme::S3), &inst.name, theme::body_strong(), theme::TEXT, text_width);
    let meta = match inst.loader {
        Loader::Vanilla => inst.mc_version.clone(),
        l => format!("{}  ·  {}", inst.mc_version, l.name()),
    };
    widgets::line(ui, Pos2::new(text_x, art.bottom() + 32.0), &meta, theme::label(), theme::TEXT_MUTED, text_width);
    widgets::focus_ring(ui, rect, &response);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &inst.name));
    (if cut { response.on_hover_text(&inst.name) } else { response }, play)
}

/// As many columns as fit, sharing the full width, so the grid lines up with the bar above it.
fn grid(app: &App, ui: &mut egui::Ui, list: &[&Instance], actions: &mut Vec<Action>) {
    let can_play_offline = app.accounts.owns_game();
    let available = ui.available_width();
    let columns = (((available + GAP) / (CARD_MIN_WIDTH + GAP)).floor() as usize).max(1);
    let width = (available - GAP * (columns - 1) as f32) / columns as f32;
    ui.spacing_mut().item_spacing.y = GAP;
    for row in list.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for inst in row {
                let selected = app.selected.as_deref() == Some(inst.id.as_str());
                let running = app.running.contains_key(&inst.id);
                let (r, play) = card(ui, &app.icons, inst, width, selected, running);
                if play || r.double_clicked() {
                    actions.push(Action::Launch(inst.id.clone(), false));
                }
                if r.clicked() || play {
                    actions.push(Action::Select(inst.id.clone()));
                }
                r.context_menu(|ui| {
                    if !running && widgets::menu_item(ui, "Play").clicked() {
                        actions.push(Action::Launch(inst.id.clone(), false));
                        ui.close_menu();
                    }
                    let before = actions.len();
                    instance_menu(ui, inst, running, can_play_offline, actions);
                    if actions.len() > before {
                        ui.close_menu();
                    }
                });
            }
        });
    }
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn apply(app: &mut App, action: Action) {
    match action {
        Action::Select(id) => app.selected = Some(id),
        Action::Launch(id, offline) => app.launch(&id, offline),
        Action::Kill(id) => app.kill(&id),
        Action::Edit(id, tab) => app.open_editor(&id, tab),
        Action::Folder(id) => {
            if let Some(dir) = app.instance(&id).map(Instance::game_dir) {
                app.open_folder(&dir);
            }
        }
        Action::Copy(id) => {
            if let Some(inst) = app.instance(&id).cloned() {
                app.task(format!("Copying {}", inst.name), move |_, bus| async move {
                    let copy = tokio::task::spawn_blocking(move || inst.duplicate()).await??;
                    bus.send(crate::ui::Event::InstancesChanged(Some(copy.id)));
                    Ok(())
                });
            }
        }
        Action::Delete(id) => app.confirm = Some(dialogs::Confirm::DeleteInstance(id)),
        Action::ClearFilters => {
            app.filter.clear();
            app.library_filter = Filter::default();
        }
    }
}

fn toast(app: &mut App, ctx: &egui::Context) {
    let Some(toast) = &app.toast else { return };
    let left = (if toast.failed { WARNING_TIME } else { TOAST_TIME }).saturating_sub(toast.at.elapsed());
    if left.is_zero() {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(left);
    egui::Area::new(Id::new("toast"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-theme::PAGE_PAD, -theme::PAGE_PAD))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::RAISED)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(CornerRadius::same(theme::R_MD))
                .inner_margin(egui::Margin::symmetric(theme::S3 as i8, 10))
                .show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, if toast.failed { theme::ERR } else { theme::TEXT_MUTED });
                        ui.add(egui::Label::new(egui::RichText::new(toast.text.as_str()).color(theme::TEXT)).wrap());
                    });
                });
        });
}
#[cfg(test)]
mod tests {
    use super::*;

    fn inst(group: &str, loader: Loader) -> Instance {
        Instance { group: group.into(), loader, ..Default::default() }
    }

    #[test]
    fn filters_combine() {
        let filter = Filter { group: Some("Modded".into()), loader: Some(Loader::Fabric) };
        assert!(filter.matches(&inst("Modded", Loader::Fabric)));
        assert!(!filter.matches(&inst("Modded", Loader::Forge)));
        assert!(!filter.matches(&inst("", Loader::Fabric)));
        assert!(Filter::default().matches(&inst("", Loader::Vanilla)));
    }
}
