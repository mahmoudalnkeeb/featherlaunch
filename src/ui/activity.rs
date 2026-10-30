// SPDX-License-Identifier: GPL-3.0-only
//! Background task status: a summary button in the title bar and a dropdown with the details
//! of every running task (step, files, bytes, speed, time) plus the recently finished ones.

use std::ops::RangeInclusive;
use std::time::Duration;

use eframe::egui::{self, CornerRadius, Id, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, vec2};

use super::glyph::{self, Glyph};
use super::widgets::{self, Kind};
use super::{App, theme};
use crate::tasks::{Outcome, Progress, TaskState};

pub const KEEP_FINISHED: usize = 8;
/// Redraw interval while something runs, for speed and elapsed time. Byte counts don't wake the
/// UI on their own, so this timer updates the bars.
const TICK: Duration = Duration::from_millis(250);
const PANEL_WIDTH: f32 = 440.0;

pub fn title_button(app: &mut App, ui: &mut egui::Ui, bar: Rect, space: RangeInclusive<f32>) {
    if app.tasks.is_empty() {
        return;
    }
    let tasks: Vec<(Progress, TaskState)> = app.tasks.iter().map(|t| (t.clone(), t.snapshot())).collect();
    let states: Vec<&TaskState> = tasks.iter().map(|(_, s)| s).collect();
    let running: Vec<&TaskState> = states.iter().copied().filter(|s| !s.finished()).collect();
    let failed = states.iter().filter(|s| matches!(s.outcome, Outcome::Failed(_))).count();
    if !running.is_empty() {
        ui.ctx().request_repaint_after(TICK);
    }

    let width = 360.0_f32.min(space.end() - space.start());
    if width < 140.0 {
        return;
    }
    let center = bar.center().x.clamp(space.start() + width / 2.0, space.end() - width / 2.0);
    let rect = Rect::from_center_size(Pos2::new(center, bar.center().y), vec2(width, theme::H_SM));
    let response = ui.interact(rect, Id::new("activity-button"), Sense::click());
    let radius = CornerRadius::same(theme::R_MD);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, if response.hovered() { theme::HOVER } else { theme::SURFACE });
    let border = if response.hovered() { theme::TEXT_MUTED } else { theme::TEXT_FAINT };
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, border), StrokeKind::Inside);

    let icon = Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), vec2(12.0, 12.0));
    let (text, color) = match running.first() {
        Some(first) => {
            ui.put(icon, egui::Spinner::new().size(12.0).color(theme::ACCENT));
            let mut text = first.title.clone();
            if let Some(f) = first.fraction() {
                text.push_str(&format!("  ·  {}%", (f * 100.0) as u32));
            }
            if running.len() > 1 {
                text.push_str(&format!("  +{} more", running.len() - 1));
            }
            (text, theme::TEXT)
        }
        None if failed > 0 => {
            glyph::paint(painter, icon, Glyph::Close, theme::ERR);
            (format!("{failed} task{} failed", if failed == 1 { "" } else { "s" }), theme::ERR)
        }
        None => {
            glyph::paint(painter, icon, Glyph::Check, theme::OK);
            ("All tasks done".to_string(), theme::TEXT_MUTED)
        }
    };
    let text_left = rect.left() + 30.0;
    widgets::line(ui, Pos2::new(text_left, rect.center().y - 9.0), &text, theme::label(), color, rect.right() - text_left - 28.0);
    glyph::paint(
        ui.painter(),
        Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.center().y), vec2(12.0, 12.0)),
        Glyph::ChevronDown,
        theme::TEXT_MUTED,
    );

    // Overall progress of everything running, along the button's bottom edge.
    let fractions: Vec<f32> = running.iter().filter_map(|s| s.fraction()).collect();
    if !fractions.is_empty() {
        let overall = fractions.iter().sum::<f32>() / fractions.len() as f32;
        let track =
            Rect::from_min_max(Pos2::new(rect.left() + 1.0, rect.bottom() - 3.0), Pos2::new(rect.right() - 1.0, rect.bottom() - 1.0));
        widgets::progress(ui, track, overall);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), radius, Stroke::new(1.5_f32, theme::ACCENT), StrokeKind::Outside);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Activity: {text}")));

    let popup = Id::new("activity-panel");
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    let mut clear = false;
    egui::popup::popup_below_widget(ui, popup, &response, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_width(PANEL_WIDTH);
        // Popups justify their text otherwise, which stretches a wrapped error's spaces.
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| clear = panel(ui, &tasks));
    });
    if clear {
        app.tasks.retain(|t| !t.snapshot().finished());
        if app.tasks.is_empty() {
            ui.memory_mut(|m| m.close_popup());
        }
    }
}

/// The dropdown body. Returns true when "Clear finished" was pressed.
fn panel(ui: &mut egui::Ui, tasks: &[(Progress, TaskState)]) -> bool {
    let mut clear = false;
    let (running, finished): (Vec<_>, Vec<_>) = tasks.iter().partition(|(_, s)| !s.finished());
    ui.horizontal(|ui| {
        ui.label(RichText::new("Activity").font(theme::body_strong()).color(theme::TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !finished.is_empty() && widgets::pill_sized(ui, Kind::Ghost, None, "Clear finished", theme::H_SM).clicked() {
                clear = true;
            }
        });
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(460.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        if running.is_empty() {
            ui.label(RichText::new("Nothing is running.").color(theme::TEXT_MUTED));
        }
        for (progress, s) in &running {
            if running_card(ui, s) {
                progress.cancel();
            }
        }
        if !finished.is_empty() {
            ui.add_space(6.0);
            widgets::section(ui, "Recent");
            // Newest first.
            for (_, s) in finished.iter().rev() {
                finished_card(ui, s);
            }
        }
    });
    clear
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::BG)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui);
        });
}

/// One running task. Returns true when its Cancel button was pressed.
fn running_card(ui: &mut egui::Ui, s: &TaskState) -> bool {
    let mut cancel = false;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
            ui.label(RichText::new(&s.title).font(theme::body_strong()).color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if s.can_cancel() && widgets::pill_sized(ui, Kind::Secondary, None, "Cancel", theme::H_SM).clicked() {
                    cancel = true;
                }
                if let Some(f) = s.fraction() {
                    ui.label(RichText::new(format!("{}%", (f * 100.0) as u32)).font(theme::label()).color(theme::TEXT_MUTED));
                }
            });
        });
        if s.label != s.title {
            ui.label(RichText::new(&s.label).font(theme::label()).color(theme::TEXT_MUTED));
        }
        if let Some(f) = s.fraction() {
            let (track, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
            widgets::progress(ui, track, f);
        }
        let stats = stats_line(s);
        if !stats.is_empty() {
            ui.label(RichText::new(stats).font(theme::label()).color(theme::TEXT_MUTED));
        }
        if !s.active.is_empty() {
            let shown: Vec<&str> = s.active.iter().take(3).map(String::as_str).collect();
            let mut text = format!("Downloading {}", shown.join(", "));
            if s.active.len() > 3 {
                text.push_str(&format!(" +{} more", s.active.len() - 3));
            }
            let w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
            widgets::line(ui, rect.min, &text, theme::label(), theme::TEXT_MUTED, w);
        }
        if s.cancellable && s.critical > 0 {
            ui.label(RichText::new("This step can't be cancelled; it will finish first.").font(theme::label()).color(theme::TEXT_MUTED));
        }
    });
    cancel
}

fn finished_card(ui: &mut egui::Ui, s: &TaskState) {
    card(ui, |ui| {
        let (glyph, color, line) = match &s.outcome {
            Outcome::Failed(e) => (Glyph::Close, theme::ERR, e.clone()),
            Outcome::Cancelled => (Glyph::Close, theme::TEXT_MUTED, format!("Cancelled after {}", duration(s.elapsed()))),
            _ => (Glyph::Check, theme::OK, format!("Done in {}", duration(s.elapsed()))),
        };
        ui.horizontal(|ui| {
            let (icon, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            glyph::paint(ui.painter(), icon, glyph, color);
            ui.label(RichText::new(&s.title).font(theme::body_strong()).color(theme::TEXT));
        });
        let text_color = if color == theme::ERR { theme::ERR } else { theme::TEXT_MUTED };
        ui.add(egui::Label::new(RichText::new(line).font(theme::label()).color(text_color)).wrap());
    });
}

/// "312 / 1204 files  ·  84.2 / 210.5 MB  ·  4.1 MB/s  ·  0:42  ·  about 1:10 left"
fn stats_line(s: &TaskState) -> String {
    let mut parts = Vec::new();
    if s.total > 0 {
        parts.push(format!("{} / {} files", s.done, s.total));
    }
    if s.bytes_total > 0 {
        parts.push(format!("{} / {}", bytes(s.bytes_done), bytes(s.bytes_total)));
    } else if s.bytes_done > 0 {
        parts.push(bytes(s.bytes_done));
    }
    if s.speed >= 1.0 && !s.active.is_empty() {
        parts.push(format!("{}/s", bytes(s.speed as u64)));
    }
    parts.push(duration(s.elapsed()));
    if let Some(left) = s.remaining().filter(|_| !s.active.is_empty()) {
        parts.push(format!("about {} left", duration(left)));
    }
    parts.join("  ·  ")
}

fn bytes(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} B"),
        1_000..1_000_000 => format!("{:.0} KB", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.2} GB", n as f64 / 1e9),
    }
}

/// 0:42, 12:05, 1:02:09.
fn duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(84_200_000), "84.2 MB");
        assert_eq!(duration(Duration::from_secs(42)), "0:42");
        assert_eq!(duration(Duration::from_secs(3729)), "1:02:09");
    }
}
