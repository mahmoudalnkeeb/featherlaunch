// SPDX-License-Identifier: GPL-3.0-only
//! CurseForge API, for builds with an API key: set `FEATHERLAUNCH_CURSEFORGE_KEY` when building.
//! Without one, everything here stays out of sight. CurseForge projects use the types Modrinth
//! results do, with their IDs marked by [`PREFIX`], so installing and updating work the same way.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use zip::ZipArchive;

use crate::log;
use crate::minecraft::instance::{Instance, Loader, PackSource};
use crate::modplatform::import::extract_under;
use crate::modplatform::modrinth::{
    self, ContentIndex, ContentKind, Dependency, ProjectMeta, SearchHit, SearchResponse, Update, Version, VersionFile,
};
use crate::modplatform::mrpack::PartialInstance;
use crate::net::{self, Download};
use crate::tasks::Progress;

const API: &str = "https://api.curseforge.com/v1";
const MINECRAFT: u32 = 432;
const PAGE_SIZE: u64 = 20;
/// CurseForge refuses to page past this many results.
const MAX_RESULTS: u64 = 10_000;
const SHA1: u32 = 1;
const REQUIRED_DEPENDENCY: u32 = 3;
/// Marks a project ID as CurseForge's.
pub const PREFIX: &str = "cf:";

fn key() -> Option<&'static str> {
    option_env!("FEATHERLAUNCH_CURSEFORGE_KEY").filter(|k| !k.is_empty())
}

pub fn available() -> bool {
    key().is_some()
}

fn class_id(kind: ContentKind) -> u32 {
    match kind {
        ContentKind::Mod => 6,
        ContentKind::ResourcePack => 12,
        ContentKind::ShaderPack => 6552,
        ContentKind::Modpack => 4471,
        ContentKind::DataPack => 6945,
    }
}

fn folder_of_class(class: Option<u32>) -> &'static str {
    let kinds = [ContentKind::ResourcePack, ContentKind::ShaderPack];
    let kind = kinds.into_iter().find(|k| Some(class_id(*k)) == class).unwrap_or(ContentKind::Mod);
    kind.folder()
}

fn loader_type(loader: Loader) -> Option<u32> {
    match loader {
        Loader::Vanilla => None,
        Loader::Forge => Some(1),
        Loader::Fabric => Some(4),
        Loader::Quilt => Some(5),
        Loader::NeoForge => Some(6),
    }
}

#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mod {
    id: u64,
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    authors: Vec<Author>,
    #[serde(default)]
    logo: Option<Logo>,
    #[serde(default)]
    download_count: u64,
    #[serde(default)]
    class_id: Option<u32>,
    #[serde(default)]
    links: Links,
}

#[derive(Deserialize)]
struct Author {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Logo {
    thumbnail_url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Links {
    website_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFile {
    id: u64,
    mod_id: u64,
    display_name: String,
    file_name: String,
    /// Empty when the author doesn't allow downloads outside CurseForge.
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<FileHash>,
    #[serde(default)]
    file_length: u64,
    #[serde(default)]
    file_date: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<FileDependency>,
}

#[derive(Deserialize)]
struct FileHash {
    value: String,
    algo: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDependency {
    mod_id: u64,
    relation_type: u32,
}

#[derive(Deserialize)]
struct SearchPage {
    data: Vec<Mod>,
    pagination: Pagination,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    total_count: u64,
}

fn request(builder: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
    Ok(builder.header("x-api-key", key().context("this build has no CurseForge API key")?))
}

async fn get<T: DeserializeOwned>(path: &str, query: &[(&str, String)]) -> Result<T> {
    Ok(request(net::client().get(format!("{API}{path}")).query(query))?.send().await?.error_for_status()?.json().await?)
}

async fn post<T: DeserializeOwned>(path: &str, body: serde_json::Value) -> Result<T> {
    Ok(request(net::client().post(format!("{API}{path}")).json(&body))?.send().await?.error_for_status()?.json().await?)
}

fn hit(m: Mod) -> SearchHit {
    SearchHit {
        project_id: format!("{PREFIX}{}", m.id),
        title: m.name,
        description: m.summary,
        author: m.authors.into_iter().next().map(|a| a.name).unwrap_or_default(),
        downloads: m.download_count,
        icon_url: m.logo.and_then(|l| l.thumbnail_url),
    }
}

fn version(file: CfFile) -> Version {
    let sha1 = file.hashes.iter().find(|h| h.algo == SHA1).map(|h| h.value.clone());
    let files = file
        .download_url
        .map(|url| VersionFile {
            url,
            filename: file.file_name,
            primary: true,
            hashes: sha1.into_iter().map(|h| ("sha1".to_string(), h)).collect(),
            size: file.file_length,
        })
        .into_iter()
        .collect();
    let dependencies = file
        .dependencies
        .iter()
        .filter(|d| d.relation_type == REQUIRED_DEPENDENCY)
        .map(|d| Dependency {
            project_id: Some(format!("{PREFIX}{}", d.mod_id)),
            version_id: None,
            dependency_type: "required".to_string(),
        })
        .collect();
    // Loaders and game versions share this list.
    let (game_versions, loaders) = file.game_versions.into_iter().partition(|v| v.starts_with(|c: char| c.is_ascii_digit()));
    Version {
        id: file.id.to_string(),
        project_id: format!("{PREFIX}{}", file.mod_id),
        name: file.display_name.clone(),
        version_number: file.display_name,
        files,
        dependencies,
        loaders,
        game_versions,
        changelog: None,
    }
}

pub async fn search(query: &str, kind: ContentKind, mc: Option<&str>, loader: Option<Loader>, offset: u64) -> Result<SearchResponse> {
    let mut params = vec![
        ("gameId", MINECRAFT.to_string()),
        ("classId", class_id(kind).to_string()),
        ("searchFilter", query.to_string()),
        ("sortOrder", "desc".to_string()),
        ("index", offset.to_string()),
        ("pageSize", PAGE_SIZE.to_string()),
    ];
    if let Some(mc) = mc {
        params.push(("gameVersion", mc.to_string()));
    }
    if kind == ContentKind::Mod
        && let Some(loader) = loader.and_then(loader_type)
    {
        params.push(("modLoaderType", loader.to_string()));
    }
    let page: SearchPage = get("/mods/search", &params).await?;
    Ok(SearchResponse { hits: page.data.into_iter().map(hit).collect(), total_hits: page.pagination.total_count.min(MAX_RESULTS) })
}

/// A project's files, newest first.
pub async fn versions(project: &str, kind: ContentKind, mc: Option<&str>, loader: Option<Loader>) -> Result<Vec<Version>> {
    let mut params = vec![("pageSize", "50".to_string())];
    if let Some(mc) = mc {
        params.push(("gameVersion", mc.to_string()));
    }
    if kind == ContentKind::Mod
        && let Some(loader) = loader.and_then(loader_type)
    {
        params.push(("modLoaderType", loader.to_string()));
    }
    let mut files = get::<Data<Vec<CfFile>>>(&format!("/mods/{project}/files"), &params).await?.data;
    files.sort_by(|a, b| b.file_date.cmp(&a.file_date));
    Ok(files.into_iter().map(version).collect())
}

pub async fn meta(project: &str) -> Result<ProjectMeta> {
    let m = get::<Data<Mod>>(&format!("/mods/{project}"), &[]).await?.data;
    Ok(hit(m).meta())
}

/// Newer files for what was installed from CurseForge. Mods from elsewhere are Modrinth's to check.
pub async fn updates(inst: &Instance, kind: ContentKind) -> Result<Vec<Update>> {
    if !available() {
        return Ok(Vec::new());
    }
    let index = ContentIndex::load(inst);
    let mut out = Vec::new();
    for file in modrinth::list_local(inst, kind) {
        let Some(entry) = index.files.get(&ContentIndex::key(kind.folder(), &file.file_name)) else { continue };
        let Some(project) = entry.project_id.strip_prefix(PREFIX) else { continue };
        let newest = versions(project, kind, Some(&inst.mc_version), Some(inst.loader)).await?.into_iter().next();
        if let Some(version) = newest.filter(|v| v.id != entry.version_id && !v.files.is_empty()) {
            out.push(Update { file, version });
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    name: String,
    #[serde(default)]
    version: String,
    minecraft: ManifestGame,
    files: Vec<ManifestFile>,
    #[serde(default)]
    overrides: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestGame {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<ManifestLoader>,
}

#[derive(Debug, Deserialize)]
struct ManifestLoader {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    #[serde(rename = "fileID")]
    file_id: u64,
}

/// `forge-47.2.0` → Forge, 47.2.0.
fn parse_loader(id: &str) -> Result<(Loader, String)> {
    let (name, version) = id.split_once('-').with_context(|| format!("unknown mod loader {id}"))?;
    let loader = match name {
        "forge" => Loader::Forge,
        "neoforge" => Loader::NeoForge,
        "fabric" => Loader::Fabric,
        "quilt" => Loader::Quilt,
        _ => bail!("unknown mod loader {id}"),
    };
    Ok((loader, version.to_string()))
}

/// The message for mods whose authors only allow downloads from CurseForge's own site.
fn blocked_message(blocked: &[String]) -> Option<String> {
    (!blocked.is_empty()).then(|| format!("Download these mods from CurseForge and add them to the instance:\n\n{}", blocked.join("\n")))
}

/// Makes an instance from a CurseForge pack zip. Mods that can't be downloaded here are left out
/// and named in the returned message.
pub async fn import_pack(pack: &Path, group: String, parallel: usize, progress: &Progress) -> Result<(Instance, Option<String>)> {
    if !available() {
        bail!("This build of FeatherLaunch can't download CurseForge packs");
    }
    let mut archive = ZipArchive::new(File::open(pack)?)?;
    let manifest: Manifest = {
        let entry = archive.by_name("manifest.json").context("not a CurseForge pack (no manifest.json)")?;
        serde_json::from_reader(entry).context("reading manifest.json")?
    };
    let (loader, loader_version) = match manifest.minecraft.mod_loaders.first() {
        Some(l) => parse_loader(&l.id).map(|(loader, version)| (loader, Some(version)))?,
        None => (Loader::Vanilla, None),
    };
    let name = if manifest.version.is_empty() { manifest.name.clone() } else { format!("{} {}", manifest.name, manifest.version) };
    let source = PackSource { title: manifest.name.clone(), version: manifest.version.clone(), ..Default::default() };
    let mc_version = manifest.minecraft.version.clone();
    let inst = Instance { name, group, mc_version, loader, loader_version, pack: Some(source), ..Default::default() }.create()?;
    let game_dir = inst.game_dir();
    let partial = PartialInstance::new(inst);

    progress.set_label("Looking up modpack files");
    let ids: Vec<u64> = manifest.files.iter().map(|f| f.file_id).collect();
    let files = post::<Data<Vec<CfFile>>>("/mods/files", json!({ "fileIds": ids })).await?.data;
    let mod_ids: Vec<u64> = files.iter().map(|f| f.mod_id).collect();
    let mods = post::<Data<Vec<Mod>>>("/mods", json!({ "modIds": mod_ids })).await?.data;
    let mods: HashMap<u64, Mod> = mods.into_iter().map(|m| (m.id, m)).collect();

    let mut items = Vec::new();
    let mut blocked = Vec::new();
    for file in files {
        let m = mods.get(&file.mod_id);
        let name = modrinth::safe_file_name(&file.file_name)?;
        let Some(url) = &file.download_url else {
            let title = m.map_or(file.display_name.as_str(), |m| m.name.as_str());
            blocked.push(format!("{title}: {}", m.and_then(|m| m.links.website_url.as_deref()).unwrap_or("no link")));
            continue;
        };
        let folder = folder_of_class(m.and_then(|m| m.class_id));
        let sha1 = file.hashes.iter().find(|h| h.algo == SHA1).map(|h| h.value.clone());
        items.push(Download { url: url.clone(), path: game_dir.join(folder).join(name), sha1, size: Some(file.file_length) });
    }
    progress.set_label(format!("Downloading {} modpack files", items.len()));
    net::download_all(items, parallel, progress).await?;

    progress.set_label("Applying modpack overrides");
    // Extraction runs on a blocking thread that a cancel would not stop.
    let _critical = progress.critical();
    let overrides = format!("{}/", manifest.overrides.as_deref().unwrap_or("overrides"));
    tokio::task::spawn_blocking(move || extract_under(&mut archive, &overrides, &game_dir)).await??;
    if !blocked.is_empty() {
        log::error(&format!("{} mods can't be downloaded here: {}", blocked.len(), blocked.join(", ")));
    }
    Ok((partial.keep(), blocked_message(&blocked)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{"id":5,"modId":7,"displayName":"Sodium 1.0","fileName":"sodium.jar",
        "downloadUrl":"https://edge.forgecdn.net/files/sodium.jar","fileLength":10,"fileDate":"2026-01-02T00:00:00Z",
        "hashes":[{"value":"md5hash","algo":2},{"value":"sha1hash","algo":1}],
        "gameVersions":["1.21.1","Fabric","Client"],
        "dependencies":[{"modId":9,"relationType":3},{"modId":10,"relationType":2}]}"#;

    #[test]
    fn maps_a_file_to_a_version() {
        let v = version(serde_json::from_str(FILE).unwrap());
        assert_eq!((v.id.as_str(), v.project_id.as_str()), ("5", "cf:7"));
        assert_eq!(v.game_versions, ["1.21.1"]);
        assert_eq!(v.loaders, ["Fabric", "Client"]);
        let file = v.primary_file().unwrap();
        assert_eq!(file.hashes.get("sha1").map(String::as_str), Some("sha1hash"));
        assert_eq!(v.dependencies.len(), 1);
        assert_eq!(v.dependencies[0].project_id.as_deref(), Some("cf:9"));
    }

    #[test]
    fn a_blocked_file_has_nothing_to_download() {
        let json = r#"{"id":5,"modId":7,"displayName":"A","fileName":"a.jar","downloadUrl":null}"#;
        assert!(version(serde_json::from_str(json).unwrap()).files.is_empty());
    }

    #[test]
    fn maps_a_project_to_a_result() {
        let json = r#"{"id":7,"name":"Sodium","summary":"Fast","authors":[{"name":"jelly"}],
            "logo":{"thumbnailUrl":"https://media.forgecdn.net/a.png"},"downloadCount":12,"classId":6}"#;
        let h = hit(serde_json::from_str(json).unwrap());
        assert_eq!((h.project_id.as_str(), h.author.as_str(), h.downloads), ("cf:7", "jelly", 12));
    }

    #[test]
    fn reads_a_pack_manifest() {
        let json = r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]},
            "manifestType":"minecraftModpack","name":"Pack","version":"2","overrides":"overrides",
            "files":[{"projectID":1,"fileID":2,"required":true}]}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.files[0].file_id, 2);
        assert_eq!(parse_loader(&m.minecraft.mod_loaders[0].id).unwrap(), (Loader::Forge, "47.2.0".to_string()));
        assert!(parse_loader("rift-1").is_err());
    }

    #[test]
    fn sorts_files_into_their_folders() {
        assert_eq!(folder_of_class(Some(6)), "mods");
        assert_eq!(folder_of_class(Some(12)), "resourcepacks");
        assert_eq!(folder_of_class(Some(6552)), "shaderpacks");
        assert_eq!(folder_of_class(None), "mods");
    }

    #[test]
    fn names_blocked_mods() {
        assert_eq!(blocked_message(&[]), None);
        assert!(blocked_message(&["A: https://x".to_string()]).unwrap().contains("A: https://x"));
    }
}
