// SPDX-License-Identifier: GPL-3.0-only
//! Modrinth search: the results grid and project details shared by the Browse tab and the Modpacks
//! page. Details open in a popup, so the results keep the whole page.

use std::collections::HashSet;

use eframe::egui::{self, RichText, Sense, Ui, vec2};

use crate::modplatform::modrinth::{SearchFilter, SearchHit, SearchResponse, Sort, Version, category_label};
use crate::ui::icons::Icons;
use crate::ui::theme;
use crate::ui::widgets::{self, CardMark, Kind};

/// Results per request. Modrinth's default, and what the pager steps by.
const PAGE_SIZE: u64 = 20;
const CARD_MIN_WIDTH: f32 = 250.0;
const CARD_GAP: f32 = theme::S2;
const DETAIL_DIALOG_WIDTH: f32 = 520.0;
const DESCRIPTION_HEIGHT: f32 = 170.0;
pub const INSTALL_WIDTH: f32 = 72.0;
const FILTER_WIDTH: f32 = 112.0;
const NOTE_WIDTH: f32 = 120.0;

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
    pub filter: SearchFilter,
    /// Results can be picked with a box, for installing together.
    pub selectable: bool,
    /// Project IDs already in the instance.
    pub installed: HashSet<String>,
    /// Project IDs picked for installing.
    pub picked: HashSet<String>,
    /// Names of the categories that can be picked.
    pub categories: Vec<String>,
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
    Toggle(SearchHit),
}

fn subtitle(hit: &SearchHit) -> String {
    format!("by {}  ·  {} downloads", hit.author, widgets::compact(hit.downloads))
}

fn icon(icons: &Icons, hit: &SearchHit) -> Option<egui::TextureId> {
    icons.get_opt(hit.icon_url.as_deref().filter(|u| !u.is_empty()))
}

/// How many cards fit across `width`.
fn columns(width: f32) -> usize {
    (((width + CARD_GAP) / (CARD_MIN_WIDTH + CARD_GAP)).floor() as usize).max(1)
}

/// The search box and results, filling `ui`, and the open project's popup. `footer` draws the
/// install controls under a project with versions.
pub fn show(
    ui: &mut Ui,
    icons: &Icons,
    b: &mut Browser,
    hint: &str,
    note: &str,
    footer: impl FnOnce(&mut Ui, &[Version], &mut usize),
) -> Option<Request> {
    let mut request = None;
    let before = b.filter.clone();
    ui.horizontal(|ui| {
        let width = (ui.available_width() * 0.32).clamp(150.0, 300.0);
        let r = widgets::search(ui, &mut b.query, hint, width);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            request = Some(Request::Search(0));
        }
        if widgets::pill(ui, Kind::Secondary, None, "Search").clicked() {
            request = Some(Request::Search(0));
        }
        widgets::combo("project-sort", b.filter.sort.label(), FILTER_WIDTH).show_ui(ui, |ui| {
            for sort in Sort::ALL {
                ui.selectable_value(&mut b.filter.sort, sort, sort.label());
            }
        });
        if !b.categories.is_empty() {
            let selected = b.filter.category.as_deref().map_or("All categories".to_string(), category_label);
            widgets::combo("project-category", selected, FILTER_WIDTH).show_ui(ui, |ui| {
                ui.selectable_value(&mut b.filter.category, None, "All categories");
                for name in &b.categories {
                    ui.selectable_value(&mut b.filter.category, Some(name.clone()), category_label(name));
                }
            });
        }
        if b.searching {
            ui.add(egui::Spinner::new().color(theme::ACCENT));
        }
        // Left out where the window is too narrow for it; the list is already filtered.
        if ui.available_width() >= NOTE_WIDTH {
            ui.label(RichText::new(note).font(theme::label()).color(theme::TEXT_MUTED));
        }
    });
    if b.filter != before {
        request = Some(Request::Search(0));
    }
    ui.add_space(theme::S3);

    list(ui, icons, b, &mut request);
    detail(ui, icons, b, footer, &mut request);
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
        let width = ui.available_width();
        let columns = columns(width);
        let card_width = (width - CARD_GAP * (columns - 1) as f32) / columns as f32;
        for row in b.results.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for hit in row {
                    let mark = match (b.selectable, b.installed.contains(&hit.project_id)) {
                        (_, true) => CardMark::Installed,
                        (true, false) => CardMark::Check(b.picked.contains(&hit.project_id)),
                        (false, false) => CardMark::None,
                    };
                    let (card, check) =
                        widgets::project_card(ui, card_width, icon(icons, hit), &hit.title, &subtitle(hit), &hit.description, mark);
                    if check.is_some_and(|c| c.clicked()) {
                        *request = Some(Request::Toggle(hit.clone()));
                    } else if card.clicked() {
                        *request = Some(Request::Versions(hit.clone()));
                    }
                }
            });
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
    footer: impl FnOnce(&mut Ui, &[Version], &mut usize),
    request: &mut Option<Request>,
) {
    let Some(hit) = b.hit.clone() else { return };
    let (_, closed) = widgets::dialog(ui.ctx(), "project-details", &hit.title, DETAIL_DIALOG_WIDTH, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            widgets::icon(ui, rect, icon(icons, &hit), &hit.title, theme::TEXT_MUTED);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(3.0);
                widgets::line_tip(ui, &subtitle(&hit), theme::label(), theme::TEXT_MUTED, 18.0);
                if b.installed.contains(&hit.project_id) {
                    widgets::line_tip(ui, "Installed in this instance", theme::label(), theme::OK, 18.0);
                }
            });
        });
        ui.add_space(theme::S2);
        egui::ScrollArea::vertical().id_salt("detail").max_height(DESCRIPTION_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
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
    if closed {
        b.hit = None;
    }
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
    fn the_grid_fits_the_width() {
        assert_eq!(columns(200.0), 1);
        assert_eq!(columns(508.0), 2);
        assert_eq!(columns(516.0), 2);
        assert_eq!(columns(780.0), 3);
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
