// SPDX-License-Identifier: GPL-3.0-only
//! Background work started by the UI: tasks with progress, and data loaded for pages.

use std::future::Future;
use std::sync::Arc;

use futures_util::future::{AbortHandle, Abortable, Aborted};

use super::{App, Bus, Event};
use crate::minecraft::auth::Account;
use crate::minecraft::instance::Loader;
use crate::minecraft::meta::loaders;
use crate::minecraft::meta::mojang;
use crate::minecraft::skin;
use crate::modplatform::modrinth::{self, ContentKind};
use crate::tasks::Progress;

impl App {
    pub(super) fn spawn(&self, fut: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
        let bus = self.bus.clone();
        self.rt.spawn(async move {
            if let Err(e) = fut.await {
                bus.send(Event::Error(format!("{e:#}")));
            }
        });
    }

    pub(super) fn task<F, Fut>(&mut self, label: impl Into<String>, f: F)
    where
        F: FnOnce(Progress, Bus) -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        self.run_task(label, false, f);
    }

    /// Like [`Self::task`], with a Cancel button. Cancelling drops the future at its next await,
    /// so `f` must leave nothing half-done behind when dropped (use drop guards) and must mark
    /// steps that can't be cut short with [`Progress::critical`].
    pub(super) fn cancellable_task<F, Fut>(&mut self, label: impl Into<String>, f: F)
    where
        F: FnOnce(Progress, Bus) -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        self.run_task(label, true, f);
    }

    pub(super) fn run_task<F, Fut>(&mut self, label: impl Into<String>, cancellable: bool, f: F)
    where
        F: FnOnce(Progress, Bus) -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let ctx = self.bus.ctx.clone();
        let progress = Progress::new(label, Arc::new(move || ctx.request_repaint()));
        let (abort, registration) = AbortHandle::new_pair();
        if cancellable {
            progress.set_cancellable(abort);
        }
        self.tasks.push(progress.clone());
        let fut = Abortable::new(f(progress.clone(), self.bus.clone()), registration);
        self.spawn(async move {
            match fut.await {
                Ok(result) => {
                    progress.finish(&result);
                    result
                }
                Err(Aborted) => {
                    progress.cancelled();
                    Ok(())
                }
            }
        });
    }

    pub(super) fn request_manifest(&mut self) {
        if self.manifest_requested {
            return;
        }
        self.manifest_requested = true;
        self.manifest_failed = false;
        let bus = self.bus.clone();
        self.rt.spawn(async move {
            match mojang::manifest().await {
                Ok(manifest) => bus.send(Event::Manifest(manifest.versions)),
                Err(e) => {
                    crate::log::error(&format!("Loading the version list failed: {e:#}"));
                    bus.send(Event::ManifestFailed);
                }
            }
        });
    }

    pub(super) fn request_loader_versions(&mut self, loader: Loader, mc: &str) {
        let key = (loader, mc.to_string());
        if loader == Loader::Vanilla || mc.is_empty() || !self.loader_requested.insert(key) {
            return;
        }
        let bus = self.bus.clone();
        let mc = mc.to_string();
        self.rt.spawn(async move {
            match loaders::versions(loader, &mc).await {
                Ok(versions) => bus.send(Event::LoaderVersions { loader, mc, versions }),
                Err(e) => {
                    crate::log::error(&format!("Loading {} versions for {mc} failed: {e:#}", loader.name()));
                    bus.send(Event::LoaderVersionsFailed { loader, mc });
                }
            }
        });
    }

    /// Forgets a failed lookup, so the next frame that wants it tries again.
    pub(super) fn retry_loader_versions(&mut self, loader: Loader, mc: &str) {
        let key = (loader, mc.to_string());
        self.loader_failed.remove(&key);
        self.loader_requested.remove(&key);
    }

    pub(super) fn search_modrinth(&self, query: String, kind: ContentKind, target: Option<(String, Loader)>, offset: u64) {
        let bus = self.bus.clone();
        self.rt.spawn(async move {
            let (mc, loader) = target.unzip();
            match modrinth::search(&query, kind, mc.as_deref(), loader, offset).await {
                Ok(results) => bus.send(Event::Search { kind, offset, results }),
                Err(e) => {
                    crate::log::error(&format!("Searching Modrinth failed: {e:#}"));
                    bus.send(Event::SearchFailed(kind));
                }
            }
        });
    }

    pub(super) fn load_project_versions(&self, project: String, kind: ContentKind, target: Option<(String, Loader)>) {
        let bus = self.bus.clone();
        self.rt.spawn(async move {
            let (mc, loader) = target.unzip();
            match modrinth::project_versions(&project, kind, mc.as_deref(), loader).await {
                Ok(versions) => bus.send(Event::Versions { project, versions }),
                Err(e) => {
                    crate::log::error(&format!("Loading the versions of {project} failed: {e:#}"));
                    bus.send(Event::VersionsFailed(project));
                }
            }
        });
    }

    pub(super) fn load_skin(&self, account: &Account) {
        let Some(url) = account.skin_url.clone() else { return };
        let (id, slim, bus) = (account.id.clone(), account.skin_slim, self.bus.clone());
        self.spawn(async move {
            let png = skin::fetch(&url).await?;
            let (front, head) = skin::compose(&png, slim)?;
            bus.send(Event::Skin { id, front, head });
            Ok(())
        });
    }

    pub(super) fn fetch_wanted_icons(&self) {
        for key in self.icons.take_wanted() {
            let bus = self.bus.clone();
            self.rt.spawn(async move {
                let image = match key.strip_prefix("file:") {
                    Some(rest) => {
                        let path = rest.rsplit_once('|').map_or(rest, |(p, _)| p);
                        crate::icons::load_file(path.into()).await
                    }
                    None => crate::icons::fetch(&key).await,
                };
                bus.send(Event::Icon { key, image: image.ok() });
            });
        }
    }
}
