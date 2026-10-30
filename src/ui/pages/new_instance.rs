// SPDX-License-Identifier: GPL-3.0-only
//! The "new instance" page: a custom version and loader, a Modrinth modpack, or a local .mrpack.

use eframe::egui::{self, CornerRadius, Layout, Pos2, Rect, RichText, Sense, UiBuilder, vec2};

use crate::minecraft::instance::{Instance, Loader};
use crate::modplatform::modrinth::{ContentKind, Version};
use crate::modplatform::mrpack;
use crate::ui::project::{self, Browser, Request};
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, Page, theme};

const SIDE_WIDTH: f32 = 240.0;
const PACK_FOOTER: f32 = 72.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Custom,
    Modpack,
    Import,
}

pub struct NewInstance {
    pub mode: Mode,
    name: String,
    name_edited: bool,
    group: String,
    mc: String,
    filter: String,
    snapshots: bool,
    loader: Loader,
    loader_version: Option<String>,
    pub browser: Browser,
}

impl Default for NewInstance {
    fn default() -> Self {
        Self {
            mode: Mode::Custom,
            name: String::new(),
            name_edited: false,
            group: String::new(),
            mc: String::new(),
            filter: String::new(),
            snapshots: false,
            loader: Loader::Vanilla,
            loader_version: None,
            browser: Browser::default(),
        }
    }
}

impl NewInstance {
    fn auto_name(&mut self) {
        if !self.name_edited {
            self.name = match self.loader {
                Loader::Vanilla => self.mc.clone(),
                l => format!("{} {}", self.mc, l.name()),
            };
        }
    }
}

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let mut st = std::mem::take(&mut app.new_instance);
    let subtitle = match st.mode {
        Mode::Modpack => "Install a modpack from Modrinth as a new instance.",
        Mode::Import => "Make an instance from a .mrpack file.",
        Mode::Custom => "Pick a Minecraft version and an optional mod loader.",
    };
    // No page scroll: both columns fit, and the lists inside them scroll.
    let created = widgets::page_frame()
        .show(ui, |ui| {
            widgets::page_header(ui, "New instance", subtitle, |_| {});
            ui.add_space(theme::S4);
            let mut mode = st.mode;
            let modes = [(Mode::Custom, "Custom version"), (Mode::Modpack, "Modrinth modpacks"), (Mode::Import, "Import file")];
            widgets::segmented(ui, &mut mode, &modes);
            st.mode = mode;
            ui.add_space(theme::S4);
            match st.mode {
                Mode::Modpack => modpacks(app, ui, &mut st),
                Mode::Import => import(app, ui, &mut st),
                Mode::Custom => custom(app, ui, &mut st),
            }
        })
        .inner;
    // Go back to the Library once an install starts, since it shows the progress, and reset the form.
    if created {
        app.go(Page::Library);
    } else {
        app.new_instance = st;
    }
}

fn well() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(egui::Margin::same(6))
}

fn stacked(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.spacing_mut().item_spacing.y = theme::S1;
    widgets::section(ui, label);
    add(ui);
    ui.add_space(theme::S2);
}

fn custom(app: &mut App, ui: &mut egui::Ui, st: &mut NewInstance) -> bool {
    app.request_manifest();
    // Preselect the newest release, since most people want it.
    if st.mc.is_empty()
        && let Some(latest) = app.manifest.iter().flatten().find(|v| v.kind == "release")
    {
        st.mc = latest.id.clone();
        st.auto_name();
    }
    let area = ui.available_rect_before_wrap();
    let split = area.right() - SIDE_WIDTH;
    let left = Rect::from_min_max(area.min, Pos2::new(split - theme::S6, area.bottom()));
    let right = Rect::from_min_max(Pos2::new(split, area.top()), area.max);
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::top_down(egui::Align::LEFT)), |ui| versions(app, ui, st));
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::top_down(egui::Align::LEFT)), |ui| details(app, ui, st)).inner
}

fn versions(app: &mut App, ui: &mut egui::Ui, st: &mut NewInstance) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 110.0).max(100.0);
        widgets::search(ui, &mut st.filter, "Filter versions", width);
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("Snapshots").font(theme::label()).color(theme::TEXT_MUTED));
            widgets::toggle(ui, &mut st.snapshots);
        });
    });
    ui.add_space(theme::S2);
    if app.manifest_failed {
        if widgets::notice(ui, theme::ERR, "Couldn't load the version list.", Some("Try again")) {
            app.manifest_requested = false;
        }
        return;
    }
    let mut picked = None;
    let height = (ui.available_height() - 14.0).max(60.0);
    well().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("versions").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(manifest) = &app.manifest else {
                ui.add(egui::Spinner::new().color(theme::ACCENT));
                return;
            };
            for v in manifest.iter().filter(|v| {
                (v.kind == "release" || (st.snapshots && v.kind == "snapshot")) && (st.filter.is_empty() || v.id.contains(&st.filter))
            }) {
                let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let selected = st.mc == v.id;
                if selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(theme::R_SM), theme::alpha(theme::ACCENT, 0.14));
                } else if r.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(theme::R_SM), theme::RAISED);
                }
                let y = rect.center().y;
                let painter = ui.painter();
                let id =
                    painter.text(Pos2::new(rect.left() + 10.0, y), egui::Align2::LEFT_CENTER, &v.id, theme::body_strong(), theme::TEXT);
                if v.kind == "snapshot" {
                    painter.text(
                        Pos2::new(id.right() + theme::S2, y),
                        egui::Align2::LEFT_CENTER,
                        "Snapshot",
                        theme::label(),
                        theme::TEXT_MUTED,
                    );
                }
                let date = v.release_time.get(..10).unwrap_or("");
                painter.text(Pos2::new(rect.right() - 10.0, y), egui::Align2::RIGHT_CENTER, date, theme::label(), theme::TEXT_MUTED);
                widgets::focus_ring(ui, rect.shrink(2.0), &r);
                if r.clicked() {
                    picked = Some(v.id.clone());
                }
            }
        });
    });
    if let Some(mc) = picked {
        st.mc = mc;
        st.loader_version = None;
        st.auto_name();
    }
}

fn details(app: &mut App, ui: &mut egui::Ui, st: &mut NewInstance) -> bool {
    stacked(ui, "Name", |ui| {
        if ui.add(widgets::input(&mut st.name, "Instance name").desired_width(ui.available_width())).changed() {
            st.name_edited = true;
        }
    });
    stacked(ui, "Group", |ui| {
        ui.add(widgets::input(&mut st.group, "Optional").desired_width(ui.available_width()));
    });
    stacked(ui, "Mod loader", |ui| {
        let mut loader = st.loader;
        widgets::combo("loader", loader.name(), ui.available_width()).show_ui(ui, |ui| {
            for l in Loader::ALL {
                ui.selectable_value(&mut loader, l, l.name());
            }
        });
        if loader != st.loader {
            st.loader = loader;
            st.loader_version = None;
            st.auto_name();
        }
    });
    if st.loader != Loader::Vanilla && !st.mc.is_empty() {
        app.request_loader_versions(st.loader, &st.mc);
        let key = (st.loader, st.mc.clone());
        stacked(ui, "Loader version", |ui| {
            if app.loader_failed.contains(&key) {
                if widgets::notice(ui, theme::ERR, "Couldn't load versions.", Some("Try again")) {
                    app.retry_loader_versions(key.0, &key.1);
                }
                return;
            }
            let versions = app.loader_versions.get(&key);
            match versions {
                None => {
                    ui.add(egui::Spinner::new().color(theme::ACCENT));
                }
                Some(v) if v.is_empty() => {
                    ui.label(
                        RichText::new(format!("{} has no build for {}", st.loader.name(), st.mc)).font(theme::label()).color(theme::WARN),
                    );
                }
                Some(v) => {
                    let text = st.loader_version.clone().unwrap_or_else(|| "Latest".into());
                    widgets::combo("loader-version", text, ui.available_width()).show_ui(ui, |ui| {
                        ui.selectable_value(&mut st.loader_version, None, "Latest");
                        for version in v {
                            ui.selectable_value(&mut st.loader_version, Some(version.clone()), version);
                        }
                    });
                }
            }
        });
    }
    ui.add_space(theme::S1);
    let can_create = !st.mc.is_empty() && !st.name.trim().is_empty();
    let mut close = false;
    let create = ui.add_enabled_ui(can_create, |ui| widgets::pill(ui, Kind::Primary, None, "Create"));
    if create.inner.clicked() {
        let inst = Instance {
            name: st.name.trim().to_string(),
            group: st.group.trim().to_string(),
            mc_version: st.mc.clone(),
            loader: st.loader,
            loader_version: st.loader_version.clone(),
            ..Default::default()
        };
        match inst.create() {
            Ok(i) => {
                app.reload_instances(Some(i.id));
                close = true;
            }
            Err(e) => app.errors.push(format!("Could not create the instance: {e:#}")),
        }
    }
    ui.add_space(theme::S1);
    widgets::hint(ui, "Game files download on first launch.");
    close
}

fn modpacks(app: &mut App, ui: &mut egui::Ui, st: &mut NewInstance) -> bool {
    let browser = &mut st.browser;
    let group = &mut st.group;
    let first_search = browser.results.is_empty() && !browser.searching && !browser.search_failed && browser.query.is_empty();
    let mut install = None;
    let footer = |ui: &mut egui::Ui, versions: &[Version], idx: &mut usize| {
        *idx = (*idx).min(versions.len() - 1);
        let label = |v: &Version| format!("{} ({})", v.version_number, v.game_versions.join(", "));
        widgets::combo_cut(ui, "pack-version", label(&versions[*idx]), ui.available_width(), |ui| {
            for (i, v) in versions.iter().enumerate() {
                ui.selectable_value(idx, i, label(v));
            }
        });
        ui.horizontal(|ui| {
            let width = ui.available_width() - project::INSTALL_WIDTH - theme::S2;
            ui.add(widgets::input(group, "Group (optional)").desired_width(width - widgets::INPUT_MARGIN));
            if widgets::pill(ui, Kind::Primary, None, "Install").clicked() {
                install = Some(versions[*idx].clone());
            }
        });
    };
    let mut request = project::show(ui, &app.icons, browser, "Search modpacks", "", PACK_FOOTER, footer);
    if first_search {
        request = Some(Request::Search(0));
    }
    match request {
        Some(Request::Search(offset)) => {
            st.browser.start_search();
            app.search_modrinth(st.browser.query.clone(), ContentKind::Modpack, None, offset);
        }
        Some(Request::Versions(hit)) => {
            let project = hit.project_id.clone();
            st.browser.open(hit);
            app.load_project_versions(project, ContentKind::Modpack, None);
        }
        None => {}
    }
    let (Some(version), Some(pack)) = (install, st.browser.hit.clone()) else { return false };
    let group = st.group.trim().to_string();
    let parallel = app.cfg.parallel_downloads;
    app.cancellable_task(format!("Installing {}", pack.title), move |progress, bus| async move {
        let inst = mrpack::install_from_modrinth(&version, &pack.title, pack.icon_url.as_deref(), group, parallel, &progress).await?;
        bus.send(crate::ui::Event::InstancesChanged(Some(inst.id)));
        Ok(())
    });
    true
}

fn import(app: &mut App, ui: &mut egui::Ui, st: &mut NewInstance) -> bool {
    let mut close = false;
    widgets::form(ui, |ui| {
        ui.label(RichText::new("A .mrpack file exported from Modrinth, Prism or another launcher.").color(theme::TEXT_MUTED));
        ui.add_space(theme::S3);
        widgets::form_row(ui, "Group", |ui| {
            ui.add(widgets::input(&mut st.group, "Optional").desired_width(ui.available_width()));
        });
        ui.add_space(theme::S2);
        if widgets::pill(ui, Kind::Primary, None, "Choose .mrpack file…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Modrinth modpack", &["mrpack"]).pick_file()
        {
            let group = st.group.trim().to_string();
            let parallel = app.cfg.parallel_downloads;
            app.cancellable_task("Importing modpack", move |progress, bus| async move {
                let inst = mrpack::import(path, group, parallel, &progress).await?;
                bus.send(crate::ui::Event::InstancesChanged(Some(inst.id)));
                Ok(())
            });
            close = true;
        }
    });
    close
}
