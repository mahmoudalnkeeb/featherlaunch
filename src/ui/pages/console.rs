// SPDX-License-Identifier: GPL-3.0-only
//! Game log viewer: a capped ring buffer drawn with virtual scrolling, so long sessions stay cheap.

use std::collections::VecDeque;

use eframe::egui::{self, RichText};

use crate::minecraft::launch::{Level, LogLine};
use crate::ui::widgets::{self, Kind};
use crate::ui::{App, theme};

const MAX_LINES: usize = 10_000;

pub struct Console {
    lines: VecDeque<LogLine>,
    /// Sequence number of `lines[0]`. Lines keep their number as older ones are dropped.
    first: u64,
    filter: String,
    follow: bool,
    /// Lines (by sequence number) matching `matched_filter`, checked up to `matched_to`. Each
    /// frame only checks new lines, instead of lowercasing the whole log again while it streams.
    matches: VecDeque<u64>,
    matched_filter: String,
    matched_to: u64,
}

impl Default for Console {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            first: 0,
            filter: String::new(),
            follow: true,
            matches: VecDeque::new(),
            matched_filter: String::new(),
            matched_to: 0,
        }
    }
}

pub enum Action {
    None,
    OpenCrashReports,
}

impl Console {
    pub fn push(&mut self, line: LogLine) {
        for part in line.text.split('\n') {
            if self.lines.len() >= MAX_LINES {
                self.lines.pop_front();
                self.first += 1;
            }
            self.lines.push_back(LogLine { level: line.level, text: part.to_string() });
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Action {
        let mut action = Action::None;
        ui.horizontal(|ui| {
            widgets::search(ui, &mut self.filter, "Filter log", 200.0);
            ui.add_space(theme::S2);
            widgets::toggle_labeled(ui, &mut self.follow, "Follow");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::pill(ui, Kind::Ghost, None, "Clear").clicked() {
                    self.first += self.lines.len() as u64;
                    self.lines.clear();
                }
                if widgets::pill(ui, Kind::Ghost, None, "Copy all").clicked() {
                    let text: Vec<&str> = self.lines.iter().map(|l| l.text.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
                if widgets::pill(ui, Kind::Ghost, None, "Crash reports").clicked() {
                    action = Action::OpenCrashReports;
                }
            });
        });
        ui.add_space(8.0);

        let filtered = !self.filter.is_empty();
        if filtered {
            self.update_matches();
        }
        let count = if filtered { self.matches.len() } else { self.lines.len() };
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let frame = egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(theme::R_LG))
            .inner_margin(egui::Margin::same(theme::S3 as i8));
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if count == 0 {
                ui.label(RichText::new("No log output yet. Start the game to see its log here.").color(theme::TEXT_MUTED));
            }
            egui::ScrollArea::both().auto_shrink([false; 2]).stick_to_bottom(self.follow).show_rows(ui, row_height, count, |ui, range| {
                for row in range {
                    let i = if filtered { (self.matches[row] - self.first) as usize } else { row };
                    let line = &self.lines[i];
                    let color = match line.level {
                        Level::Error => theme::ERR,
                        Level::Warn => theme::WARN,
                        Level::Debug => theme::TEXT_MUTED,
                        Level::Info => theme::TEXT,
                    };
                    ui.add(egui::Label::new(RichText::new(&line.text).monospace().color(color)).extend().selectable(true));
                }
            });
        });
        action
    }

    fn update_matches(&mut self) {
        let filter = self.filter.to_lowercase();
        if filter != self.matched_filter {
            self.matches.clear();
            self.matched_filter = filter;
            self.matched_to = self.first;
        }
        while self.matches.front().is_some_and(|&seq| seq < self.first) {
            self.matches.pop_front();
        }
        let end = self.first + self.lines.len() as u64;
        for seq in self.matched_to.max(self.first)..end {
            if self.lines[(seq - self.first) as usize].text.to_lowercase().contains(&self.matched_filter) {
                self.matches.push_back(seq);
            }
        }
        self.matched_to = end;
    }
}

pub fn handle(app: &mut App, inst_id: &str, action: Action) {
    match action {
        Action::None => {}
        Action::OpenCrashReports => {
            if let Some(inst) = app.instance(inst_id) {
                match inst.latest_crash_report() {
                    Some(file) => app.open(file),
                    None => app.notify("No crash reports for this instance"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> LogLine {
        LogLine { level: Level::Info, text: text.into() }
    }

    #[test]
    fn filter_follows_new_and_dropped_lines() {
        let mut c = Console::default();
        for i in 0..MAX_LINES {
            c.push(line(if i % 2 == 0 { "Error here" } else { "fine" }));
        }
        c.filter = "error".into();
        c.update_matches();
        assert_eq!(c.matches.len(), MAX_LINES / 2);
        // Drops the two oldest lines (one match) and adds one new match.
        c.push(line("another ERROR"));
        c.push(line("ok"));
        c.update_matches();
        assert_eq!(c.matches.len(), MAX_LINES / 2);
        assert!(c.matches.iter().all(|&seq| c.lines[(seq - c.first) as usize].text.to_lowercase().contains("error")));
        c.filter = "fine".into();
        c.update_matches();
        assert_eq!(c.matches.len(), MAX_LINES / 2 - 1);
    }
}
