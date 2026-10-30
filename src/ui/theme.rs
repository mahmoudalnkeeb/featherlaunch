// SPDX-License-Identifier: GPL-3.0-only
//! Design tokens and egui style. See DESIGN.md; views use these names instead of raw colors.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Shadow, Stroke, TextStyle};

use crate::minecraft::instance::Loader;

pub const BG: Color32 = Color32::from_rgb(0x0E, 0x0F, 0x11);
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x17, 0x1A);
pub const RAISED: Color32 = Color32::from_rgb(0x1E, 0x20, 0x24);
pub const HOVER: Color32 = Color32::from_rgb(0x26, 0x28, 0x2D);
pub const BORDER: Color32 = Color32::from_rgb(0x26, 0x28, 0x2D);
pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xED, 0xEF);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x90, 0x95, 0xA0);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x5E, 0x62, 0x6B);
pub const ACCENT: Color32 = Color32::from_rgb(0x74, 0xDD, 0xE8);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x9A, 0xE8, 0xF0);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0B, 0x12, 0x14);
pub const OK: Color32 = Color32::from_rgb(0x5D, 0xD3, 0x9E);
pub const WARN: Color32 = Color32::from_rgb(0xF2, 0xC4, 0x6D);
pub const ERR: Color32 = Color32::from_rgb(0xF0, 0x71, 0x78);
pub const CLOSE: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

pub const R_SM: u8 = 2;
pub const R_MD: u8 = 3;
pub const R_LG: u8 = 4;

pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S6: f32 = 24.0;

pub const H_SM: f32 = 28.0;
pub const H_MD: f32 = 32.0;
pub const H_LG: f32 = 36.0;

pub const PAGE_PAD: f32 = S6;
pub const PAGE_TOP: f32 = S4;
/// Forms stop here, so their save bar and fields stay together on a wide window.
pub const FORM_WIDTH: f32 = 640.0;

const SEMIBOLD: &str = "semibold";

/// Semibold text at any size. Views use the named sizes below; this is for sizes computed from
/// a shape, such as the letter on a placeholder icon.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn display() -> FontId {
    semibold(22.0)
}

pub fn title() -> FontId {
    semibold(16.0)
}

pub fn body() -> FontId {
    regular(14.0)
}

pub fn body_strong() -> FontId {
    semibold(14.0)
}

pub fn label() -> FontId {
    regular(13.0)
}

/// Section labels and badges. Always uppercase.
pub fn caption() -> FontId {
    semibold(11.0)
}

pub fn alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha)
}

pub fn card() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(R_LG))
        .inner_margin(Margin::same(S4 as i8))
}

/// System UI font files as (regular, semibold), best first. Read from disk so nothing is
/// embedded in the binary.
fn system_font_files() -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    use std::path::PathBuf;
    if cfg!(windows) {
        let dir = std::env::var_os("WINDIR").map_or_else(|| r"C:\Windows".into(), PathBuf::from).join("Fonts");
        return vec![(dir.join("segoeui.ttf"), dir.join("seguisb.ttf"))];
    }
    // Linux: Noto Sans (most distros), then Ubuntu, in the folders distros put them in.
    let mut out = Vec::new();
    for dir in ["noto", "truetype/noto", "google-noto", "google-noto-sans-fonts", "opentype/noto", "TTF"] {
        let d = PathBuf::from("/usr/share/fonts").join(dir);
        out.push((d.join("NotoSans-Regular.ttf"), d.join("NotoSans-SemiBold.ttf")));
    }
    let ubuntu = PathBuf::from("/usr/share/fonts/truetype/ubuntu");
    out.push((ubuntu.join("Ubuntu-R.ttf"), ubuntu.join("Ubuntu-M.ttf")));
    out
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = fallback.clone();
    let mut semibold = fallback;
    if let Some((regular_file, semibold_file)) = system_font_files().into_iter().find(|(r, _)| r.is_file()) {
        if let Ok(bytes) = std::fs::read(&regular_file) {
            fonts.font_data.insert("ui".into(), Arc::new(FontData::from_owned(bytes)));
            regular.insert(0, "ui".into());
            semibold.insert(0, "ui".into());
        }
        if let Ok(bytes) = std::fs::read(&semibold_file) {
            fonts.font_data.insert("ui-semibold".into(), Arc::new(FontData::from_owned(bytes)));
            semibold.insert(0, "ui-semibold".into());
        }
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    fonts
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = RAISED; // text edit background
    v.faint_bg_color = SURFACE;
    v.code_bg_color = RAISED;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    // Shadows are overdraw, which is slow on old integrated GPUs.
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_corner_radius = CornerRadius::same(R_LG);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.selection.bg_fill = alpha(ACCENT, 0.22);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = None;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERR;
    v.striped = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.6 };
    v.text_cursor.stroke = Stroke::new(2.0_f32, ACCENT);

    let radius = CornerRadius::same(R_MD);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    // Inputs and combo boxes are outlined like native desktop controls. Control outlines use
    // TEXT_FAINT, not BORDER, so they reach the 3:1 contrast WCAG asks of control boundaries.
    w.inactive.bg_stroke = Stroke::new(1.0_f32, TEXT_FAINT);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = HOVER;
    w.active.weak_bg_fill = HOVER;
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open.bg_fill = HOVER;
    w.open.weak_bg_fill = HOVER;
    w.open.bg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);
    w.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.open.corner_radius = radius;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.window_margin = Margin::same(20);
        s.spacing.menu_margin = Margin::same(4);
        s.spacing.menu_width = 200.0;
        s.spacing.interact_size.y = H_MD;
        s.spacing.combo_height = 280.0;
        s.spacing.slider_width = 200.0;
        s.spacing.slider_rail_height = 4.0;
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.interaction.selectable_labels = false;
        s.text_styles = [
            (TextStyle::Heading, display()),
            (TextStyle::Body, body()),
            (TextStyle::Button, body()),
            (TextStyle::Small, label()),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
    });
}

pub fn loader_color(loader: Loader) -> Color32 {
    match loader {
        Loader::Vanilla => Color32::from_rgb(0x5F, 0xA3, 0x5A),
        Loader::Fabric => Color32::from_rgb(0xC2, 0xA5, 0x7A),
        Loader::Quilt => Color32::from_rgb(0x9A, 0x6B, 0xD0),
        Loader::Forge => Color32::from_rgb(0x5B, 0x7C, 0xB8),
        Loader::NeoForge => Color32::from_rgb(0xE0, 0x8A, 0x45),
    }
}
