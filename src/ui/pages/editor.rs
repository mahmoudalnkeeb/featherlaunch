// SPDX-License-Identifier: GPL-3.0-only
//! Instance page: mods / resource packs / shader packs (installed + Modrinth), worlds, log, settings.
//! It replaces the library in the main area instead of floating in a window.

use std::path::PathBuf;

use eframe::egui::{self, Pos2, Rect, RichText, Sense, UiBuilder, vec2};

use crate::minecraft::instance::{ExportInfo, Instance, Loader, World};
use crate::minecraft::logs::{self, SavedLog};
use crate::minecraft::world;
use crate::modplatform::export;
use crate::modplatform::modrinth::{self, ContentKind, LocalFile, SearchResponse, Update};
use crate::ui::dialogs::Confirm;
use crate::ui::glyph::Glyph;
use crate::ui::project::{self, Browser, Request};
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, Event, Page, console, main_view, theme};

const FOOTER_HEIGHT: f32 = 56.0;
/// Room for a world row's buttons, leaving the rest of the row for its name.
const WORLD_BUTTONS: f32 = 372.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Content(ContentKind),
    Worlds,
    Log,
    Settings,
    Export,
}

impl Tab {
    pub fn first_for(inst: &Instance) -> Self {
        if inst.loader == Loader::Vanilla { Tab::Worlds } else { Tab::Content(ContentKind::Mod) }
    }
}

struct ExportForm {
    as_pack: bool,
    info: ExportInfo,
    items: Vec<(String, bool)>,
    /// The player has chosen items, so changing the format no longer resets them.
    touched: bool,
}

pub struct Editor {
    inst: Instance,
    draft: Instance,
    tab: Tab,
    files: Option<Vec<LocalFile>>,
    worlds: Option<Vec<World>>,
    /// The world whose data packs are open on the Worlds tab.
    pack_world: Option<World>,
    renaming: Option<(World, String)>,
    show_saved_logs: bool,
    saved_logs: Option<Vec<SavedLog>>,
    open_log: Option<(String, console::Console)>,
    export: Option<ExportForm>,
    stale: bool,
    /// Numbers each read, so only the answer to the latest one is used.
    request: u64,
    identified: std::collections::HashSet<ContentKind>,
    browse: bool,
    pub browser: Browser,
    searched_kind: Option<ContentKind>,
    updates: Vec<Update>,
    checking: bool,
    memory_override: bool,
    jvm_override: bool,
    window_override: bool,
    /// A text field or popup was active last frame, so it handles Esc. This reads last frame's
    /// state because egui clears focus on Esc before the frame starts.
    esc_taken: bool,
}

impl Editor {
    pub fn new(inst: Instance, tab: Tab) -> Self {
        Self {
            memory_override: inst.memory_mb.is_some(),
            jvm_override: inst.jvm_args.is_some(),
            window_override: inst.window.is_some(),
            draft: inst.clone(),
            inst,
            tab,
            files: None,
            worlds: None,
            pack_world: None,
            renaming: None,
            show_saved_logs: false,
            saved_logs: None,
            open_log: None,
            export: None,
            stale: true,
            request: 0,
            browse: false,
            browser: Browser::default(),
            searched_kind: None,
            updates: Vec::new(),
            checking: false,
            identified: Default::default(),
            esc_taken: false,
        }
    }

    pub fn instance_id(&self) -> &str {
        &self.inst.id
    }

    pub fn name(&self) -> &str {
        &self.inst.name
    }

    pub fn has_changes(&self) -> bool {
        self.draft != self.inst
    }

    /// The list on screen stays until the new one arrives, so it doesn't flicker.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    fn start_read(&mut self) -> Option<u64> {
        std::mem::take(&mut self.stale).then(|| {
            self.request += 1;
            self.request
        })
    }

    pub fn on_files(&mut self, request: u64, files: Vec<LocalFile>) {
        if request == self.request {
            self.files = Some(files);
        }
    }

    pub fn on_worlds(&mut self, request: u64, worlds: Vec<World>) {
        if request == self.request {
            self.worlds = Some(worlds);
        }
    }

    pub fn on_saved_logs(&mut self, request: u64, logs: Vec<SavedLog>) {
        if request == self.request {
            self.saved_logs = Some(logs);
        }
    }

    pub fn on_saved_log(&mut self, name: String, text: &str) {
        if self.show_saved_logs {
            self.open_log = Some((name, console::Console::saved(text)));
        }
    }

    pub fn set_icon_stamp(&mut self, stamp: Option<u128>) {
        self.inst.icon_stamp = stamp;
        self.draft.icon_stamp = stamp;
    }

    fn browsing(&self, kind: ContentKind) -> bool {
        self.tab == Tab::Content(kind) || (kind == ContentKind::DataPack && self.pack_world.is_some())
    }

    pub fn on_search(&mut self, kind: ContentKind, offset: u64, results: SearchResponse) {
        if self.browsing(kind) {
            self.browser.on_search(offset, results);
        }
    }

    pub fn on_search_failed(&mut self, kind: ContentKind) {
        if self.browsing(kind) {
            self.browser.on_search_failed();
        }
    }

    fn open_data_packs(&mut self, world: World) {
        self.pack_world = Some(world);
        self.reset_content();
    }

    fn close_data_packs(&mut self) {
        self.pack_world = None;
        self.reset_content();
    }

    /// Clears what belongs to the list or search on screen.
    fn reset_content(&mut self) {
        self.files = None;
        self.worlds = None;
        self.stale = true;
        self.browse = false;
        self.updates.clear();
        self.browser.clear();
        self.searched_kind = None;
    }

    pub fn on_updates(&mut self, updates: Vec<Update>) {
        self.checking = false;
        self.updates = updates;
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            let browse = self.browse;
            self.tab = tab;
            self.pack_world = None;
            self.renaming = None;
            self.reset_content();
            self.browse = browse;
        }
    }
}

enum PageAction {
    Back,
    Launch,
    Kill,
}

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let Some(mut ed) = app.editor.take() else { return };
    let mut action = None;
    if ed.tab == Tab::Settings && ed.has_changes() {
        save_bar(app, ui, &mut ed);
    }
    widgets::page_frame().show(ui, |ui| {
        header(app, ui, &ed, &mut action);
        ui.add_space(theme::S3);
        let mut tab = ed.tab;
        let tabs = [
            (Tab::Content(ContentKind::Mod), "Mods"),
            (Tab::Content(ContentKind::ResourcePack), "Resource packs"),
            (Tab::Content(ContentKind::ShaderPack), "Shaders"),
            (Tab::Worlds, "Worlds"),
            (Tab::Log, "Log"),
            (Tab::Settings, "Settings"),
            (Tab::Export, "Export"),
        ];
        if widgets::tabs(ui, &mut tab, &tabs) {
            ed.set_tab(tab);
        }
        ui.add_space(theme::S4);
        match ed.tab {
            Tab::Settings => settings(app, ui, &mut ed),
            Tab::Content(kind) => content(app, ui, &mut ed, kind),
            Tab::Worlds => worlds(app, ui, &mut ed),
            Tab::Log => log_tab(app, ui, &mut ed),
            Tab::Export => export_tab(app, ui, &mut ed),
        }
    });
    let esc = !ed.esc_taken
        && app.errors.is_empty()
        && app.confirm.is_none()
        && app.device_code.is_none()
        && ui.input(|i| i.key_pressed(egui::Key::Escape));
    if esc {
        action = Some(PageAction::Back);
    }
    ed.esc_taken = ui.ctx().memory(|m| m.focused().is_some() || m.any_popup_open());
    let id = ed.inst.id.clone();
    app.editor = Some(ed);
    // After the page is back in place, so launching (which may open the Log tab) sees it, and
    // leaving can ask about unsaved edits.
    match action {
        Some(PageAction::Back) => app.go(Page::Library),
        Some(PageAction::Launch) => app.launch(&id, false),
        Some(PageAction::Kill) => app.kill(&id),
        None => {}
    }
}

fn header(app: &mut App, ui: &mut egui::Ui, ed: &Editor, action: &mut Option<PageAction>) {
    let running = app.running.contains_key(&ed.inst.id);
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, Glyph::Back, "Back to library (Esc)").clicked() {
            *action = Some(PageAction::Back);
        }
        let (icon, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        main_view::instance_icon(ui, &app.icons, &ed.inst, icon);
        ui.add_space(theme::S1);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if running {
                if widgets::pill_sized(ui, Kind::Danger, Some(Glyph::Stop), "Stop", theme::H_LG).clicked() {
                    *action = Some(PageAction::Kill);
                }
            } else if widgets::pill_sized(ui, Kind::Primary, Some(Glyph::Play), "Play", theme::H_LG).clicked() {
                *action = Some(PageAction::Launch);
            }
            ui.add_space(theme::S2);
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                widgets::line_tip(ui, &ed.inst.name, theme::display(), theme::TEXT, 30.0);
                let meta = format!("{}  ·  {}", ed.inst.summary(), main_view::format_playtime(ed.inst.play_seconds));
                widgets::line_tip(ui, &meta, theme::label(), theme::TEXT_MUTED, 18.0);
            });
        });
    });
}

fn icon_row(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    let mut changed = false;
    widgets::form_row(ui, "Icon", |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            main_view::instance_icon(ui, &app.icons, &ed.inst, rect);
            ui.add_space(theme::S1);
            if widgets::pill(ui, Kind::Secondary, None, "Change…").clicked() {
                let picked = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif"]).pick_file();
                if let Some(path) = picked {
                    let (dir, id, bus) = (ed.inst.dir(), ed.inst.id.clone(), app.bus.clone());
                    app.spawn(async move {
                        tokio::task::spawn_blocking(move || crate::icons::set_instance_icon_file(&path, &dir)).await??;
                        bus.send(Event::InstanceIconChanged(id));
                        Ok(())
                    });
                }
            }
            if ed.inst.icon_stamp.is_some() && widgets::pill(ui, Kind::Ghost, None, "Reset").clicked() {
                match ed.inst.reset_icon() {
                    Ok(()) => changed = true,
                    Err(e) => app.errors.push(format!("Could not reset the icon of {}: {e:#}", ed.inst.name)),
                }
            }
        });
    });
    if changed {
        app.reload_instances(Some(ed.inst.id.clone()));
        ed.set_icon_stamp(app.instance(&ed.inst.id).and_then(|i| i.icon_stamp));
    }
}

fn settings(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    app.request_manifest();
    let mut open_game_dir = false;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        widgets::form(ui, |ui| {
            widgets::form_section(ui, "General", true);
            icon_row(app, ui, ed);
            let d = &mut ed.draft;
            widgets::form_row(ui, "Name", |ui| {
                ui.add(widgets::input(&mut d.name, "Instance name").desired_width(ui.available_width()));
            });
            widgets::form_row(ui, "Group", |ui| {
                ui.add(widgets::input(&mut d.group, "None").desired_width(ui.available_width()));
            });
            widgets::form_row(ui, "Folder", |ui| {
                if widgets::pill(ui, Kind::Secondary, None, "Open instance folder").clicked() {
                    open_game_dir = true;
                }
            });

            widgets::form_section(ui, "Game", false);
            widgets::form_row(ui, "Minecraft", |ui| {
                let current = d.mc_version.clone();
                widgets::combo("mc", &current, 200.0).show_ui(ui, |ui| {
                    for v in app.manifest.iter().flatten().filter(|v| v.kind == "release" || v.kind.starts_with("old_") || v.id == current)
                    {
                        if ui.selectable_value(&mut d.mc_version, v.id.clone(), &v.id).changed() {
                            d.loader_version = None;
                        }
                    }
                });
            });
            widgets::form_row(ui, "Mod loader", |ui| {
                let mut loader = d.loader;
                let items: Vec<(Loader, &str)> = Loader::ALL.iter().map(|l| (*l, l.name())).collect();
                if widgets::segmented(ui, &mut loader, &items) {
                    d.loader = loader;
                    d.loader_version = None;
                }
            });
            if d.loader != Loader::Vanilla {
                app.request_loader_versions(d.loader, &d.mc_version);
                widgets::form_row(ui, "Loader version", |ui| {
                    let text = d.loader_version.clone().unwrap_or_else(|| "Latest".into());
                    widgets::combo("lv", text, 200.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.loader_version, None, "Latest");
                        for v in app.loader_versions.get(&(d.loader, d.mc_version.clone())).into_iter().flatten() {
                            ui.selectable_value(&mut d.loader_version, Some(v.clone()), v);
                        }
                    });
                });
            }

            widgets::form_section(ui, "Game window", false);
            widgets::form_row(ui, "Window size", |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::H_MD);
                    widgets::toggle(ui, &mut ed.window_override);
                    if !ed.window_override {
                        let w = app.cfg.window;
                        ui.label(RichText::new(format!("Default ({} × {})", w.width, w.height)).color(theme::TEXT_MUTED));
                    }
                });
            });
            if ed.window_override {
                let mut window = d.window.unwrap_or(app.cfg.window);
                super::settings::window_rows(ui, &mut window);
                d.window = Some(window);
            } else {
                d.window = None;
            }

            widgets::form_section(ui, "Java", false);
            widgets::form_row(ui, "Memory", |ui| {
                let mut mb = d.memory_mb.unwrap_or(app.cfg.memory_mb);
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::H_MD);
                    widgets::toggle(ui, &mut ed.memory_override);
                    if !ed.memory_override {
                        ui.label(RichText::new(format!("Default ({} MB)", app.cfg.memory_mb)).color(theme::TEXT_MUTED));
                    }
                });
                if ed.memory_override {
                    widgets::slider(ui, &mut mb, 512..=16384, 256.0, " MB");
                }
                d.memory_mb = ed.memory_override.then_some(mb);
            });
            if ed.memory_override {
                let mut min = d.min_memory_mb.unwrap_or(app.cfg.min_memory_mb);
                widgets::form_row(ui, "Minimum memory", |ui| {
                    widgets::slider(ui, &mut min, 256..=8192, 256.0, " MB");
                });
                d.min_memory_mb = Some(min);
            } else {
                d.min_memory_mb = None;
            }
            widgets::form_row(ui, "Java runtime", |ui| {
                let text = d.java_path.clone().unwrap_or_else(|| "Automatic".into());
                widgets::combo_cut(ui, "java", text, ui.available_width(), |ui| {
                    ui.selectable_value(&mut d.java_path, None, "Automatic");
                    for j in &app.java_list {
                        let path = j.path.to_string_lossy().into_owned();
                        ui.selectable_value(&mut d.java_path, Some(path.clone()), format!("Java {} — {path}", j.version));
                    }
                });
            });
            widgets::form_row(ui, "JVM arguments", |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::H_MD);
                    widgets::toggle(ui, &mut ed.jvm_override);
                    if !ed.jvm_override {
                        ui.label(RichText::new("Default").color(theme::TEXT_MUTED));
                    }
                });
                let mut args = d.jvm_args.clone().unwrap_or_else(|| app.cfg.jvm_args.clone());
                if ed.jvm_override {
                    widgets::multiline(ui, &mut args);
                }
                d.jvm_args = ed.jvm_override.then_some(args);
            });
        });
    });
    if open_game_dir {
        app.open_folder(&ed.inst.game_dir());
    }
}

fn save_bar(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    let running = app.running.contains_key(&ed.inst.id);
    let status = if running { "Close the game to save these changes." } else { "Unsaved changes" };
    widgets::save_bar(ui, "instance-save", status, |ui| {
        let save = ui.add_enabled_ui(!running, |ui| widgets::pill(ui, Kind::Primary, None, "Save"));
        if save.inner.clicked() {
            let mut saved = app.instance(&ed.inst.id).cloned().unwrap_or_else(|| ed.inst.clone());
            saved.apply_settings(&ed.draft);
            match saved.save() {
                Err(e) => app.errors.push(format!("Saving failed: {e:#}")),
                Ok(()) => {
                    ed.inst = saved.clone();
                    ed.draft = saved;
                    app.reload_instances(Some(ed.inst.id.clone()));
                    app.notify(format!("Saved {}", ed.inst.name));
                }
            }
        }
        if widgets::pill(ui, Kind::Secondary, None, "Discard").clicked() {
            ed.draft = ed.inst.clone();
            ed.memory_override = ed.inst.memory_mb.is_some();
            ed.jvm_override = ed.inst.jvm_args.is_some();
            ed.window_override = ed.inst.window.is_some();
        }
    });
}

fn content(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor, kind: ContentKind) {
    if kind == ContentKind::Mod && ed.inst.loader == Loader::Vanilla {
        let text = "This instance has no mod loader. Pick one in Settings to use mods.";
        if widgets::notice(ui, theme::WARN, text, Some("Open Settings")) {
            ed.set_tab(Tab::Settings);
            return;
        }
        ui.add_space(theme::S3);
    }
    if !ed.browse {
        read_files(app, ed, kind);
    }
    ui.horizontal(|ui| {
        let installed = ed.files.as_ref().map_or("Installed".to_string(), |f| format!("Installed ({})", f.len()));
        let mut browse = ed.browse;
        widgets::segmented(ui, &mut browse, &[(false, installed.as_str()), (true, "Browse Modrinth")]);
        ed.browse = browse;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::pill(ui, Kind::Ghost, None, "Open folder").clicked() {
                match modrinth::ensure_folder(&ed.inst, kind) {
                    Ok(dir) => app.open(dir),
                    Err(e) => app.errors.push(format!("Could not create the {} folder of {}: {e:#}", kind.folder(), ed.inst.name)),
                }
            }
            if !ed.browse {
                if widgets::pill(ui, Kind::Ghost, None, "Add files").clicked()
                    && let Some(files) = rfd::FileDialog::new().add_filter(kind.label(), kind.extensions()).pick_files()
                {
                    add_files(app, ed, kind, files);
                }
                update_buttons(app, ui, ed, kind);
            }
        });
    });
    ui.add_space(theme::S3);
    let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        add_files(app, ed, kind, dropped);
    }
    if ed.browse {
        browse(app, ui, ed, kind);
    } else {
        installed(app, ui, ed, kind);
    }
}

fn export_tab(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    let inst = &ed.inst;
    let form = ed.export.get_or_insert_with(|| ExportForm {
        as_pack: false,
        info: inst.export.clone(),
        items: export::top_level(&inst.game_dir()).into_iter().map(|n| (n, true)).collect(),
        touched: false,
    });
    let mut start = false;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        widgets::form(ui, |ui| {
            widgets::form_section(ui, "Format", true);
            widgets::form_row(ui, "Export as", |ui| {
                widgets::segmented(ui, &mut form.as_pack, &[(false, "Instance (.zip)"), (true, "Modpack (.mrpack)")]);
                let hint = if form.as_pack { "To share. Mods on Modrinth become links." } else { "A full copy, for backups." };
                widgets::hint(ui, hint);
            });
            if form.as_pack {
                widgets::form_row(ui, "Version", |ui| {
                    ui.add(widgets::input(&mut form.info.version, "1.0.0").desired_width(ui.available_width()));
                });
                widgets::form_row(ui, "Summary", |ui| {
                    ui.add(widgets::input(&mut form.info.summary, "Optional").desired_width(ui.available_width()));
                });
            }
            widgets::form_section(ui, "Include", false);
            if !form.touched {
                for (name, on) in &mut form.items {
                    *on = export::included_by_default(name, form.as_pack);
                }
            }
            for (name, on) in &mut form.items {
                widgets::form_row(ui, name, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(theme::H_MD);
                        form.touched |= widgets::toggle(ui, on).changed();
                    });
                });
            }
            ui.add_space(theme::S3);
            start = widgets::pill(ui, Kind::Secondary, None, "Export…").clicked();
        });
    });
    if !start {
        return;
    }
    let extension = if form.as_pack { "mrpack" } else { "zip" };
    let file_name = format!("{}.{extension}", ed.inst.name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-"));
    let Some(target) = rfd::FileDialog::new().add_filter(extension, &[extension]).set_file_name(file_name).save_file() else { return };
    let info = form.info.clone();
    let as_pack = form.as_pack;
    let include: Vec<String> = form.items.iter().filter(|(_, on)| *on).map(|(n, _)| n.clone()).collect();
    ed.inst.export = info.clone();
    ed.draft.export = info.clone();
    if let Some(saved) = app.instances.iter_mut().find(|i| i.id == ed.inst.id) {
        saved.export = info.clone();
        if let Err(e) = saved.save() {
            app.errors.push(format!("Saving {} failed: {e:#}", saved.name));
        }
    }
    let inst = ed.inst.clone();
    app.task(format!("Exporting {}", inst.name), move |progress, bus| async move {
        if as_pack {
            export::export_mrpack(&inst, include, &info, target.clone(), &progress).await?;
        } else {
            let (dir, out) = (inst.dir(), target.clone());
            tokio::task::spawn_blocking(move || export::export_zip(&dir, &include, &out)).await??;
        }
        bus.send(Event::Info(format!("Exported to {}", target.display())));
        Ok(())
    });
}

fn log_tab(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    if widgets::segmented(ui, &mut ed.show_saved_logs, &[(false, "This session"), (true, "Saved logs")]) {
        ed.saved_logs = None;
        ed.open_log = None;
        ed.stale = true;
    }
    ui.add_space(theme::S3);
    if !ed.show_saved_logs {
        let action = app.consoles.entry(ed.inst.id.clone()).or_default().ui(ui);
        console::handle(app, action);
        return;
    }
    if let Some((name, log)) = &mut ed.open_log {
        let mut back = false;
        ui.horizontal(|ui| {
            back = widgets::pill(ui, Kind::Ghost, None, "Back to saved logs").clicked();
            ui.label(RichText::new(name.as_str()).color(theme::TEXT_MUTED));
        });
        ui.add_space(theme::S2);
        let action = log.ui(ui);
        console::handle(app, action);
        if back {
            ed.open_log = None;
        }
        return;
    }
    if let Some(request) = ed.start_read() {
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.rt.spawn_blocking(move || {
            let saved = logs::list(&inst.game_dir());
            bus.send(Event::SavedLogs { inst: inst.id, request, logs: saved });
        });
    }
    let Some(saved) = &ed.saved_logs else {
        ui.add(egui::Spinner::new().color(theme::ACCENT));
        return;
    };
    if saved.is_empty() {
        widgets::empty_state(ui, "No saved logs", "Logs and crash reports show up here after you play.", |_| {});
        return;
    }
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
        for log in saved {
            let (row, response) = widgets::list_row(ui, 52.0, Sense::click(), false);
            if !ui.is_rect_visible(row) {
                continue;
            }
            let (x, width) = (row.left() + theme::S3, row.width() - 2.0 * theme::S3);
            widgets::line(ui, Pos2::new(x, row.top() + 8.0), &log.name, theme::body_strong(), theme::TEXT, width);
            let when = days_ago("Saved", log.modified);
            widgets::line(ui, Pos2::new(x, row.top() + 27.0), &when, theme::label(), theme::TEXT_MUTED, width);
            if response.clicked() {
                open = Some(log.clone());
            }
        }
    });
    if let Some(log) = open {
        let (id, bus) = (ed.inst.id.clone(), app.bus.clone());
        app.spawn(async move {
            let path = log.path.clone();
            let text = tokio::task::spawn_blocking(move || logs::read(&path)).await??;
            bus.send(Event::SavedLog { inst: id, name: log.name, text });
            Ok(())
        });
    }
}

fn add_files(app: &mut App, ed: &mut Editor, kind: ContentKind, files: Vec<PathBuf>) {
    let (inst, bus) = (ed.inst.clone(), app.bus.clone());
    let world = ed.pack_world.as_ref().map(|w| w.path.clone());
    ed.browse = false;
    app.spawn(async move {
        let added = tokio::task::spawn_blocking(move || match world {
            Some(world) => modrinth::add_data_packs(&world, &files),
            None => modrinth::add_files(&inst, kind, &files),
        })
        .await??;
        // Newly added files still need looking up on Modrinth.
        bus.send(Event::ContentChanged);
        bus.send(match added {
            0 => Event::Warning(format!("Those files don't belong in {}", kind.label())),
            1 => Event::Info("Added 1 file".to_string()),
            n => Event::Info(format!("Added {n} files")),
        });
        Ok(())
    });
    ed.identified.remove(&kind);
}

fn read_files(app: &mut App, ed: &mut Editor, kind: ContentKind) {
    if ed.identified.insert(kind) {
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.rt.spawn(async move {
            // Best effort: offline or unknown files just keep their file names.
            if let Ok(true) = modrinth::identify(&inst, kind).await {
                bus.send(Event::ContentChanged);
            }
        });
    }
    if let Some(request) = ed.start_read() {
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.rt.spawn_blocking(move || {
            let files = modrinth::list_local(&inst, kind);
            bus.send(Event::Files { inst: inst.id, request, files });
        });
    }
}

fn update_buttons(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor, kind: ContentKind) {
    if !ed.updates.is_empty() {
        if widgets::pill(ui, Kind::Primary, None, &format!("Update all ({})", ed.updates.len())).clicked() {
            let (inst, updates) = (ed.inst.clone(), std::mem::take(&mut ed.updates));
            app.cancellable_task(format!("Updating {} files", updates.len()), move |progress, bus| async move {
                for u in updates {
                    modrinth::apply_update(&inst, kind, u, &progress).await?;
                }
                bus.send(Event::ContentChanged);
                Ok(())
            });
        }
        return;
    }
    let has_files = ed.files.as_ref().is_some_and(|f| !f.is_empty());
    let check = ui.add_enabled_ui(!ed.checking && has_files, |ui| widgets::pill(ui, Kind::Secondary, None, "Check for updates"));
    if check.inner.clicked() {
        ed.checking = true;
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.spawn(async move {
            match modrinth::check_updates(&inst, kind).await {
                Ok(updates) => {
                    if updates.is_empty() {
                        bus.send(Event::Info("Everything is up to date".to_string()));
                    }
                    bus.send(Event::Updates(updates));
                    Ok(())
                }
                Err(e) => {
                    bus.send(Event::Updates(Vec::new()));
                    Err(e)
                }
            }
        });
    }
    if ed.checking {
        ui.add(egui::Spinner::new().color(theme::ACCENT));
    }
}

fn installed(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor, kind: ContentKind) {
    let Some(files) = &ed.files else {
        ui.add(egui::Spinner::new().color(theme::ACCENT));
        return;
    };
    let mut toggle = None;
    let mut delete = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        if files.is_empty() {
            widgets::empty_state(ui, "Nothing installed yet", "Browse Modrinth, or drop files here.", |ui| {
                if widgets::pill(ui, Kind::Primary, None, "Browse Modrinth").clicked() {
                    ed.browse = true;
                }
            });
        }
        ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
        for (i, f) in files.iter().enumerate() {
            let (row, _) = widgets::list_row(ui, 52.0, Sense::hover(), false);
            if !ui.is_rect_visible(row) {
                continue;
            }
            let title = f.title.clone().unwrap_or_else(|| f.file_name.trim_end_matches(".disabled").to_string());
            let update = ed.updates.iter().find(|u| u.file.file_name == f.file_name);
            let inner =
                UiBuilder::new().max_rect(row.shrink2(vec2(theme::S3, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center));
            ui.scope_builder(inner, |ui| {
                let mut enabled = f.enabled;
                let tip = if f.enabled { "Disable" } else { "Enable" };
                if widgets::toggle(ui, &mut enabled).on_hover_text(tip).changed() {
                    toggle = Some((i, enabled));
                }
                ui.add_space(theme::S1);
                let (icon, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                widgets::icon(ui, icon, app.icons.get_opt(f.icon_url.as_deref()), &title, theme::TEXT_MUTED);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, Glyph::Trash, "Delete").clicked() {
                        delete = Some(i);
                    }
                    if let Some(u) = update {
                        let label = format!("→ {}", u.version.version_number);
                        let (r, _) = ui.allocate_exact_size(vec2(widgets::badge_width(ui, &label), 18.0), Sense::hover());
                        widgets::badge_exact(ui, r.min, &label, theme::OK);
                    }
                    let (text, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                    let color = if f.enabled { theme::TEXT } else { theme::TEXT_MUTED };
                    let cut = widgets::line(ui, text.min, &title, theme::body_strong(), color, text.width());
                    let version = f.version.as_deref().unwrap_or(&f.file_name);
                    let sub = if f.enabled { version.to_string() } else { format!("Disabled  ·  {version}") };
                    widgets::line(ui, text.min + vec2(0.0, 19.0), &sub, theme::label(), theme::TEXT_MUTED, text.width());
                    if cut {
                        response.on_hover_text(&title);
                    }
                });
            });
        }
    });
    if let Some((i, enabled)) = toggle {
        if let Err(e) = modrinth::set_enabled(&files[i], enabled) {
            app.errors.push(format!("Could not change {}: {e:#}", files[i].file_name));
        }
        ed.invalidate();
    } else if let Some(i) = delete {
        app.confirm = Some(Confirm::DeleteFile { inst: Box::new(ed.inst.clone()), kind, file: Box::new(files[i].clone()) });
    }
}

fn browse(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor, kind: ContentKind) {
    let mc = ed.inst.mc_version.clone();
    let loader = ed.inst.loader;
    let target = Some((mc.clone(), loader));
    let note = if loader == Loader::Vanilla { format!("for {mc}") } else { format!("for {mc}  ·  {}", loader.name()) };
    let hint = format!("Search {}", kind.label().to_lowercase());
    let mut install = None;
    let footer = |ui: &mut egui::Ui, versions: &[crate::modplatform::modrinth::Version], idx: &mut usize| {
        *idx = (*idx).min(versions.len() - 1);
        ui.horizontal(|ui| {
            let width = ui.available_width() - project::INSTALL_WIDTH - theme::S2;
            widgets::combo_cut(ui, "project-version", &versions[*idx].name, width, |ui| {
                for (i, v) in versions.iter().enumerate() {
                    ui.selectable_value(idx, i, format!("{} ({})", v.name, v.loaders.join(", ")));
                }
            });
            if widgets::pill(ui, Kind::Primary, None, "Install").clicked() {
                install = Some(versions[*idx].clone());
            }
        });
        let deps = versions[*idx].dependencies.iter().filter(|d| d.dependency_type == "required").count();
        if deps > 0 {
            widgets::hint(ui, &format!("Also installs {deps} required dependenc{}", if deps == 1 { "y" } else { "ies" }));
        }
    };
    let mut request = project::show(ui, &app.icons, &mut ed.browser, &hint, &note, FOOTER_HEIGHT, footer);
    if ed.searched_kind != Some(kind) {
        ed.searched_kind = Some(kind);
        request = Some(Request::Search(0));
    }
    if let (Some(version), Some(hit)) = (install, &ed.browser.hit) {
        let (inst, meta, world) = (ed.inst.clone(), hit.meta(), ed.pack_world.clone());
        app.cancellable_task(format!("Installing {}", meta.title), move |progress, bus| async move {
            if let Some(world) = world {
                modrinth::install_data_pack(&world.path, &version, &progress).await?;
                bus.send(Event::ContentChanged);
                bus.send(Event::Info(format!("Installed {} in {}", meta.title, world.name)));
                return Ok(());
            }
            let installed = modrinth::install(&inst, kind, version, meta, &progress).await?;
            bus.send(Event::ContentChanged);
            match installed.warning(&inst) {
                Some(warning) => bus.send(Event::Error(warning)),
                None => bus.send(Event::Info(installed.summary())),
            }
            Ok(())
        });
    }
    match request {
        Some(Request::Search(offset)) => {
            ed.browser.start_search();
            app.search_modrinth(ed.browser.query.clone(), kind, target, offset);
        }
        Some(Request::Versions(hit)) => {
            let project = hit.project_id.clone();
            ed.browser.open(hit);
            app.load_project_versions(project, kind, target);
        }
        None => {}
    }
}
fn days_ago(verb: &str, modified: Option<std::time::SystemTime>) -> String {
    modified
        .and_then(|m| m.elapsed().ok())
        .map(|d| match d.as_secs() / 86_400 {
            0 => format!("{verb} today"),
            1 => format!("{verb} yesterday"),
            n => format!("{verb} {n} days ago"),
        })
        .unwrap_or_default()
}

enum WorldAction {
    DataPacks(World),
    Copy(World),
    Rename(World),
    Delete(World),
}

fn worlds(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    if let Some(world) = ed.pack_world.clone() {
        data_packs(app, ui, ed, &world);
        return;
    }
    if let Some(request) = ed.start_read() {
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.rt.spawn_blocking(move || {
            let worlds = inst.worlds();
            bus.send(Event::Worlds { inst: inst.id, request, worlds });
        });
    }
    rename_row(app, ui, ed);
    let Some(worlds) = &ed.worlds else {
        ui.add(egui::Spinner::new().color(theme::ACCENT));
        return;
    };
    let running = app.running.contains_key(&ed.inst.id);
    let saves = ed.inst.game_dir().join("saves");
    ui.horizontal(|ui| {
        let count = format!("{} world{}", worlds.len(), if worlds.len() == 1 { "" } else { "s" });
        ui.label(RichText::new(count).color(theme::TEXT_MUTED));
        if running {
            ui.label(RichText::new("Close the game to change worlds.").color(theme::TEXT_MUTED));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::pill(ui, Kind::Ghost, None, "Open saves folder").clicked() {
                app.open_folder(&saves);
            }
            if widgets::pill(ui, Kind::Ghost, None, "Import world").clicked()
                && let Some(zip) = rfd::FileDialog::new().add_filter("World zip", &["zip"]).pick_file()
            {
                let saves = saves.clone();
                app.task("Importing world", move |_, bus| async move {
                    tokio::task::spawn_blocking(move || world::import_zip(&saves, &zip)).await??;
                    bus.send(Event::ContentChanged);
                    Ok(())
                });
            }
        });
    });
    ui.add_space(theme::S2);
    if worlds.is_empty() {
        widgets::empty_state(ui, "No worlds yet", "Worlds you create in game show up here.", |_| {});
        return;
    }
    let mut action = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
        for world in worlds {
            let (row, _) = widgets::list_row(ui, 52.0, Sense::hover(), false);
            if !ui.is_rect_visible(row) {
                continue;
            }
            let buttons =
                Rect::from_min_max(Pos2::new(row.right() - WORLD_BUTTONS, row.top()), Pos2::new(row.right() - theme::S3, row.bottom()));
            let x = row.left() + theme::S3;
            let w = buttons.left() - x - theme::S2;
            let cut = widgets::line(ui, Pos2::new(x, row.top() + 8.0), &world.name, theme::body_strong(), theme::TEXT, w);
            widgets::line(ui, Pos2::new(x, row.top() + 27.0), &days_ago("Played", world.modified), theme::label(), theme::TEXT_MUTED, w);
            if cut {
                let text = Rect::from_min_max(row.min, Pos2::new(buttons.left(), row.bottom()));
                ui.interact(text, ui.id().with(("world", &world.path)), Sense::hover()).on_hover_text(&world.name);
            }
            let mut child = ui.new_child(UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)));
            child.add_enabled_ui(!running, |ui| {
                if widgets::icon_button(ui, Glyph::Trash, "Delete").clicked() {
                    action = Some(WorldAction::Delete(world.clone()));
                }
                if widgets::pill_sized(ui, Kind::Ghost, None, "Rename", theme::H_SM).clicked() {
                    action = Some(WorldAction::Rename(world.clone()));
                }
                if widgets::pill_sized(ui, Kind::Ghost, None, "Copy", theme::H_SM).clicked() {
                    action = Some(WorldAction::Copy(world.clone()));
                }
            });
            if widgets::pill_sized(&mut child, Kind::Ghost, None, "Open folder", theme::H_SM).clicked() {
                app.open(&world.path);
            }
            if widgets::pill_sized(&mut child, Kind::Ghost, None, "Data packs", theme::H_SM).clicked() {
                action = Some(WorldAction::DataPacks(world.clone()));
            }
        }
    });
    match action {
        Some(WorldAction::DataPacks(world)) => ed.open_data_packs(world),
        Some(WorldAction::Rename(world)) => {
            let name = world.name.clone();
            ed.renaming = Some((world, name));
        }
        Some(WorldAction::Delete(world)) => app.confirm = Some(Confirm::DeleteWorld(Box::new(world))),
        Some(WorldAction::Copy(world)) => {
            app.task(format!("Copying {}", world.name), move |_, bus| async move {
                tokio::task::spawn_blocking(move || world::duplicate(&world)).await??;
                bus.send(Event::ContentChanged);
                Ok(())
            });
        }
        None => {}
    }
}

fn rename_row(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    let Some((target, name)) = &mut ed.renaming else { return };
    let (mut save, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        ui.add(widgets::input(name, "World name").desired_width(260.0));
        save = ui.add_enabled_ui(!name.trim().is_empty(), |ui| widgets::pill(ui, Kind::Secondary, None, "Rename")).inner.clicked();
        cancel = widgets::pill(ui, Kind::Ghost, None, "Cancel").clicked();
    });
    ui.add_space(theme::S2);
    if save {
        if let Err(e) = world::set_level_name(&target.path, name.trim()) {
            app.errors.push(format!("Could not rename {}: {e:#}", target.name));
        }
        ed.invalidate();
    }
    if save || cancel {
        ed.renaming = None;
    }
}

fn data_packs(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor, world: &World) {
    if !ed.browse
        && let Some(request) = ed.start_read()
    {
        let (id, path, bus) = (ed.inst.id.clone(), world.path.clone(), app.bus.clone());
        app.rt.spawn_blocking(move || {
            let files = modrinth::list_data_packs(&path);
            bus.send(Event::Files { inst: id, request, files });
        });
    }
    let mut back = false;
    ui.horizontal(|ui| {
        back = widgets::pill(ui, Kind::Ghost, None, "Back to worlds").clicked();
        let installed = ed.files.as_ref().map_or("Installed".to_string(), |f| format!("Installed ({})", f.len()));
        let mut browse = ed.browse;
        widgets::segmented(ui, &mut browse, &[(false, installed.as_str()), (true, "Browse Modrinth")]);
        ed.browse = browse;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !ed.browse
                && widgets::pill(ui, Kind::Ghost, None, "Add files").clicked()
                && let Some(files) = rfd::FileDialog::new().add_filter("Data packs", ContentKind::DataPack.extensions()).pick_files()
            {
                add_files(app, ed, ContentKind::DataPack, files);
            }
        });
    });
    ui.add_space(theme::S3);
    let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        add_files(app, ed, ContentKind::DataPack, dropped);
    }
    if ed.browse {
        browse(app, ui, ed, ContentKind::DataPack);
    } else {
        installed(app, ui, ed, ContentKind::DataPack);
    }
    if back {
        ed.close_data_packs();
    }
}
