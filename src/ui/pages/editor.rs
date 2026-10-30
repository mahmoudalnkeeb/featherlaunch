// SPDX-License-Identifier: GPL-3.0-only
//! Instance page: mods / resource packs / shader packs (installed + Modrinth), worlds, log, settings.
//! It replaces the library in the main area instead of floating in a window.

use eframe::egui::{self, Pos2, Rect, RichText, Sense, UiBuilder, vec2};

use crate::minecraft::instance::{Instance, Loader, World};
use crate::modplatform::modrinth::{self, ContentKind, LocalFile, SearchResponse, Update};
use crate::ui::dialogs::Confirm;
use crate::ui::glyph::Glyph;
use crate::ui::project::{self, Browser, Request};
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, Event, Page, console, main_view, theme};

const FOOTER_HEIGHT: f32 = 56.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Content(ContentKind),
    Worlds,
    Log,
    Settings,
}

impl Tab {
    pub fn first_for(inst: &Instance) -> Self {
        if inst.loader == Loader::Vanilla { Tab::Worlds } else { Tab::Content(ContentKind::Mod) }
    }
}

pub struct Editor {
    inst: Instance,
    draft: Instance,
    tab: Tab,
    files: Option<Vec<LocalFile>>,
    worlds: Option<Vec<World>>,
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
    /// A text field or popup was active last frame, so it handles Esc. This reads last frame's
    /// state because egui clears focus on Esc before the frame starts.
    esc_taken: bool,
}

impl Editor {
    pub fn new(inst: Instance, tab: Tab) -> Self {
        Self {
            memory_override: inst.memory_mb.is_some(),
            jvm_override: inst.jvm_args.is_some(),
            draft: inst.clone(),
            inst,
            tab,
            files: None,
            worlds: None,
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

    pub fn set_icon_stamp(&mut self, stamp: Option<u128>) {
        self.inst.icon_stamp = stamp;
        self.draft.icon_stamp = stamp;
    }

    pub fn on_search(&mut self, kind: ContentKind, offset: u64, results: SearchResponse) {
        if self.tab == Tab::Content(kind) {
            self.browser.on_search(offset, results);
        }
    }

    pub fn on_search_failed(&mut self, kind: ContentKind) {
        if self.tab == Tab::Content(kind) {
            self.browser.on_search_failed();
        }
    }

    pub fn on_updates(&mut self, updates: Vec<Update>) {
        self.checking = false;
        self.updates = updates;
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.files = None;
            self.worlds = None;
            self.stale = true;
            self.updates.clear();
            self.browser.clear();
            self.searched_kind = None;
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
        ];
        if widgets::tabs(ui, &mut tab, &tabs) {
            ed.set_tab(tab);
        }
        ui.add_space(theme::S4);
        match ed.tab {
            Tab::Settings => settings(app, ui, &mut ed),
            Tab::Content(kind) => content(app, ui, &mut ed, kind),
            Tab::Worlds => worlds(app, ui, &mut ed),
            Tab::Log => {
                let id = ed.inst.id.clone();
                let action = app.consoles.entry(id.clone()).or_default().ui(ui);
                console::handle(app, &id, action);
            }
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
                    for v in app.manifest.iter().flatten().filter(|v| v.kind == "release" || v.id == current) {
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
                update_buttons(app, ui, ed, kind);
            }
        });
    });
    ui.add_space(theme::S3);
    if ed.browse {
        browse(app, ui, ed, kind);
    } else {
        installed(app, ui, ed, kind);
    }
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
            widgets::empty_state(ui, "Nothing installed yet", "Browse Modrinth, or drop files into the folder.", |ui| {
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
        let (inst, meta) = (ed.inst.clone(), hit.meta());
        app.cancellable_task(format!("Installing {}", meta.title), move |progress, bus| async move {
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
fn days_ago(modified: Option<std::time::SystemTime>) -> String {
    modified
        .and_then(|m| m.elapsed().ok())
        .map(|d| match d.as_secs() / 86_400 {
            0 => "Played today".to_string(),
            1 => "Played yesterday".to_string(),
            n => format!("Played {n} days ago"),
        })
        .unwrap_or_default()
}

fn worlds(app: &mut App, ui: &mut egui::Ui, ed: &mut Editor) {
    if let Some(request) = ed.start_read() {
        let (inst, bus) = (ed.inst.clone(), app.bus.clone());
        app.rt.spawn_blocking(move || {
            let worlds = inst.worlds();
            bus.send(Event::Worlds { inst: inst.id, request, worlds });
        });
    }
    let Some(worlds) = &ed.worlds else {
        ui.add(egui::Spinner::new().color(theme::ACCENT));
        return;
    };
    ui.horizontal(|ui| {
        let count = format!("{} world{}", worlds.len(), if worlds.len() == 1 { "" } else { "s" });
        ui.label(RichText::new(count).color(theme::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::pill(ui, Kind::Ghost, None, "Open saves folder").clicked() {
                app.open_folder(&ed.inst.game_dir().join("saves"));
            }
        });
    });
    ui.add_space(theme::S2);
    if worlds.is_empty() {
        widgets::empty_state(ui, "No worlds yet", "Worlds you create in game show up here.", |_| {});
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
        for world in worlds {
            let (row, _) = widgets::list_row(ui, 52.0, Sense::hover(), false);
            if !ui.is_rect_visible(row) {
                continue;
            }
            let button = Rect::from_min_size(row.right_top() - vec2(112.0, -12.0), vec2(100.0, theme::H_SM));
            let x = row.left() + theme::S3;
            let w = button.left() - x - theme::S2;
            let cut = widgets::line(ui, Pos2::new(x, row.top() + 8.0), &world.name, theme::body_strong(), theme::TEXT, w);
            widgets::line(ui, Pos2::new(x, row.top() + 27.0), &days_ago(world.modified), theme::label(), theme::TEXT_MUTED, w);
            if cut {
                let text = Rect::from_min_max(row.min, Pos2::new(button.left(), row.bottom()));
                ui.interact(text, ui.id().with(("world", &world.name)), Sense::hover()).on_hover_text(&world.name);
            }
            let mut child = ui.new_child(UiBuilder::new().max_rect(button).layout(egui::Layout::right_to_left(egui::Align::Center)));
            if widgets::pill_sized(&mut child, Kind::Ghost, None, "Open folder", theme::H_SM).clicked() {
                app.open(&world.path);
            }
        }
    });
}
