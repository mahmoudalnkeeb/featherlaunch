// SPDX-License-Identifier: GPL-3.0-only
// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
#[cfg(target_os = "linux")]
mod desktop_entry;
mod icons;
mod java;
mod log;
mod minecraft;
mod modplatform;
mod net;
mod paths;
mod tasks;
mod ui;
mod updater;

use eframe::egui;

fn main() -> eframe::Result<()> {
    if let Err(e) = paths::ensure_dirs() {
        eprintln!("cannot create the data folder {}: {e}", paths::root().display());
    }
    log::install_panic_hook();
    #[cfg(target_os = "linux")]
    if let Err(e) = desktop_entry::install() {
        log::error(&format!("cannot install the menu entry and icon: {e:#}"));
    }
    // The work is network and disk bound, and fewer threads use less RAM.
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    let handle = runtime.handle().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FeatherLaunch")
            // Wayland app ID / X11 WM_CLASS; matches packaging/linux/featherlaunch.desktop.
            .with_app_id("featherlaunch")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default())
            .with_inner_size([1060.0, 680.0])
            .with_min_inner_size([760.0, 480.0])
            // The app draws its own title bar (src/ui/chrome.rs) to match the theme.
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native("FeatherLaunch", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, handle)))))
}
