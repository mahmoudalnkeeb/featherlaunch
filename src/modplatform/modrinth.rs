// SPDX-License-Identifier: GPL-3.0-only
//! Modrinth API v2: search, versions, installs with dependencies, updates and enable/disable.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::minecraft::instance::{Instance, Loader};
use crate::modplatform::mrpack;
use crate::net::{self, Download};
use crate::paths;
use crate::tasks::Progress;

const API: &str = "https://api.modrinth.com/v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentKind {
    Mod,
    ResourcePack,
    ShaderPack,
    Modpack,
}

impl ContentKind {
    pub fn project_type(self) -> &'static str {
        match self {
            ContentKind::Mod => "mod",
            ContentKind::ResourcePack => "resourcepack",
            ContentKind::ShaderPack => "shader",
            ContentKind::Modpack => "modpack",
        }
    }

    pub fn folder(self) -> &'static str {
        match self {
            ContentKind::Mod => "mods",
            ContentKind::ResourcePack => "resourcepacks",
            ContentKind::ShaderPack => "shaderpacks",
            ContentKind::Modpack => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Mod => "Mods",
            ContentKind::ResourcePack => "Resource packs",
            ContentKind::ShaderPack => "Shader packs",
            ContentKind::Modpack => "Modpacks",
        }
    }

    fn uses_loader(self) -> bool {
        matches!(self, ContentKind::Mod | ContentKind::Modpack)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
}

impl SearchHit {
    pub fn meta(&self) -> ProjectMeta {
        ProjectMeta { title: self.title.clone(), icon_url: self.icon_url.clone().filter(|u| !u.is_empty()) }
    }
}

#[derive(Clone, Debug)]
pub struct ProjectMeta {
    pub title: String,
    pub icon_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
}

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    pub hashes: HashMap<String, String>,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    pub dependency_type: String,
}

/// File names come from the API and are joined onto content folders, so anything other than a
/// plain name (a path, `..`, a drive letter) is refused rather than written outside the folder.
pub fn safe_file_name(name: &str) -> Result<&str> {
    match mrpack::safe_rel_path(name) {
        Some(p) if p.components().count() == 1 => Ok(name),
        _ => bail!("Modrinth returned an unsafe file name: {name}"),
    }
}

/// Modrinth facets: inner lists are OR, the outer list is AND.
pub fn facets(kind: ContentKind, mc: Option<&str>, loader: Option<Loader>) -> String {
    let mut f: Vec<Vec<String>> = vec![vec![format!("project_type:{}", kind.project_type())]];
    if let Some(mc) = mc {
        f.push(vec![format!("versions:{mc}")]);
    }
    if let Some(loader) = loader.filter(|_| kind.uses_loader()) {
        let names = loader.modrinth_names();
        if !names.is_empty() {
            f.push(names.iter().map(|n| format!("categories:{n}")).collect());
        }
    }
    serde_json::to_string(&f).unwrap()
}

pub async fn search(query: &str, kind: ContentKind, mc: Option<&str>, loader: Option<Loader>, offset: u64) -> Result<SearchResponse> {
    let resp = net::client()
        .get(format!("{API}/search"))
        .query(&[
            ("query", query.to_string()),
            ("facets", facets(kind, mc, loader)),
            ("limit", "20".to_string()),
            ("offset", offset.to_string()),
            ("index", if query.is_empty() { "downloads" } else { "relevance" }.to_string()),
        ])
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

pub async fn project_versions(project: &str, kind: ContentKind, mc: Option<&str>, loader: Option<Loader>) -> Result<Vec<Version>> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(mc) = mc {
        query.push(("game_versions", json!([mc]).to_string()));
    }
    if let Some(loader) = loader.filter(|_| kind.uses_loader())
        && !loader.modrinth_names().is_empty()
    {
        query.push(("loaders", json!(loader.modrinth_names()).to_string()));
    }
    let resp = net::client().get(format!("{API}/project/{project}/version")).query(&query).send().await?.error_for_status()?;
    Ok(resp.json().await?)
}

async fn version(id: &str) -> Result<Version> {
    net::get_json(&format!("{API}/version/{id}")).await
}

async fn project_meta(id: &str) -> ProjectMeta {
    #[derive(Deserialize)]
    struct P {
        title: String,
        #[serde(default)]
        icon_url: Option<String>,
    }
    match net::get_json::<P>(&format!("{API}/project/{id}")).await {
        Ok(p) => ProjectMeta { title: p.title, icon_url: p.icon_url.filter(|u| !u.is_empty()) },
        Err(_) => ProjectMeta { title: id.to_string(), icon_url: None },
    }
}

/// Serializes changes to content indexes, so concurrent installs, updates and lookups can't
/// overwrite each other's entries. Held only for a load, edit and save, never across an await.
static INDEX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContentIndex {
    pub files: HashMap<String, IndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexEntry {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub version_number: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

impl ContentIndex {
    fn path(inst: &Instance) -> PathBuf {
        inst.dir().join("content.json")
    }

    pub fn load(inst: &Instance) -> Self {
        paths::load_json(&Self::path(inst))
    }

    fn save(&self, inst: &Instance) -> Result<()> {
        paths::write_json(&Self::path(inst), self)
    }

    pub fn update<T>(inst: &Instance, edit: impl FnOnce(&mut ContentIndex) -> T) -> Result<T> {
        let _lock = INDEX_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut index = Self::load(inst);
        let out = edit(&mut index);
        index.save(inst)?;
        Ok(out)
    }

    fn has_project(&self, project: &str) -> bool {
        self.files.values().any(|e| e.project_id == project)
    }

    pub fn key(folder: &str, file_name: &str) -> String {
        format!("{folder}/{}", file_name.trim_end_matches(".disabled"))
    }
}

#[derive(Debug, Default)]
pub struct Installed {
    pub titles: Vec<String>,
    pub missing: Vec<String>,
}

impl Installed {
    pub fn summary(&self) -> String {
        format!("Installed {}", self.titles.join(", "))
    }

    pub fn warning(&self, inst: &Instance) -> Option<String> {
        (!self.missing.is_empty()).then(|| {
            format!(
                "{}, but found no version of {} for {}. The game may not start until you add it.",
                self.summary(),
                self.missing.join(", "),
                inst.summary()
            )
        })
    }
}

/// Installs a version and, for mods, its required dependencies. A dependency with no compatible
/// version is listed in the result; a failed lookup stops the install with an error.
pub async fn install(inst: &Instance, kind: ContentKind, chosen: Version, meta: ProjectMeta, progress: &Progress) -> Result<Installed> {
    let mut queue = vec![(chosen, meta)];
    let mut done = HashSet::new();
    let mut installed = Installed::default();
    let mc = inst.mc_version.as_str();

    while let Some((v, ProjectMeta { title, icon_url })) = queue.pop() {
        if !done.insert(v.project_id.clone()) {
            continue;
        }
        progress.set_label(format!("Installing {title}"));
        let file = v.primary_file().context("this version has no files")?;
        let folder = kind.folder();
        let path = inst.game_dir().join(folder).join(safe_file_name(&file.filename)?);

        let mut d = Download::new(&file.url, &path);
        d.sha1 = file.hashes.get("sha1").cloned();
        net::download_tracked(&d, Some(progress)).await?;

        // Replace an older version of the same project only after the new one is on disk, so a
        // failed or cancelled download keeps the working version.
        let new_key = ContentIndex::key(folder, &file.filename);
        let entry = IndexEntry {
            project_id: v.project_id.clone(),
            version_id: v.id.clone(),
            title: title.clone(),
            version_number: v.version_number.clone(),
            icon_url,
        };
        // Saved per file: a cancel between dependencies still leaves an index matching the disk.
        let old = ContentIndex::update(inst, |index| {
            let old: Vec<String> =
                index.files.iter().filter(|(k, e)| e.project_id == v.project_id && **k != new_key).map(|(k, _)| k.clone()).collect();
            for key in &old {
                index.files.remove(key);
            }
            index.files.insert(new_key, entry);
            old
        })?;
        for key in old {
            remove_file_variants(&inst.game_dir().join(&key));
        }
        installed.titles.push(title.clone());

        if kind != ContentKind::Mod {
            continue;
        }
        let index = ContentIndex::load(inst);
        let failed = |id: &str| format!("{title} was installed, but looking up its dependency {id} failed");
        for dep in v.dependencies.iter().filter(|d| d.dependency_type == "required") {
            let dep_version = match (&dep.version_id, &dep.project_id) {
                (_, Some(pid)) if index.has_project(pid) => continue,
                (Some(vid), _) => version(vid).await.with_context(|| failed(vid))?,
                (None, Some(pid)) => {
                    let versions = project_versions(pid, kind, Some(mc), Some(inst.loader)).await.with_context(|| failed(pid))?;
                    let Some(newest) = versions.into_iter().next() else {
                        installed.missing.push(project_meta(pid).await.title);
                        continue;
                    };
                    newest
                }
                (None, None) => continue,
            };
            if !index.has_project(&dep_version.project_id) && !done.contains(&dep_version.project_id) {
                let meta = project_meta(&dep_version.project_id).await;
                queue.push((dep_version, meta));
            }
        }
    }
    Ok(installed)
}

fn remove_file_variants(path: &Path) {
    let _ = std::fs::remove_file(path);
    let mut disabled = path.as_os_str().to_owned();
    disabled.push(".disabled");
    let _ = std::fs::remove_file(PathBuf::from(disabled));
}

#[derive(Clone, Debug)]
pub struct LocalFile {
    pub file_name: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub title: Option<String>,
    pub version: Option<String>,
    pub icon_url: Option<String>,
}

pub fn list_local(inst: &Instance, kind: ContentKind) -> Vec<LocalFile> {
    let index = ContentIndex::load(inst);
    let dir = inst.game_dir().join(kind.folder());
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<LocalFile> = entries
        .flatten()
        .filter(|e| e.path().is_file() || kind != ContentKind::Mod)
        .filter_map(|e| {
            let file_name = e.file_name().into_string().ok()?;
            let entry = index.files.get(&ContentIndex::key(kind.folder(), &file_name));
            Some(LocalFile {
                enabled: !file_name.ends_with(".disabled"),
                path: e.path(),
                title: entry.map(|e| e.title.clone()),
                version: entry.map(|e| e.version_number.clone()),
                icon_url: entry.and_then(|e| e.icon_url.clone()),
                file_name,
            })
        })
        .collect();
    out.sort_by_key(|f| f.title.clone().unwrap_or_else(|| f.file_name.clone()).to_lowercase());
    out
}

/// Prism-compatible toggling: a disabled file gets a `.disabled` suffix.
pub fn set_enabled(file: &LocalFile, enabled: bool) -> Result<()> {
    let name = &file.file_name;
    let new_name = match (enabled, name.strip_suffix(".disabled")) {
        (true, Some(base)) => base.to_string(),
        (false, None) => format!("{name}.disabled"),
        _ => return Ok(()),
    };
    std::fs::rename(&file.path, file.path.with_file_name(new_name))?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Update {
    pub file: LocalFile,
    pub version: Version,
}

pub async fn check_updates(inst: &Instance, kind: ContentKind) -> Result<Vec<Update>> {
    let files = list_local(inst, kind);
    let hashed: Vec<(String, LocalFile)> = tokio::task::spawn_blocking(move || {
        files.into_iter().filter(|f| f.path.is_file()).filter_map(|f| net::sha1_file(&f.path).ok().map(|h| (h, f))).collect()
    })
    .await?;
    if hashed.is_empty() {
        return Ok(Vec::new());
    }
    let mut body = json!({
        "hashes": hashed.iter().map(|(h, _)| h).collect::<Vec<_>>(),
        "algorithm": "sha1",
        "game_versions": [inst.mc_version],
    });
    if kind.uses_loader() && !inst.loader.modrinth_names().is_empty() {
        body["loaders"] = json!(inst.loader.modrinth_names());
    }
    let latest: HashMap<String, Version> =
        net::client().post(format!("{API}/version_files/update")).json(&body).send().await?.error_for_status()?.json().await?;
    Ok(hashed
        .into_iter()
        .filter_map(|(hash, file)| {
            let v = latest.get(&hash)?;
            let current = v.files.iter().any(|f| f.hashes.get("sha1") == Some(&hash));
            (!current).then(|| Update { file, version: v.clone() })
        })
        .collect())
}

/// Fills in names and icons for files missing them: older installs, and files added by hand
/// (found on Modrinth by their SHA-1). Returns true if the index changed.
pub async fn identify(inst: &Instance, kind: ContentKind) -> Result<bool> {
    let index = ContentIndex::load(inst);
    let files: Vec<LocalFile> = list_local(inst, kind).into_iter().filter(|f| f.path.is_file()).collect();
    let unknown: Vec<LocalFile> = files.iter().filter(|f| f.title.is_none()).cloned().collect();
    let hashed: Vec<(String, LocalFile)> =
        tokio::task::spawn_blocking(move || unknown.into_iter().filter_map(|f| net::sha1_file(&f.path).ok().map(|h| (h, f))).collect())
            .await?;

    // Hash → version for files we have never seen.
    let mut found: Vec<(String, Version)> = Vec::new();
    if !hashed.is_empty() {
        let by_hash: HashMap<String, Version> = net::client()
            .post(format!("{API}/version_files"))
            .json(&json!({ "hashes": hashed.iter().map(|(h, _)| h).collect::<Vec<_>>(), "algorithm": "sha1" }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for (hash, file) in &hashed {
            if let Some(v) = by_hash.get(hash) {
                found.push((ContentIndex::key(kind.folder(), &file.file_name), v.clone()));
            }
        }
    }

    let missing_icon: Vec<String> = files
        .iter()
        .filter(|f| f.title.is_some() && f.icon_url.is_none())
        .filter_map(|f| index.files.get(&ContentIndex::key(kind.folder(), &f.file_name)).map(|e| e.project_id.clone()))
        .chain(found.iter().map(|(_, v)| v.project_id.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if missing_icon.is_empty() {
        return Ok(false);
    }

    #[derive(Deserialize)]
    struct Project {
        id: String,
        title: String,
        #[serde(default)]
        icon_url: Option<String>,
    }
    let projects: Vec<Project> = net::client()
        .get(format!("{API}/projects"))
        .query(&[("ids", json!(missing_icon).to_string())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let projects: HashMap<String, Project> = projects.into_iter().map(|p| (p.id.clone(), p)).collect();

    ContentIndex::update(inst, |index| {
        for (key, v) in found {
            if let Some(p) = projects.get(&v.project_id) {
                index.files.insert(
                    key,
                    IndexEntry {
                        project_id: v.project_id.clone(),
                        version_id: v.id.clone(),
                        title: p.title.clone(),
                        version_number: v.version_number.clone(),
                        icon_url: p.icon_url.clone().filter(|u| !u.is_empty()),
                    },
                );
            }
        }
        for entry in index.files.values_mut().filter(|e| e.icon_url.is_none()) {
            if let Some(p) = projects.get(&entry.project_id) {
                entry.icon_url = p.icon_url.clone().filter(|u| !u.is_empty());
            }
        }
    })?;
    Ok(true)
}

pub async fn apply_update(inst: &Instance, kind: ContentKind, update: Update, progress: &Progress) -> Result<()> {
    // Files added by hand have no index entry, but Modrinth has their name and icon.
    let meta = match (&update.file.title, &update.file.icon_url) {
        (Some(title), Some(icon)) => ProjectMeta { title: title.clone(), icon_url: Some(icon.clone()) },
        _ => project_meta(&update.version.project_id).await,
    };
    let was_enabled = update.file.enabled;
    let file_name = update.version.primary_file().map(|f| f.filename.clone());
    install(inst, kind, update.version, meta, progress).await?;
    // Remove the old file only after the new one is installed, so a failed or cancelled update
    // keeps the working version.
    let old_name = update.file.file_name.trim_end_matches(".disabled");
    if file_name.as_deref() != Some(old_name) {
        remove_file_variants(&update.file.path.with_file_name(old_name));
        let key = ContentIndex::key(kind.folder(), &update.file.file_name);
        ContentIndex::update(inst, |index| index.files.remove(&key))?;
    }
    if !was_enabled && let Some(name) = file_name {
        let path = inst.game_dir().join(kind.folder()).join(&name);
        let local = LocalFile { file_name: name, path, enabled: true, title: None, version: None, icon_url: None };
        set_enabled(&local, false)?;
    }
    Ok(())
}

pub fn delete_local(inst: &Instance, kind: ContentKind, file: &LocalFile) -> Result<()> {
    if file.path.is_dir() {
        std::fs::remove_dir_all(&file.path)?;
    } else {
        std::fs::remove_file(&file.path)?;
    }
    let key = ContentIndex::key(kind.folder(), &file.file_name);
    ContentIndex::update(inst, |index| index.files.remove(&key))?;
    Ok(())
}

pub fn ensure_folder(inst: &Instance, kind: ContentKind) -> Result<PathBuf> {
    if kind == ContentKind::Modpack {
        bail!("modpacks have no content folder");
    }
    let dir = inst.game_dir().join(kind.folder());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_dependencies_are_named_in_a_warning() {
        let inst = Instance { mc_version: "1.20.1".into(), loader: Loader::Fabric, ..Default::default() };
        let mut installed = Installed { titles: vec!["Sodium".into()], missing: Vec::new() };
        assert_eq!(installed.summary(), "Installed Sodium");
        assert_eq!(installed.warning(&inst), None);

        installed.missing.push("Fabric API".into());
        let warning = installed.warning(&inst).unwrap();
        assert!(warning.contains("Fabric API") && warning.contains("1.20.1"), "{warning}");
    }

    #[test]
    fn facet_building() {
        assert_eq!(
            facets(ContentKind::Mod, Some("1.20.1"), Some(Loader::Quilt)),
            r#"[["project_type:mod"],["versions:1.20.1"],["categories:quilt","categories:fabric"]]"#
        );
        assert_eq!(
            facets(ContentKind::ResourcePack, Some("1.21"), Some(Loader::Fabric)),
            r#"[["project_type:resourcepack"],["versions:1.21"]]"#
        );
        assert_eq!(facets(ContentKind::Mod, None, Some(Loader::Vanilla)), r#"[["project_type:mod"]]"#);
    }

    #[test]
    fn file_names_must_be_plain() {
        assert!(safe_file_name("sodium-0.5.jar").is_ok());
        assert!(safe_file_name("../evil.jar").is_err());
        assert!(safe_file_name("mods/evil.jar").is_err());
        assert!(safe_file_name("/etc/evil.jar").is_err());
        assert!(safe_file_name("C:evil.jar").is_err());
    }

    #[test]
    fn index_keys_ignore_disabled_suffix() {
        assert_eq!(ContentIndex::key("mods", "sodium.jar.disabled"), "mods/sodium.jar");
        assert_eq!(ContentIndex::key("mods", "sodium.jar"), "mods/sodium.jar");
    }
}
