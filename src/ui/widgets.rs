// SPDX-License-Identifier: GPL-3.0-only
//! The design system's components (DESIGN.md → Components), painted on top of egui.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TextEdit, Ui, emath, vec2,
};

use super::glyph::{self, Glyph};
use super::theme;

/// 1234 → "1.2K", 15_300_000 → "15.3M".
pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}K", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
    .replace(".0", "")
}

pub fn icon(ui: &Ui, rect: Rect, tex: Option<egui::TextureId>, label: &str, color: Color32) {
    let radius = CornerRadius::same(theme::R_MD);
    match tex {
        Some(id) => {
            egui::Image::new(egui::load::SizedTexture::new(id, rect.size())).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, radius, color.gamma_multiply(0.22));
            letter(ui, rect, label, color);
        }
    }
}

pub fn letter(ui: &Ui, rect: Rect, label: &str, color: Color32) {
    let letter: String = label.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().collect();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, theme::semibold(rect.height() * 0.42), color);
}

/// One line of text cut with "…" at `max_width`. Returns true when it was cut, so the caller can
/// offer the full text as a tooltip.
pub fn line(ui: &Ui, pos: Pos2, text: &str, font: FontId, color: Color32, max_width: f32) -> bool {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(10.0));
    let galley = ui.fonts(|f| f.layout_job(job));
    let elided = galley.elided;
    ui.painter().galley(pos, galley, color);
    elided
}

pub fn line_tip(ui: &mut Ui, text: &str, font: FontId, color: Color32, height: f32) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let row = ui.fonts(|f| f.row_height(&font));
    let top = rect.top() + (height - row) / 2.0;
    if line(ui, Pos2::new(rect.left(), top), text, font, color, width) {
        response.on_hover_text(text);
    }
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x)
}

/// Keyboard focus outline for painted widgets, which egui doesn't outline on its own.
pub fn focus_ring(ui: &Ui, rect: Rect, response: &Response) {
    if response.has_focus() {
        let stroke = Stroke::new(1.5_f32, theme::ACCENT);
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(theme::R_MD), stroke, StrokeKind::Outside);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub fn pill(ui: &mut Ui, kind: Kind, glyph: Option<Glyph>, text: &str) -> Response {
    pill_sized(ui, kind, glyph, text, theme::H_MD)
}

pub fn pill_sized(ui: &mut Ui, kind: Kind, glyph: Option<Glyph>, text: &str, height: f32) -> Response {
    let font = if kind == Kind::Primary { theme::body_strong() } else { theme::body() };
    // With a label, glyph ink ends about 13 px into its box and the label follows 8 px later.
    let glyph_w = match glyph {
        None => 0.0,
        Some(_) if text.is_empty() => 16.0,
        Some(g) => 21.0 - glyph::ink_left(g),
    };
    let pad = if text.is_empty() { (height - 16.0) / 2.0 } else { theme::S4 };
    let width = text_width(ui, text, font.clone()) + glyph_w + pad * 2.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = response.hovered() && enabled;
        let (fill, fg) = match kind {
            Kind::Primary => (if hovered { theme::ACCENT_HOVER } else { theme::ACCENT }, theme::ON_ACCENT),
            Kind::Secondary => (if hovered { theme::HOVER } else { theme::RAISED }, theme::TEXT),
            Kind::Ghost => {
                (if hovered { theme::HOVER } else { Color32::TRANSPARENT }, if hovered { theme::TEXT } else { theme::TEXT_MUTED })
            }
            Kind::Danger => (theme::alpha(theme::ERR, if hovered { 0.28 } else { 0.16 }), theme::ERR),
        };
        let (fill, fg) = if enabled { (fill, fg) } else { (theme::RAISED, theme::TEXT_FAINT) };
        let painter = ui.painter();
        let radius = CornerRadius::same(theme::R_MD);
        painter.rect_filled(rect, radius, fill);
        if kind == Kind::Secondary || !enabled {
            let border = if hovered { theme::TEXT_MUTED } else { theme::TEXT_FAINT };
            painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, border), StrokeKind::Inside);
        }
        let mut x = rect.left() + pad;
        if let Some(g) = glyph {
            let left = if text.is_empty() { x } else { x - glyph::ink_left(g) };
            glyph::paint(painter, Rect::from_center_size(Pos2::new(left + 8.0, rect.center().y), vec2(16.0, 16.0)), g, fg);
            x += glyph_w;
        }
        if !text.is_empty() {
            painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, text, font, fg);
        }
        focus_ring(ui, rect, &response);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response
}

/// A 32×32 square button holding a glyph. Always pass a tooltip: the glyph alone is not a label.
pub fn icon_button(ui: &mut Ui, glyph: Glyph, tip: &str) -> Response {
    icon_button_sized(ui, glyph, tip, theme::H_MD)
}

pub fn icon_button_sized(ui: &mut Ui, glyph: Glyph, tip: &str, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() && ui.is_enabled();
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::R_MD), theme::HOVER);
        }
        let color = match (ui.is_enabled(), hovered) {
            (false, _) => theme::TEXT_FAINT,
            (true, true) => theme::TEXT,
            (true, false) => theme::TEXT_MUTED,
        };
        glyph::paint(ui.painter(), glyph::centered(rect, 16.0), glyph, color);
        focus_ring(ui, rect, &response);
    }
    // The tooltip doubles as the accessible name, since the glyph has no text.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
    response.on_hover_text(tip)
}

pub fn menu_item(ui: &mut Ui, text: &str) -> Response {
    menu_row(ui, text, theme::TEXT, false)
}

pub fn menu_check(ui: &mut Ui, text: &str, checked: bool) -> Response {
    menu_row(ui, text, theme::TEXT, checked)
}

pub fn menu_danger(ui: &mut Ui, text: &str) -> Response {
    menu_row(ui, text, theme::ERR, false)
}

fn menu_row(ui: &mut Ui, text: &str, color: Color32, checked: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), theme::H_SM + 2.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::R_SM), theme::HOVER);
        }
        if checked {
            let g = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), vec2(14.0, 14.0));
            glyph::paint(ui.painter(), g, Glyph::Check, theme::TEXT_MUTED);
        }
        let pos = Pos2::new(rect.left() + 10.0, rect.center().y - 9.0);
        line(ui, pos, text, theme::body(), color, rect.right() - pos.x - 30.0);
        if response.has_focus() {
            ui.painter().rect_stroke(rect, CornerRadius::same(theme::R_SM), Stroke::new(1.5_f32, theme::ACCENT), StrokeKind::Inside);
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
        // Lists show a whole column of these, so "on" stays quieter than a primary button.
        let track = if *on {
            theme::alpha(theme::ACCENT, 0.28)
        } else if response.hovered() {
            theme::HOVER
        } else {
            theme::RAISED
        };
        let radius = CornerRadius::same(theme::R_SM);
        ui.painter().rect_filled(rect, radius, track);
        if !*on {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0_f32, theme::TEXT_FAINT), StrokeKind::Inside);
        }
        focus_ring(ui, rect, &response);
        let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        let knob = if *on { theme::ACCENT } else { theme::TEXT_MUTED };
        ui.painter().rect_filled(Rect::from_center_size(Pos2::new(x, rect.center().y), vec2(12.0, 12.0)), radius, knob);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    response
}

pub fn toggle_labeled(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    ui.horizontal(|ui| {
        let response = toggle(ui, on);
        ui.label(egui::RichText::new(text).color(theme::TEXT));
        response
    })
    .inner
}

pub fn chip(ui: &mut Ui, text: &str, count: Option<usize>, selected: bool) -> Response {
    let font = theme::label();
    let count_text = count.map(|c| c.to_string());
    let count_font = theme::caption();
    let count_w = count_text.as_ref().map_or(0.0, |c| text_width(ui, c, count_font.clone()) + 12.0 + 6.0);
    let width = text_width(ui, text, font.clone()) + 24.0 + count_w;
    let (rect, response) = ui.allocate_exact_size(vec2(width, theme::H_SM), Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, fg, outline) = match (selected, response.hovered()) {
            (true, _) => (theme::alpha(theme::ACCENT, 0.14), theme::TEXT, theme::ACCENT),
            (false, true) => (theme::HOVER, theme::TEXT, theme::TEXT_MUTED),
            (false, false) => (theme::RAISED, theme::TEXT_MUTED, theme::TEXT_FAINT),
        };
        let painter = ui.painter();
        let radius = CornerRadius::same(theme::R_MD);
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, outline), StrokeKind::Inside);
        focus_ring(ui, rect, &response);
        painter.text(Pos2::new(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, text, font, fg);
        if let Some(c) = count_text {
            let w = count_w - 6.0;
            let capsule = Rect::from_min_size(Pos2::new(rect.right() - 6.0 - w, rect.top() + 5.0), vec2(w, 18.0));
            painter.rect_filled(capsule, CornerRadius::same(theme::R_SM), theme::BG);
            painter.text(capsule.center(), Align2::CENTER_CENTER, c, count_font, fg);
        }
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, text));
    response
}

/// Underlined tabs for going to a view, on a baseline as wide as the page. Returns true when the
/// selection changed.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let font = theme::body();
    let widths: Vec<f32> = items.iter().map(|(_, t)| text_width(ui, t, font.clone()) + 24.0).collect();
    let (track, _) = ui.allocate_exact_size(vec2(ui.available_width(), theme::H_LG), Sense::hover());
    ui.painter().hline(track.x_range(), track.bottom() - 0.5, Stroke::new(1.0_f32, theme::BORDER));
    let mut changed = false;
    let mut x = track.left();
    for ((value, text), w) in items.iter().zip(widths) {
        let rect = Rect::from_min_size(Pos2::new(x, track.top()), vec2(w, track.height()));
        let response = ui.interact(rect, ui.id().with(("tab", text)), Sense::click());
        let selected = *current == *value;
        if selected {
            let underline = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.max);
            ui.painter().rect_filled(underline, CornerRadius::ZERO, theme::ACCENT);
        } else if response.hovered() {
            ui.painter().rect_filled(rect.shrink2(vec2(0.0, 4.0)), CornerRadius::same(theme::R_SM), theme::HOVER);
        }
        let fg = if selected || response.hovered() { theme::TEXT } else { theme::TEXT_MUTED };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, *text, font.clone(), fg);
        focus_ring(ui, rect.shrink2(vec2(2.0, 4.0)), &response);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, *text));
        if response.clicked() && !selected {
            *current = *value;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Picks one of a few values or modes, as a joined row of buttons. Returns true when it changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let font = theme::label();
    let widths: Vec<f32> = items.iter().map(|(_, t)| text_width(ui, t, font.clone()) + 24.0).collect();
    let (track, _) = ui.allocate_exact_size(vec2(widths.iter().sum(), theme::H_MD), Sense::hover());
    let radius = CornerRadius::same(theme::R_MD);
    ui.painter().rect_filled(track, radius, theme::RAISED);
    let mut changed = false;
    let mut x = track.left();
    for ((value, text), w) in items.iter().zip(widths) {
        let rect = Rect::from_min_size(Pos2::new(x, track.top()), vec2(w, track.height()));
        let response = ui.interact(rect, ui.id().with(("segment", text)), Sense::click());
        let selected = *current == *value;
        if selected {
            ui.painter().rect_filled(rect, radius, theme::HOVER);
        }
        let fg = if selected || response.hovered() { theme::TEXT } else { theme::TEXT_MUTED };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, *text, font.clone(), fg);
        focus_ring(ui, rect.shrink(2.0), &response);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, *text));
        if response.clicked() && !selected {
            *current = *value;
            changed = true;
        }
        x += w;
    }
    ui.painter().rect_stroke(track, radius, Stroke::new(1.0_f32, theme::TEXT_FAINT), StrokeKind::Inside);
    changed
}

pub const INPUT_MARGIN: f32 = 20.0;

pub fn input<'t>(text: &'t mut String, hint: &str) -> TextEdit<'t> {
    TextEdit::singleline(text).hint_text(egui::RichText::new(hint).color(theme::TEXT_MUTED)).margin(Margin::symmetric(10, 7))
}

pub fn multiline(ui: &mut Ui, text: &mut String) -> Response {
    let width = ui.available_width();
    ui.add(TextEdit::multiline(text).desired_rows(3).desired_width(width).margin(Margin::same(10)))
}

pub fn combo(id: &str, selected: impl Into<egui::WidgetText>, width: f32) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id).selected_text(selected).width(width).icon(|ui, rect, visuals, _, _| {
        glyph::paint(ui.painter(), glyph::centered(rect, 12.0), Glyph::ChevronDown, visuals.fg_stroke.color);
    })
}

/// A [`combo`] that stays `width` wide, cutting a long selection with "…". egui measures a
/// truncating ComboBox against the space left in its row, so it needs a box of its own or a long
/// version name or path pushes whatever follows off screen.
pub fn combo_cut(ui: &mut Ui, id: &str, selected: impl Into<egui::WidgetText>, width: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui(vec2(width, theme::H_MD), |ui| {
        combo(id, selected, width).truncate().show_ui(ui, add);
    });
}

pub fn slider<N: emath::Numeric>(ui: &mut Ui, value: &mut N, range: std::ops::RangeInclusive<N>, step: f64, suffix: &str) {
    ui.horizontal(|ui| {
        ui.scope(|ui| {
            // egui sizes the handle from this; a full control height makes it a tall bar.
            ui.spacing_mut().interact_size.y = 20.0;
            ui.add(egui::Slider::new(value, range.clone()).step_by(step).show_value(false));
        });
        ui.add(egui::DragValue::new(value).range(range).speed(step).suffix(suffix));
    });
}

pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, theme::H_MD), Sense::hover());
    let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 32.0, rect.top()), Pos2::new(rect.right() - 12.0, rect.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
    child.set_clip_rect(text_rect.intersect(ui.clip_rect()));
    let edit = TextEdit::singleline(text)
        .hint_text(egui::RichText::new(hint).color(theme::TEXT_MUTED))
        .frame(false)
        .margin(Margin::ZERO)
        .vertical_align(egui::Align::Center)
        .min_size(vec2(0.0, theme::H_MD))
        .desired_width(text_rect.width());
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::R_MD);
    painter.rect_filled(rect, radius, theme::RAISED);
    let response = child.add(edit);
    let border = if response.has_focus() { theme::ACCENT } else { theme::TEXT_FAINT };
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0_f32, border), StrokeKind::Inside);
    let g = Rect::from_center_size(Pos2::new(rect.left() + 18.0, rect.center().y), vec2(15.0, 15.0));
    glyph::paint(ui.painter(), g, Glyph::Search, theme::TEXT_FAINT);
    response
}

pub fn list_row(ui: &mut Ui, height: f32, sense: Sense, selected: bool) -> (Rect, Response) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(theme::R_MD);
        // contains_pointer, not hovered: buttons on the row would otherwise "unhover" it.
        let fill = if selected {
            theme::alpha(theme::ACCENT, 0.12)
        } else if response.contains_pointer() {
            theme::RAISED
        } else {
            theme::SURFACE
        };
        ui.painter().rect_filled(rect, radius, fill);
        if selected {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0_f32, theme::alpha(theme::ACCENT, 0.6)), StrokeKind::Inside);
        }
    }
    (rect, response)
}

pub const LIST_GAP: f32 = theme::S1;

pub fn project_row(ui: &mut Ui, tex: Option<egui::TextureId>, title: &str, subtitle: &str, detail: &str, selected: bool) -> Response {
    let height = if detail.is_empty() { 56.0 } else { 68.0 };
    let (rect, response) = list_row(ui, height, Sense::click(), selected);
    if ui.is_rect_visible(rect) {
        let icon_size = 40.0;
        let icon_rect = Rect::from_min_size(rect.min + vec2(10.0, (height - icon_size) / 2.0), vec2(icon_size, icon_size));
        icon(ui, icon_rect, tex, title, theme::TEXT_MUTED);
        let x = icon_rect.right() + theme::S3;
        let width = rect.right() - x - 10.0;
        let top = rect.top() + if detail.is_empty() { 9.0 } else { 8.0 };
        line(ui, Pos2::new(x, top), title, theme::body_strong(), theme::TEXT, width);
        line(ui, Pos2::new(x, top + 20.0), subtitle, theme::label(), theme::TEXT_MUTED, width);
        if !detail.is_empty() {
            line(ui, Pos2::new(x, top + 37.0), detail, theme::label(), theme::TEXT_MUTED, width);
        }
        focus_ring(ui, rect, &response);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, title));
    response
}

pub fn badge(ui: &Ui, pos: Pos2, text: &str, color: Color32) -> Rect {
    badge_text(ui, pos, text.to_uppercase(), color)
}

/// A badge holding data such as a version number, whose case has to stay as it is.
pub fn badge_exact(ui: &Ui, pos: Pos2, text: &str, color: Color32) -> Rect {
    badge_text(ui, pos, text.to_string(), color)
}

fn badge_text(ui: &Ui, pos: Pos2, text: String, color: Color32) -> Rect {
    let galley = ui.fonts(|f| f.layout_no_wrap(text, theme::caption(), color));
    let rect = Rect::from_min_size(pos, galley.size() + vec2(12.0, 4.0));
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R_SM), theme::alpha(color, 0.18));
    ui.painter().galley(rect.min + vec2(6.0, 2.0), galley, color);
    rect
}

pub fn badge_width(ui: &Ui, text: &str) -> f32 {
    text_width(ui, text, theme::caption()) + 12.0
}

pub fn section(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).font(theme::caption()).color(theme::TEXT_MUTED).extra_letter_spacing(0.8));
}

pub fn progress(ui: &Ui, rect: Rect, fraction: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, theme::RAISED);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if fill.width() > 0.5 {
        painter.rect_filled(fill, CornerRadius::ZERO, theme::ACCENT);
    }
}

pub fn page_frame() -> Frame {
    let (side, top) = (theme::PAGE_PAD as i8, theme::PAGE_TOP as i8);
    Frame::new().inner_margin(Margin { left: side, right: side, top, bottom: side })
}

pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(egui::RichText::new(title).font(theme::display()).color(theme::TEXT));
            ui.label(egui::RichText::new(subtitle).font(theme::label()).color(theme::TEXT_MUTED));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

pub fn page<R>(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            page_frame()
                .show(ui, |ui| {
                    page_header(ui, title, subtitle, |_| {});
                    ui.add_space(theme::S6);
                    add(ui)
                })
                .inner
        })
        .inner
}

pub fn form<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width().min(theme::FORM_WIDTH);
    ui.vertical(|ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

const FORM_LABEL_WIDTH: f32 = 160.0;

pub fn form_section(ui: &mut Ui, text: &str, first: bool) {
    if !first {
        ui.add_space(theme::S6);
    }
    section(ui, text);
    ui.add_space(theme::S1);
}

pub fn form_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(FORM_LABEL_WIDTH, theme::H_MD), Sense::hover());
        ui.painter().text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, label, theme::body(), theme::TEXT_MUTED);
        ui.vertical(|ui| add(ui));
    });
    ui.add_space(theme::S1);
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::label()).color(theme::TEXT_MUTED));
}

pub fn empty_state(ui: &mut Ui, title: &str, body: &str, action: impl FnOnce(&mut Ui)) {
    ui.add_space(theme::S6 * 2.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = theme::S1;
        ui.label(egui::RichText::new(title).font(theme::title()).color(theme::TEXT));
        ui.label(egui::RichText::new(body).color(theme::TEXT_MUTED));
        ui.add_space(theme::S3);
        action(ui);
    });
}

pub fn notice(ui: &mut Ui, color: Color32, text: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(theme::alpha(color, 0.12))
        .corner_radius(CornerRadius::same(theme::R_MD))
        .inner_margin(Margin::symmetric(theme::S3 as i8, theme::S1 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(theme::H_SM);
                if let Some(action) = action {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked = pill_sized(ui, Kind::Secondary, None, action, theme::H_SM).clicked();
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
                        });
                    });
                } else {
                    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
                }
            });
        });
    clicked
}

pub fn save_bar(ui: &mut Ui, id: &str, left: &str, right: impl FnOnce(&mut Ui)) {
    let (side, gap) = (theme::PAGE_PAD as i8, theme::S2 as i8);
    let frame = Frame::new().fill(theme::BG).inner_margin(Margin { left: side, right: side, top: gap, bottom: gap });
    egui::TopBottomPanel::bottom(Id::new(id)).frame(frame).show_inside(ui, |ui| {
        form(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(theme::H_MD);
                ui.label(egui::RichText::new(left).font(theme::label()).color(theme::TEXT_MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
            });
        });
    });
}

/// Fixes a popup menu's width: without an upper limit a context menu takes spacing.menu_width.
pub fn popup_width(ui: &mut Ui, width: f32) {
    ui.set_min_width(width);
    ui.set_max_width(width);
}

/// A modal dialog: a title bar strip with a close button, then `add`. Esc or ✕ ask to close (`.1`); clicking the
/// backdrop does not, so a half-filled form is never lost by a stray click.
pub fn dialog<R>(ctx: &egui::Context, id: &str, title: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let radius = theme::R_LG;
    let frame = Frame::new().fill(theme::SURFACE).stroke(Stroke::new(1.0_f32, theme::BORDER)).corner_radius(CornerRadius::same(radius));
    let max_height = (ctx.screen_rect().height() - 140.0).max(200.0);
    let mut close = false;
    let response = egui::Modal::new(Id::new(id)).backdrop_color(Color32::from_black_alpha(140)).frame(frame).show(ctx, |ui| {
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_width(width.min(ctx.screen_rect().width() - 64.0) + 40.0);
        let header = Frame::new()
            .fill(theme::BG)
            .corner_radius(CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 })
            .inner_margin(Margin { left: 16, right: 6, top: 6, bottom: 6 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).font(theme::body_strong()).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button_sized(ui, Glyph::Close, "Close (Esc)", theme::H_SM).clicked() {
                            close = true;
                        }
                    });
                });
            });
        let bottom = header.response.rect.bottom();
        ui.painter().hline(header.response.rect.x_range(), bottom - 0.5, Stroke::new(1.0_f32, theme::BORDER));
        Frame::new()
            .inner_margin(Margin::same(20))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                ui.set_max_height(max_height);
                add(ui)
            })
            .inner
    });
    let esc = response.is_top_modal && !response.any_popup_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    (response.inner, close || esc)
}

pub fn dialog_buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(theme::S3);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

#[cfg(test)]
mod tests {
    use super::compact;

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1K");
        assert_eq!(compact(1_234), "1.2K");
        assert_eq!(compact(15_300_000), "15.3M");
        assert_eq!(compact(2_000_000_000), "2B");
    }
}
