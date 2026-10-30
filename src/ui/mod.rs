// SPDX-License-Identifier: GPL-3.0-only
//! egui front end. Background work runs on a small tokio runtime and reports back through
//! [`Event`]s; the UI only repaints when an event or user input arrives.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

use anyhow::Context as _;
use eframe::egui;

use crate::config::Config;
use crate::java::{self, JavaInstall};
use crate::minecraft::auth::{self, AccountKind, AccountStore, DeviceCode};
use crate::minecraft::instance::{self, Instance, Loader};
use crate::minecraft::meta::mojang::ManifestEntry;
use crate::tasks::Progress;
use crate::updater;

mod activity;
mod chrome;
mod dialogs;
mod events;
mod game;
mod glyph;
mod icons;
mod jobs;
mod launcher_update;
mod pages;
mod theme;
mod widgets;

pub use events::{Bus, Event};
use game::Running;
pub use launcher_update::LauncherUpdate;
use pages::{accounts, console, editor, main_view, new_instance, project, settings};

/// The page shown next to the rail. An open instance page (`App::editor`) sits on top of Library.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Library,
    New,
    Accounts,
    Settings,
}

pub struct SkinTex {
    pub front: egui::TextureHandle,
    pub head: egui::TextureHandle,
}

pub struct App {
    rt: tokio::runtime::Handle,
    bus: Bus,
    rx: mpsc::Receiver<Event>,
    cfg: Config,
    accounts: AccountStore,
    instances: Vec<Instance>,
    selected: Option<String>,
    filter: String,
    tasks: Vec<Progress>,
    running: HashMap<String, Running>,
    consoles: HashMap<String, console::Console>,
    skins: HashMap<String, SkinTex>,
    icons: icons::Icons,
    errors: Vec<String>,
    toast: Option<main_view::Toast>,
    library_filter: main_view::Filter,
    manifest: Option<Vec<ManifestEntry>>,
    manifest_requested: bool,
    manifest_failed: bool,
    loader_versions: HashMap<(Loader, String), Vec<String>>,
    loader_requested: HashSet<(Loader, String)>,
    loader_failed: HashSet<(Loader, String)>,
    java_list: Vec<JavaInstall>,
    device_code: Option<DeviceCode>,
    login_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    page: Page,
    new_instance: new_instance::NewInstance,
    editor: Option<editor::Editor>,
    accounts_ui: accounts::AccountsUi,
    settings_draft: Option<Config>,
    confirm: Option<dialogs::Confirm>,
    deleting: HashSet<String>,
    launcher_update: LauncherUpdate,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, rt: tokio::runtime::Handle) -> Self {
        theme::apply(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            rt,
            bus: Bus { tx, ctx: cc.egui_ctx.clone() },
            rx,
            cfg: Config::load(),
            accounts: AccountStore::load(),
            instances: instance::list(),
            selected: None,
            filter: String::new(),
            tasks: Vec::new(),
            running: HashMap::new(),
            consoles: HashMap::new(),
            skins: HashMap::new(),
            icons: Default::default(),
            errors: Vec::new(),
            toast: None,
            library_filter: Default::default(),
            manifest: None,
            manifest_requested: false,
            manifest_failed: false,
            loader_versions: HashMap::new(),
            loader_requested: HashSet::new(),
            loader_failed: HashSet::new(),
            java_list: Vec::new(),
            device_code: None,
            login_cancel: None,
            page: Page::Library,
            new_instance: Default::default(),
            editor: None,
            accounts_ui: Default::default(),
            settings_draft: None,
            confirm: None,
            deleting: HashSet::new(),
            launcher_update: Default::default(),
        };
        app.selected = app.instances.first().map(|i| i.id.clone());
        app.startup();
        app
    }

    fn startup(&mut self) {
        self.rt.spawn_blocking(crate::paths::clean_cache);
        self.rt.spawn_blocking(updater::clean_old);
        if self.cfg.check_updates {
            self.check_launcher_update(false);
        }
        for account in self.accounts.accounts.clone() {
            self.load_skin(&account);
        }
        // Silent sign-in refresh so launching is instant; failures show as a toast.
        if let Some(account) = self.accounts.selected().cloned()
            && account.kind == AccountKind::Microsoft
        {
            let bus = self.bus.clone();
            self.rt.spawn(async move {
                match auth::refresh(&account).await {
                    Ok(a) => bus.send(Event::Account(a)),
                    Err(e) => {
                        let message = format!("Not signed in: {e:#}");
                        crate::log::error(&message);
                        bus.send(Event::Warning(message));
                    }
                }
            });
        }
        self.refresh_java();
    }

    fn open(&mut self, target: impl AsRef<std::ffi::OsStr>) {
        let target = target.as_ref();
        if let Err(e) = open::that(target) {
            self.errors.push(format!("Could not open {}: {e}", target.to_string_lossy()));
        }
    }

    /// Like [`Self::open`], for a folder the game may not have created yet.
    fn open_folder(&mut self, dir: &std::path::Path) {
        match std::fs::create_dir_all(dir) {
            Ok(()) => self.open(dir),
            Err(e) => self.errors.push(format!("Could not create {}: {e}", dir.display())),
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.toast = Some(main_view::Toast::new(text, false));
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.toast = Some(main_view::Toast::new(text, true));
    }

    /// Switches the main area to `page`. An instance page with unsaved edits asks first.
    fn go(&mut self, page: Page) {
        if self.editor.as_ref().is_some_and(editor::Editor::has_changes) {
            self.confirm = Some(dialogs::Confirm::Discard(page));
        } else {
            self.go_now(page);
        }
    }

    fn go_now(&mut self, page: Page) {
        self.editor = None;
        if page == Page::Settings && self.settings_draft.is_none() {
            self.settings_draft = Some(self.cfg.clone());
        }
        self.page = page;
    }

    /// Opens the instance page on `tab`, keeping the open page (and its unsaved edits) if it is
    /// already this instance.
    fn open_editor(&mut self, id: &str, tab: editor::Tab) {
        match &mut self.editor {
            Some(ed) if ed.instance_id() == id => ed.set_tab(tab),
            _ => {
                if let Some(inst) = self.instance(id) {
                    self.editor = Some(editor::Editor::new(inst.clone(), tab));
                }
            }
        }
        self.selected = Some(id.to_string());
        self.page = Page::Library;
    }

    fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.id == id)
    }

    fn reload_instances(&mut self, select: Option<String>) {
        self.instances = instance::list();
        self.instances.retain(|i| !self.deleting.contains(&i.id));
        if select.is_some() {
            self.selected = select;
        }
        if self.selected.as_ref().is_none_or(|s| self.instance(s).is_none()) {
            self.selected = self.instances.first().map(|i| i.id.clone());
        }
    }

    /// Takes the instance off the list at once and deletes it in the background: a big one can
    /// take minutes, and it must not be launched meanwhile.
    fn delete_instance(&mut self, inst: Instance) {
        self.deleting.insert(inst.id.clone());
        self.reload_instances(None);
        self.task(format!("Deleting {}", inst.name), move |_, bus| async move {
            let (id, name) = (inst.id.clone(), inst.name.clone());
            let deleted = tokio::task::spawn_blocking(move || inst.delete()).await;
            bus.send(Event::InstanceDeleted(id));
            deleted?.with_context(|| format!("Could not delete {name}"))
        });
    }

    fn refresh_java(&self) {
        let bus = self.bus.clone();
        self.rt.spawn_blocking(move || bus.send(Event::Java(java::detect())));
    }

    fn save_accounts(&mut self) {
        if let Err(e) = self.accounts.save() {
            self.errors.push(format!("Saving accounts failed: {e:#}"));
        }
    }

    fn errors_window(&mut self, ctx: &egui::Context) {
        let Some(message) = self.errors.first().cloned() else { return };
        let mut dismiss = false;
        let more = self.errors.len() - 1;
        let (_, closed) = widgets::dialog(ctx, "error", "Something went wrong", 460.0, |ui| {
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                ui.label(egui::RichText::new(&message).color(theme::TEXT));
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if more > 0 {
                    ui.label(egui::RichText::new(format!("{more} more")).color(theme::TEXT_MUTED));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::pill(ui, widgets::Kind::Primary, None, "OK").clicked() {
                        dismiss = true;
                    }
                    if widgets::pill(ui, widgets::Kind::Secondary, None, "Copy").clicked() {
                        ui.ctx().copy_text(message.clone());
                    }
                });
            });
        });
        if dismiss || closed {
            self.errors.remove(0);
        }
    }
}

impl eframe::App for App {
    /// egui keeps its font atlas in RAM as 4-byte coverage values, as wide as the GPU allows
    /// (8192+ px on Intel HD graphics). 2048 px is plenty for the launcher's text and cuts that
    /// buffer to a quarter.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.max_texture_side = Some(raw_input.max_texture_side.map_or(2048, |side| side.min(2048)));
    }

    fn update(&mut self, ctx: &egui::Context, #[cfg_attr(not(windows), allow(unused))] frame: &mut eframe::Frame) {
        #[cfg(windows)]
        chrome::fix_window_style(frame);
        self.process_events(ctx);
        // Running tasks stay; of the finished ones only the newest few are kept for the Recent list.
        let finished = self.tasks.iter().filter(|t| t.snapshot().finished()).count();
        let mut drop = finished.saturating_sub(activity::KEEP_FINISHED);
        self.tasks.retain(|t| {
            let old = drop > 0 && t.snapshot().finished();
            drop -= usize::from(old);
            !old
        });

        main_view::show(self, ctx);
        accounts::device_code_window(self, ctx);
        dialogs::window(self, ctx);
        self.errors_window(ctx);
        chrome::frame_edges(ctx);
        self.fetch_wanted_icons();
    }
}
