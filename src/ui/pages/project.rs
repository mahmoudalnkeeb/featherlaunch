// SPDX-License-Identifier: GPL-3.0-only
//! Modrinth search: the results list and project detail shared by the Browse tab and the Modpacks
//! page. Results sit on the left and the selected project on the right, or one at a time when the
//! window is too narrow for both.

use eframe::egui::{self, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};

use crate::modplatform::modrinth::{SearchHit, SearchResponse, Version};
use crate::ui::icons::Icons;
use crate::ui::theme;
use crate::ui::widgets::{self, Kind};

/// Results per request. Modrinth's default, and what the pager steps by.
const PAGE_SIZE: u64 = 20;
const DETAIL_WIDTH: f32 = 300.0;
const TWO_PANES: f32 = 640.0;
pub const INSTALL_WIDTH: f32 = 72.0;

#[derive(Default)]
pub struct Browser {
    pub query: String,
    pub searching: bool,
    pub search_failed: bool,
    pub results: Vec<SearchHit>,
    pub total: u64,
    pub offset: u64,
    pub hit: Option<SearchHit>,
    pub versions: Option<Vec<Version>>,
    pub versions_failed: bool,
    pub version_idx: usize,
}

impl Browser {
    pub fn on_search(&mut self, offset: u64, results: SearchResponse) {
        self.searching = false;
        self.search_failed = false;
        self.offset = offset;
        self.total = results.total_hits;
        self.results = results.hits;
    }

    pub fn on_search_failed(&mut self) {
        self.searching = false;
        self.search_failed = true;
    }

    pub fn on_versions(&mut self, project: &str, versions: &[Version]) {
        if self.is_open(project) {
            self.versions = Some(versions.to_vec());
            self.versions_failed = false;
            self.version_idx = 0;
        }
    }

    pub fn on_versions_failed(&mut self, project: &str) {
        if self.is_open(project) {
            self.versions_failed = true;
        }
    }

    pub fn clear(&mut self) {
        *self = Self { query: std::mem::take(&mut self.query), ..Default::default() };
    }

    fn is_open(&self, project: &str) -> bool {
        self.hit.as_ref().is_some_and(|h| h.project_id == project)
    }

    /// Marks the start of a search; the caller sends it.
    pub fn start_search(&mut self) {
        self.searching = true;
        self.search_failed = false;
    }

    /// Opens `hit`; the caller loads its versions.
    pub fn open(&mut self, hit: SearchHit) {
        self.hit = Some(hit);
        self.reload_versions();
    }

    /// Marks the versions as loading; the caller loads them.
    pub fn reload_versions(&mut self) {
        self.versions = None;
        self.versions_failed = false;
    }
}

pub enum Request {
    Search(u64),
    Versions(SearchHit),
}

fn subtitle(hit: &SearchHit) -> String {
    format!("by {}  ·  {} downloads", hit.author, widgets::compact(hit.downloads))
}

fn icon(icons: &Icons, hit: &SearchHit) -> Option<egui::TextureId> {
    icons.get_opt(hit.icon_url.as_deref().filter(|u| !u.is_empty()))
}

/// The search box, results and detail, filling `ui`. `footer` draws the install controls under a
/// project with versions, at `footer_height`, so they stay on screen however long the text is.
pub fn show(
    ui: &mut Ui,
    icons: &Icons,
    b: &mut Browser,
    hint: &str,
    note: &str,
    footer_height: f32,
    footer: impl FnOnce(&mut Ui, &[Version], &mut usize),
) -> Option<Request> {
    let mut request = None;
    let two_panes = ui.available_width() >= TWO_PANES;
    if !two_panes && b.hit.is_some() {
        if widgets::pill(ui, Kind::Ghost, None, "Back to results").clicked() {
            b.hit = None;
            return None;
        }
        ui.add_space(theme::S2);
        detail(ui, icons, b, footer_height, footer, &mut request);
        return request;
    }

    ui.horizontal(|ui| {
        let width = (ui.available_width() / 2.0).clamp(180.0, 300.0);
        let r = widgets::search(ui, &mut b.query, hint, width);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            request = Some(Request::Search(0));
        }
        if widgets::pill(ui, Kind::Secondary, None, "Search").clicked() {
            request = Some(Request::Search(0));
        }
        if b.searching {
            ui.add(egui::Spinner::new().color(theme::ACCENT));
        }
        ui.label(RichText::new(note).font(theme::label()).color(theme::TEXT_MUTED));
    });
    ui.add_space(theme::S3);

    let area = ui.available_rect_before_wrap();
    if !two_panes {
        list(ui, icons, b, &mut request);
        return request;
    }
    let split = area.right() - DETAIL_WIDTH;
    let left = Rect::from_min_max(area.min, egui::pos2(split - theme::S4, area.bottom()));
    let right = Rect::from_min_max(egui::pos2(split, area.top()), area.max);
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::top_down(egui::Align::LEFT)), |ui| {
        list(ui, icons, b, &mut request);
    });
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::top_down(egui::Align::LEFT)), |ui| {
        detail(ui, icons, b, footer_height, footer, &mut request);
    });
    request
}

fn list(ui: &mut Ui, icons: &Icons, b: &mut Browser, request: &mut Option<Request>) {
    let paged = b.total > PAGE_SIZE;
    let pager_height = if paged { theme::H_MD + theme::S2 } else { 0.0 };
    let height = (ui.available_height() - pager_height).max(0.0);
    egui::ScrollArea::vertical().id_salt("results").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = widgets::LIST_GAP;
        if b.search_failed {
            if widgets::notice(ui, theme::ERR, "Couldn't reach Modrinth.", Some("Try again")) {
                *request = Some(Request::Search(b.offset));
            }
            return;
        }
        if b.results.is_empty() {
            if !b.searching {
                widgets::empty_state(ui, "No results", "Try a different search.", |_| {});
            }
            return;
        }
        for hit in &b.results {
            let selected = b.hit.as_ref().is_some_and(|h| h.project_id == hit.project_id);
            if widgets::project_row(ui, icon(icons, hit), &hit.title, &subtitle(hit), &hit.description, selected).clicked() {
                *request = Some(Request::Versions(hit.clone()));
            }
        }
    });
    if paged {
        ui.add_space(theme::S2);
        ui.horizontal(|ui| {
            let previous = ui.add_enabled_ui(b.offset > 0, |ui| widgets::pill(ui, Kind::Secondary, None, "Previous"));
            if previous.inner.clicked() {
                *request = Some(Request::Search(b.offset.saturating_sub(PAGE_SIZE)));
            }
            let next = ui.add_enabled_ui(b.offset + PAGE_SIZE < b.total, |ui| widgets::pill(ui, Kind::Secondary, None, "Next"));
            if next.inner.clicked() {
                *request = Some(Request::Search(b.offset + PAGE_SIZE));
            }
            let range = format!("{}–{} of {}", b.offset + 1, (b.offset + PAGE_SIZE).min(b.total), b.total);
            ui.label(RichText::new(range).font(theme::label()).color(theme::TEXT_MUTED));
        });
    }
}

fn detail(
    ui: &mut Ui,
    icons: &Icons,
    b: &mut Browser,
    footer_height: f32,
    footer: impl FnOnce(&mut Ui, &[Version], &mut usize),
    request: &mut Option<Request>,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let Some(hit) = b.hit.clone() else {
            ui.label(RichText::new("Select a project to see its versions.").color(theme::TEXT_MUTED));
            return;
        };
        let height = (ui.available_height() - footer_height - theme::S3).max(48.0);
        egui::ScrollArea::vertical().id_salt("detail").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                widgets::icon(ui, rect, icon(icons, &hit), &hit.title, theme::TEXT_MUTED);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(3.0);
                    widgets::line_tip(ui, &hit.title, theme::title(), theme::TEXT, 22.0);
                    widgets::line_tip(ui, &subtitle(&hit), theme::label(), theme::TEXT_MUTED, 18.0);
                });
            });
            ui.add_space(theme::S2);
            ui.label(RichText::new(&hit.description).color(theme::TEXT));
        });
        ui.add_space(theme::S3);
        match &b.versions {
            None if b.versions_failed => {
                if widgets::notice(ui, theme::ERR, "Couldn't load the versions.", Some("Try again")) {
                    *request = Some(Request::Versions(hit.clone()));
                }
            }
            None => {
                ui.add(egui::Spinner::new().color(theme::ACCENT));
            }
            Some(versions) if versions.is_empty() => {
                ui.label(RichText::new("No version fits this instance.").color(theme::WARN));
            }
            Some(versions) => footer(ui, versions, &mut b.version_idx),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str) -> SearchHit {
        SearchHit {
            project_id: id.into(),
            title: id.into(),
            description: String::new(),
            author: String::new(),
            downloads: 0,
            icon_url: None,
        }
    }

    #[test]
    fn a_failed_search_stops_the_spinner_until_the_next_one() {
        let mut b = Browser::default();
        b.start_search();
        b.on_search_failed();
        assert!(!b.searching);
        assert!(b.search_failed);
        b.start_search();
        assert!(b.searching);
        assert!(!b.search_failed);
        b.on_search(0, SearchResponse { hits: vec![hit("a")], total_hits: 1 });
        assert!(!b.searching);
        assert_eq!(b.results.len(), 1);
    }

    #[test]
    fn versions_only_land_on_the_open_project() {
        let mut b = Browser::default();
        b.open(hit("a"));
        b.on_versions_failed("b");
        assert!(!b.versions_failed);
        b.on_versions("b", &[]);
        assert!(b.versions.is_none());
        b.on_versions_failed("a");
        assert!(b.versions_failed);
        b.on_versions("a", &[]);
        assert!(!b.versions_failed);
        assert!(b.versions.is_some());
    }

    #[test]
    fn clear_keeps_the_query() {
        let mut b = Browser { query: "sodium".into(), ..Default::default() };
        b.open(hit("a"));
        b.clear();
        assert_eq!(b.query, "sodium");
        assert!(b.hit.is_none());
    }
}
