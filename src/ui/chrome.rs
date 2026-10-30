// SPDX-License-Identifier: GPL-3.0-only
//! Window chrome drawn by the app: title bar, navigation rail, resize edges and outline.
//! The native title bar is off (see main.rs), so dragging, resizing and the window buttons live here.

use eframe::egui::viewport::ResizeDirection;
use eframe::egui::{
    self, Align2, CornerRadius, CursorIcon, Frame, Id, LayerId, Margin, Order, Pos2, Rect, Sense, Stroke, StrokeKind, ViewportCommand, vec2,
};

use super::glyph::{self, Glyph};
use super::{App, Page, activity, theme, widgets};
use crate::minecraft::auth::AccountKind;

pub const TITLE_HEIGHT: f32 = 40.0;
const SIDEBAR_WIDTH: f32 = 180.0;
const SIDEBAR_COLLAPSED_WIDTH: f32 = 56.0;
const EDGE: f32 = 6.0;

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn resize_direction(ctx: &egui::Context) -> Option<ResizeDirection> {
    if maximized(ctx) {
        return None;
    }
    let pos = ctx.input(|i| i.pointer.hover_pos())?;
    let screen = ctx.screen_rect();
    let (left, right) = (pos.x - screen.left() < EDGE, screen.right() - pos.x < EDGE);
    let (top, bottom) = (pos.y - screen.top() < EDGE, screen.bottom() - pos.y < EDGE);
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

/// Set while the sizing border is on for a drag (see [`drag_wndproc`]).
#[cfg(windows)]
static NATIVE_DRAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
static WINIT_WNDPROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
unsafe fn set_sizing_border(hwnd: windows_sys::Win32::Foundation::HWND, on: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_STYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_THICKFRAME};

    NATIVE_DRAG.store(on, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let wanted = if on { style | WS_THICKFRAME } else { style & !WS_THICKFRAME };
        SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
    }
}

/// Windows only starts an edge resize, or a title bar drag that can snap to the screen edges or
/// leave maximized, on a window with `WS_THICKFRAME`. A frame presented with that style lands
/// 8 px off (see [`fix_window_style`]), so the style can't be set from a frame: it goes on here,
/// after the last frame and just before `WM_NCLBUTTONDOWN` starts the drag, and comes off as soon
/// as the drag is under way.
#[cfg(windows)]
unsafe extern "system" fn drag_wndproc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, WM_ENTERSIZEMOVE, WM_NCLBUTTONDOWN, WNDPROC,
    };

    let original = || unsafe {
        let wndproc = std::mem::transmute::<isize, WNDPROC>(WINIT_WNDPROC.load(std::sync::atomic::Ordering::Relaxed));
        CallWindowProcW(wndproc, hwnd, message, wparam, lparam)
    };
    let hit = wparam as u32;
    match message {
        WM_NCLBUTTONDOWN if hit == HTCAPTION || (HTLEFT..=HTBOTTOMRIGHT).contains(&hit) => {
            unsafe { set_sizing_border(hwnd, true) };
            // Returns when the drag ends.
            let result = original();
            unsafe { set_sizing_border(hwnd, false) };
            result
        }
        WM_ENTERSIZEMOVE => {
            unsafe { set_sizing_border(hwnd, false) };
            original()
        }
        _ => original(),
    }
}

/// winit keeps `WS_CAPTION` and `WS_THICKFRAME` on borderless windows and hides them through
/// `WM_NCCALCSIZE`. Some OpenGL drivers (Intel HD 4000/4600) ignore that and present the picture
/// inset by the caption and border, so it lands up to 30 px off from where clicks go and the
/// bottom and right are cut off. Both styles are dropped.
///
/// Without a caption, Windows treats a maximized window as fullscreen and lets it cover the
/// taskbar, so a maximized window is kept to the monitor's work area.
///
/// winit restores its styles on some state changes (maximize, focus), so this runs every frame;
/// it is a few cheap calls when nothing changed.
#[cfg(windows)]
pub fn fix_window_style(frame: &eframe::Frame) {
    use std::sync::atomic::Ordering;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GWLP_WNDPROC, GetWindowLongPtrW, GetWindowRect, IsZoomed, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_THICKFRAME,
    };
    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(win) = handle.as_raw() else { return };
    let hwnd = win.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    unsafe {
        if WINIT_WNDPROC.load(Ordering::Relaxed) == 0 {
            WINIT_WNDPROC.store(SetWindowLongPtrW(hwnd, GWLP_WNDPROC, drag_wndproc as *const () as isize), Ordering::Relaxed);
        }
        let zoomed = IsZoomed(hwnd) != 0;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let base = style & !(WS_CAPTION | WS_THICKFRAME);
        let wanted = if NATIVE_DRAG.load(Ordering::Relaxed) { base | WS_THICKFRAME } else { base };
        if style != wanted {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, flags | SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE);
        }
        if zoomed {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            let mut rect: RECT = std::mem::zeroed();
            if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) != 0 && GetWindowRect(hwnd, &mut rect) != 0 {
                let work = info.rcWork;
                if (rect.left, rect.top, rect.right, rect.bottom) != (work.left, work.top, work.right, work.bottom) {
                    let (w, h) = (work.right - work.left, work.bottom - work.top);
                    SetWindowPos(hwnd, std::ptr::null_mut(), work.left, work.top, w, h, flags);
                }
            }
        }
    }
}

/// Edge resizing and the 1 px outline. Runs after the views so its cursor wins over theirs.
pub fn frame_edges(ctx: &egui::Context) {
    if let Some(dir) = resize_direction(ctx) {
        ctx.set_cursor_icon(match dir {
            ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
    if !maximized(ctx) {
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("window-outline")));
        painter.rect_stroke(ctx.screen_rect(), CornerRadius::ZERO, Stroke::new(1.0_f32, theme::BORDER), StrokeKind::Inside);
    }
}

pub fn title_bar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("titlebar").exact_height(TITLE_HEIGHT).frame(Frame::new().fill(theme::BG)).show_separator_line(true).show(
        ctx,
        |ui| {
            let bar = ui.max_rect();
            // Registered first so every widget drawn later sits on top of the drag area.
            let drag = ui.interact(bar, Id::new("titlebar-drag"), Sense::click_and_drag());
            if drag.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
            } else if drag.drag_started_by(egui::PointerButton::Primary) && resize_direction(ctx).is_none() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }

            let mark = Rect::from_min_size(Pos2::new(bar.left() + 17.0, bar.center().y - 11.0), vec2(22.0, 22.0));
            glyph::app_mark(ui.painter(), mark, theme::ACCENT, theme::ON_ACCENT);
            let name = ui.painter().text(
                Pos2::new(mark.right() + 12.0, bar.center().y),
                Align2::LEFT_CENTER,
                "FeatherLaunch",
                theme::body_strong(),
                theme::TEXT,
            );
            let mut x = bar.right();
            let restore = maximized(ctx);
            for (glyph, tip) in [
                (Glyph::Close, "Close"),
                (if restore { Glyph::Restore } else { Glyph::Maximize }, if restore { "Restore" } else { "Maximize" }),
                (Glyph::Minimize, "Minimize"),
            ] {
                let rect = Rect::from_min_max(Pos2::new(x - 46.0, bar.top()), Pos2::new(x, bar.bottom()));
                x -= 46.0;
                let r = ui.interact(rect, Id::new(("window-button", glyph as u8)), Sense::click()).on_hover_text(tip);
                let close = glyph == Glyph::Close;
                if r.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, if close { theme::CLOSE } else { theme::HOVER });
                }
                let color = if r.hovered() { theme::TEXT } else { theme::TEXT_MUTED };
                glyph::paint(ui.painter(), glyph::centered(rect, 14.0), glyph, color);
                if r.has_focus() {
                    ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.5_f32, theme::ACCENT), StrokeKind::Inside);
                }
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
                if r.clicked() {
                    ctx.send_viewport_cmd(match glyph {
                        Glyph::Close => ViewportCommand::Close,
                        Glyph::Minimize => ViewportCommand::Minimized(true),
                        _ => ViewportCommand::Maximized(!maximized(ctx)),
                    });
                }
            }

            let chip_left = account_chip(app, ui, Pos2::new(x - 12.0, bar.center().y));
            activity::title_button(app, ui, bar, name.right() + 12.0..=chip_left - 12.0);
        },
    );
}

/// Selected account (skin head and name) with a menu to switch or manage accounts. Returns the
/// chip's left edge.
fn account_chip(app: &mut App, ui: &mut egui::Ui, right_center: Pos2) -> f32 {
    let selected = app.accounts.selected().cloned();
    let name = selected.as_ref().map_or("Sign in".to_string(), |a| a.name.clone());
    let font = theme::label();
    let text_w = ui.fonts(|f| f.layout_no_wrap(name.clone(), font.clone(), theme::TEXT).size().x);
    let width = 6.0 + 22.0 + 8.0 + text_w + 6.0 + 14.0 + 10.0;
    let rect = Rect::from_min_size(Pos2::new(right_center.x - width, right_center.y - 15.0), vec2(width, 30.0));
    let r = ui.interact(rect, Id::new("account-chip"), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::R_MD), if r.hovered() { theme::HOVER } else { theme::SURFACE });
    painter.rect_stroke(rect, CornerRadius::same(theme::R_MD), Stroke::new(1.0_f32, theme::BORDER), StrokeKind::Inside);
    let head = Rect::from_min_size(Pos2::new(rect.left() + 4.0, rect.top() + 4.0), vec2(22.0, 22.0));
    match selected.as_ref().and_then(|a| app.skins.get(&a.id)) {
        Some(tex) => {
            egui::Image::new(egui::load::SizedTexture::new(tex.head.id(), head.size()))
                .corner_radius(CornerRadius::same(theme::R_SM))
                .paint_at(ui, head);
        }
        None => {
            painter.rect_filled(head, CornerRadius::same(theme::R_SM), theme::RAISED);
            glyph::paint(painter, glyph::centered(head, 12.0), Glyph::User, theme::TEXT_MUTED);
        }
    }
    painter.text(Pos2::new(head.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, &name, font, theme::TEXT);
    glyph::paint(
        painter,
        Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), vec2(12.0, 12.0)),
        Glyph::ChevronDown,
        theme::TEXT_MUTED,
    );
    widgets::focus_ring(ui, rect, &r);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));

    let popup = Id::new("account-menu");
    if r.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    let mut select = None;
    egui::popup::popup_below_widget(ui, popup, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        widgets::popup_width(ui, 220.0);
        for a in &app.accounts.accounts {
            let is_sel = app.accounts.selected.as_deref() == Some(a.id.as_str());
            let kind = if a.kind == AccountKind::Microsoft { "Microsoft" } else { "Offline" };
            if widgets::menu_check(ui, &format!("{}  ·  {kind}", a.name), is_sel).clicked() {
                select = Some(a.id.clone());
            }
        }
        if !app.accounts.accounts.is_empty() {
            ui.separator();
        }
        if widgets::menu_item(ui, "Manage accounts…").clicked() {
            app.go(Page::Accounts);
        }
    });
    if let Some(id) = select {
        app.accounts.selected = Some(id);
        app.save_accounts();
    }
    rect.left()
}

pub fn rail(app: &mut App, ctx: &egui::Context) {
    let collapsed = app.cfg.sidebar_collapsed;
    egui::SidePanel::left("rail")
        .exact_width(if collapsed { SIDEBAR_COLLAPSED_WIDTH } else { SIDEBAR_WIDTH })
        .resizable(false)
        .show_separator_line(true)
        .frame(Frame::new().fill(theme::BG).inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 8 }))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let page = app.page;
            if nav_item(ui, Glyph::Library, "Library", page == Page::Library, collapsed).clicked() {
                app.go(Page::Library);
            }
            if nav_item(ui, Glyph::Plus, "New instance", page == Page::New, collapsed).clicked() {
                app.go(Page::New);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                let (glyph, label) = if collapsed { (Glyph::Forward, "Expand sidebar") } else { (Glyph::Back, "Collapse sidebar") };
                if nav_item(ui, glyph, label, false, collapsed).clicked() {
                    app.cfg.sidebar_collapsed = !collapsed;
                    if let Err(e) = app.cfg.save() {
                        app.errors.push(format!("Saving settings failed: {e:#}"));
                    }
                }
                separator(ui);
                if nav_item(ui, Glyph::Gear, "Settings", page == Page::Settings, collapsed).clicked() {
                    app.go(Page::Settings);
                }
                if nav_item(ui, Glyph::User, "Accounts", page == Page::Accounts, collapsed).clicked() {
                    app.go(Page::Accounts);
                }
                if let Some(label) = app.launcher_update.row_label() {
                    let row = nav_item(ui, Glyph::Download, &label, false, collapsed);
                    let rect = row.rect;
                    let dot = if collapsed { rect.center() + vec2(9.0, -9.0) } else { Pos2::new(rect.right() - 14.0, rect.center().y) };
                    ui.painter().circle_filled(dot, 3.5, theme::OK);
                    if row.clicked() {
                        app.launcher_update_clicked(ctx);
                    }
                }
            });
        });
}

fn nav_item(ui: &mut egui::Ui, glyph: Glyph, label: &str, active: bool, collapsed: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let radius = CornerRadius::same(theme::R_MD);
        if active {
            ui.painter().rect_filled(rect, radius, theme::RAISED);
            let bar = Rect::from_min_size(Pos2::new(rect.left() - 8.0, rect.top() + 8.0), vec2(3.0, 20.0));
            ui.painter().rect_filled(bar, CornerRadius::ZERO, theme::ACCENT);
        } else if hovered {
            ui.painter().rect_filled(rect, radius, theme::HOVER);
        }
        let color = if active || hovered { theme::TEXT } else { theme::TEXT_MUTED };
        let x = if collapsed { rect.center().x } else { rect.left() + 20.0 };
        glyph::paint(ui.painter(), Rect::from_center_size(Pos2::new(x, rect.center().y), vec2(16.0, 16.0)), glyph, color);
        if !collapsed {
            let font = if active { theme::body_strong() } else { theme::body() };
            let pos = Pos2::new(rect.left() + 40.0, rect.center().y - 9.5);
            widgets::line(ui, pos, label, font, color, rect.right() - pos.x - 8.0);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.5_f32, theme::ACCENT), StrokeKind::Inside);
        }
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label));
    if collapsed { response.on_hover_text(label) } else { response }
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 13.0), Sense::hover());
    ui.painter().hline(rect.shrink2(vec2(6.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0_f32, theme::BORDER));
}
