// SPDX-License-Identifier: GPL-3.0-only
//! Messages from background work to the UI thread, and how the app handles them.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use tokio::sync::oneshot;

use super::{App, LauncherUpdate, SkinTex};
use crate::config::OnLaunch;
use crate::java::JavaInstall;
use crate::minecraft::auth::{Account, DeviceCode};
use crate::minecraft::instance::{Loader, World};
use crate::minecraft::launch::LogLine;
use crate::minecraft::meta::mojang::ManifestEntry;
use crate::minecraft::skin;
use crate::modplatform::modrinth::{ContentKind, LocalFile, SearchResponse, Update, Version};
use crate::updater::Release;

pub enum Event {
    Error(String),
    Info(String),
    Warning(String),
    Manifest(Vec<ManifestEntry>),
    ManifestFailed,
    LoaderVersions { loader: Loader, mc: String, versions: Vec<String> },
    LoaderVersionsFailed { loader: Loader, mc: String },
    InstancesChanged(Option<String>),
    InstanceDeleted(String),
    InstanceIconChanged(String),
    Files { inst: String, request: u64, files: Vec<LocalFile> },
    Worlds { inst: String, request: u64, worlds: Vec<World> },
    DeviceCode(DeviceCode),
    LoginFinished,
    Account(Account),
    Skin { id: String, front: Vec<u8>, head: Vec<u8> },
    Log { inst: String, line: LogLine },
    GameStarted { inst: String, kill: oneshot::Sender<()> },
    GameExited { inst: String, code: Option<i32> },
    Search { kind: ContentKind, offset: u64, results: SearchResponse },
    SearchFailed(ContentKind),
    Versions { project: String, versions: Vec<Version> },
    VersionsFailed(String),
    ContentChanged,
    Updates(Vec<Update>),
    Java(Vec<JavaInstall>),
    Icon { key: String, image: Option<crate::icons::Decoded> },
    LauncherUpdateFound(Release),
    LauncherUpdateInstalled(PathBuf),
    LauncherUpdateStopped,
}

#[derive(Clone)]
pub struct Bus {
    pub(super) tx: mpsc::Sender<Event>,
    pub(super) ctx: egui::Context,
}

impl Bus {
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    /// For high-volume events (game logs): batch repaints instead of one per line.
    pub fn send_batched(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl App {
    pub(super) fn process_events(&mut self, ctx: &egui::Context) {
        // Already in launcher.log: `load_json` logs them when it finds them.
        self.errors.extend(crate::paths::take_problems());
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Error(e) => {
                    crate::log::error(&e);
                    self.errors.push(e);
                }
                Event::Info(s) => self.notify(s),
                Event::Warning(s) => self.warn(s),
                Event::Manifest(m) => self.manifest = Some(m),
                Event::ManifestFailed => self.manifest_failed = true,
                Event::LoaderVersions { loader, mc, versions } => {
                    self.loader_versions.insert((loader, mc), versions);
                }
                Event::LoaderVersionsFailed { loader, mc } => {
                    self.loader_failed.insert((loader, mc));
                }
                Event::InstancesChanged(select) => self.reload_instances(select),
                Event::InstanceDeleted(id) => {
                    self.deleting.remove(&id);
                    self.reload_instances(None);
                }
                Event::InstanceIconChanged(id) => {
                    self.reload_instances(Some(id.clone()));
                    let stamp = self.instance(&id).and_then(|i| i.icon_stamp);
                    if let Some(e) = self.editor.as_mut().filter(|e| e.instance_id() == id) {
                        e.set_icon_stamp(stamp);
                    }
                }
                Event::Files { inst, request, files } => {
                    if let Some(e) = self.editor.as_mut().filter(|e| e.instance_id() == inst) {
                        e.on_files(request, files);
                    }
                }
                Event::Worlds { inst, request, worlds } => {
                    if let Some(e) = self.editor.as_mut().filter(|e| e.instance_id() == inst) {
                        e.on_worlds(request, worlds);
                    }
                }
                Event::DeviceCode(code) => self.device_code = Some(code),
                Event::LoginFinished => {
                    self.device_code = None;
                    self.login_cancel = None;
                }
                Event::Account(account) => {
                    let skin_changed = self
                        .accounts
                        .accounts
                        .iter()
                        .find(|a| a.id == account.id)
                        .is_none_or(|old| old.skin_url != account.skin_url || old.skin_slim != account.skin_slim);
                    if skin_changed || !self.skins.contains_key(&account.id) {
                        self.load_skin(&account);
                    }
                    self.accounts.upsert(account);
                    self.save_accounts();
                }
                Event::Skin { id, front, head } => {
                    let tex = |name: String, size: [usize; 2], rgba: &[u8]| {
                        ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, rgba), egui::TextureOptions::NEAREST)
                    };
                    let front = tex(format!("skin-{id}"), [skin::FRONT_W as usize, skin::FRONT_H as usize], &front);
                    let head = tex(format!("head-{id}"), [8, 8], &head);
                    self.skins.insert(id, SkinTex { front, head });
                }
                Event::Log { inst, line } => self.consoles.entry(inst).or_default().push(line),
                Event::GameStarted { inst, kill } => {
                    if let Some(r) = self.running.get_mut(&inst) {
                        r.kill = Some(kill);
                        r.started = Instant::now();
                    }
                    if let Some(i) = self.instances.iter_mut().find(|i| i.id == inst)
                        && let Err(e) = i.record_launch()
                    {
                        self.errors.push(format!("Saving {} failed: {e:#}", i.name));
                    }
                    match self.cfg.on_launch {
                        OnLaunch::KeepOpen => {}
                        OnLaunch::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                        OnLaunch::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    }
                }
                Event::GameExited { inst, code } => self.on_game_exit(ctx, inst, code),
                Event::Search { kind, offset, results } => {
                    if kind == ContentKind::Modpack {
                        self.new_instance.browser.on_search(offset, results);
                    } else if let Some(e) = &mut self.editor {
                        e.on_search(kind, offset, results);
                    }
                }
                Event::SearchFailed(kind) => {
                    if kind == ContentKind::Modpack {
                        self.new_instance.browser.on_search_failed();
                    } else if let Some(e) = &mut self.editor {
                        e.on_search_failed(kind);
                    }
                }
                Event::Versions { project, versions } => {
                    self.new_instance.browser.on_versions(&project, &versions);
                    if let Some(e) = &mut self.editor {
                        e.browser.on_versions(&project, &versions);
                    }
                }
                Event::VersionsFailed(project) => {
                    self.new_instance.browser.on_versions_failed(&project);
                    if let Some(e) = &mut self.editor {
                        e.browser.on_versions_failed(&project);
                    }
                }
                Event::ContentChanged => {
                    if let Some(e) = &mut self.editor {
                        e.invalidate();
                    }
                }
                Event::Updates(updates) => {
                    if let Some(e) = &mut self.editor {
                        e.on_updates(updates);
                    }
                }
                Event::Java(list) => self.java_list = list,
                Event::Icon { key, image } => self.icons.finish(ctx, key, image),
                Event::LauncherUpdateFound(release) => {
                    if self.launcher_update.can_check() {
                        self.launcher_update = LauncherUpdate::Available(release);
                    }
                }
                Event::LauncherUpdateInstalled(exe) => self.launcher_update = LauncherUpdate::Ready(exe),
                Event::LauncherUpdateStopped => {
                    if let LauncherUpdate::Installing(release) = &self.launcher_update {
                        self.launcher_update = LauncherUpdate::Available(release.clone());
                    }
                }
            }
        }
    }
}
