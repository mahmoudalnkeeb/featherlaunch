// SPDX-License-Identifier: GPL-3.0-only
//! Accounts and Microsoft sign-in (device code → Xbox Live → XSTS → Minecraft services).
//!
//! The Microsoft refresh token is the only long-lived secret. It lives in the system keyring
//! (Windows Credential Manager, or the Secret Service on Linux), split into chunks because a
//! Windows credential holds at most 2.5 KB.
//! Minecraft access tokens stay in memory and are refreshed when they expire.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::net;
use crate::paths;

/// FeatherLaunch's Microsoft application ID, the one Mojang approved for the Minecraft API. It
/// is a public identifier, not a secret, and deliberately can't be changed per install. Modified
/// builds set their own approved ID with `FEATHERLAUNCH_MSA_CLIENT_ID` at build time.
const CLIENT_ID: &str = match option_env!("FEATHERLAUNCH_MSA_CLIENT_ID") {
    Some(id) => id,
    None => "193567a7-c727-4a8f-b94b-ac72614868e2",
};
const KEYRING_SERVICE: &str = "featherlaunch";
const CHUNK: usize = 1000;
const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    /// Minecraft profile UUID without dashes.
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub skin_url: Option<String>,
    #[serde(default)]
    pub skin_slim: bool,
    #[serde(skip)]
    pub token: Option<String>,
    #[serde(skip)]
    pub token_expires: u64,
    #[serde(skip)]
    pub xuid: String,
}

impl Account {
    pub fn has_valid_token(&self) -> bool {
        self.token.is_some() && now() + 300 < self.token_expires
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountStore {
    pub accounts: Vec<Account>,
    pub selected: Option<String>,
}

impl AccountStore {
    fn path() -> std::path::PathBuf {
        paths::root().join("accounts.json")
    }

    pub fn load() -> Self {
        paths::load_json(&Self::path())
    }

    pub fn save(&self) -> Result<()> {
        paths::write_json(&Self::path(), self)
    }

    /// Offline play is only allowed once a Microsoft account that owns the game has signed in;
    /// Mojang requires this. Microsoft accounts are only stored after the ownership check in
    /// [`fetch_profile`] passes.
    pub fn owns_game(&self) -> bool {
        self.accounts.iter().any(|a| a.kind == AccountKind::Microsoft)
    }

    pub fn selected(&self) -> Option<&Account> {
        let id = self.selected.as_ref()?;
        self.accounts.iter().find(|a| &a.id == id)
    }

    /// Inserts or replaces by id, keeping any in-memory token of the stored copy.
    pub fn upsert(&mut self, account: Account) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.id == account.id) {
            *existing = account;
        } else {
            self.accounts.push(account);
        }
        if self.selected.is_none() {
            self.selected = self.accounts.last().map(|a| a.id.clone());
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
        let _ = delete_secret(id);
        if self.selected.as_deref() == Some(id) {
            self.selected = self.accounts.first().map(|a| a.id.clone());
        }
    }
}

fn entry(account: &str, index: usize) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, &format!("{account}#{index}"))?)
}

/// Splits on character boundaries into chunks of at most [`CHUNK`] bytes, so no chunk ends
/// inside a multi-byte character.
fn split_chunks(secret: &str) -> Vec<String> {
    let mut chunks = vec![String::new()];
    for c in secret.chars() {
        if chunks.last().is_some_and(|last| last.len() + c.len_utf8() > CHUNK) {
            chunks.push(String::new());
        }
        if let Some(last) = chunks.last_mut() {
            last.push(c);
        }
    }
    chunks
}

fn stored_count(account: &str) -> usize {
    entry(account, 0).ok().and_then(|e| e.get_password().ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Overwrites the chunks in place and the count last, instead of deleting first: a keyring that
/// refuses writes (locked or unavailable) fails on the first one and keeps the old sign-in.
pub fn store_secret(account: &str, secret: &str) -> Result<()> {
    let old_count = stored_count(account);
    let chunks = split_chunks(secret);
    for (i, chunk) in chunks.iter().enumerate() {
        entry(account, i + 1)?.set_password(chunk)?;
    }
    entry(account, 0)?.set_password(&chunks.len().to_string())?;
    for i in chunks.len() + 1..=old_count {
        let _ = entry(account, i)?.delete_credential();
    }
    Ok(())
}

pub fn load_secret(account: &str) -> Result<String> {
    let count: usize = entry(account, 0)?.get_password()?.parse()?;
    let mut out = String::new();
    for i in 1..=count {
        out.push_str(&entry(account, i)?.get_password()?);
    }
    Ok(out)
}

fn delete_secret(account: &str) -> Result<()> {
    for i in 0..=stored_count(account) {
        let _ = entry(account, i)?.delete_credential();
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct MsToken {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct MsError {
    error: String,
    #[serde(default)]
    error_description: String,
}

const MS_BASE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";

pub async fn start_device_login() -> Result<DeviceCode> {
    let resp = net::client().post(format!("{MS_BASE}/devicecode")).form(&[("client_id", CLIENT_ID), ("scope", SCOPE)]).send().await?;
    if !resp.status().is_success() {
        let err: MsError = resp.json().await.unwrap_or(MsError { error: "unknown".into(), error_description: String::new() });
        bail!("Microsoft rejected the sign-in request: {} {}", err.error, err.error_description);
    }
    Ok(resp.json().await?)
}

/// Polls until the user has entered the code, then finishes the whole chain.
/// Returns `None` when `cancel` is set before sign-in completes.
pub async fn finish_device_login(code: &DeviceCode, cancel: &AtomicBool) -> Result<Option<Account>> {
    let deadline = now() + code.expires_in;
    let mut interval = code.interval.max(1);
    let ms = loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if now() > deadline {
            bail!("The sign-in code expired. Try again.");
        }
        let resp = net::client()
            .post(format!("{MS_BASE}/token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", code.device_code.as_str()),
            ])
            .send()
            .await?;
        if resp.status().is_success() {
            break resp.json::<MsToken>().await?;
        }
        let err: MsError = resp.json().await?;
        match err.error.as_str() {
            "authorization_pending" => continue,
            "slow_down" => interval += 5,
            "authorization_declined" => bail!("Sign-in was cancelled."),
            "expired_token" => bail!("The sign-in code expired. Try again."),
            _ => bail!("Sign-in failed: {} {}", err.error, err.error_description),
        }
    };
    complete(ms).await.map(Some)
}

pub async fn refresh(account: &Account) -> Result<Account> {
    let refresh_token = load_secret(&account.id).context("no saved sign-in for this account; sign in again")?;
    let resp = net::client()
        .post(format!("{MS_BASE}/token"))
        .form(&[("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", refresh_token.as_str()), ("scope", SCOPE)])
        .send()
        .await?;
    if !resp.status().is_success() {
        let err: MsError = resp.json().await?;
        bail!("Your Microsoft sign-in expired ({}). Remove the account and sign in again.", err.error);
    }
    complete(resp.json().await?).await
}

pub async fn ensure_token(account: Account) -> Result<Account> {
    if account.kind == AccountKind::Offline || account.has_valid_token() {
        return Ok(account);
    }
    refresh(&account).await
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XblResponse {
    token: String,
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    uhs: String,
    #[serde(default)]
    xid: Option<String>,
}

#[derive(Deserialize)]
struct McLogin {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<Skin>,
}

#[derive(Deserialize)]
pub struct Skin {
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub variant: String,
}

async fn complete(ms: MsToken) -> Result<Account> {
    let client = net::client();
    let xbl: XblResponse = client
        .post("https://user.auth.xboxlive.com/user/authenticate")
        .json(&json!({
            "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={}", ms.access_token)},
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }))
        .send()
        .await?
        .error_for_status()
        .context("Xbox Live sign-in failed")?
        .json()
        .await?;

    let resp = client
        .post("https://xsts.auth.xboxlive.com/xsts/authorize")
        .json(&json!({
            "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl.token]},
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()
        .await?;
    if resp.status().as_u16() == 401 {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg = match body["XErr"].as_u64() {
            Some(2148916233) => "This Microsoft account has no Xbox profile. Sign in once at minecraft.net first.",
            Some(2148916235) => "Xbox Live is not available in your country.",
            Some(2148916236 | 2148916237) => "This account needs adult verification on xbox.com.",
            Some(2148916238) => "This is a child account; an adult must add it to a Microsoft family.",
            _ => "Xbox authorization was denied.",
        };
        bail!(msg);
    }
    let xsts: XblResponse = resp.error_for_status().context("Xbox authorization failed")?.json().await?;
    let xui = xsts.display_claims.xui.first().ok_or_else(|| anyhow!("Xbox response had no user"))?;

    let mc: McLogin = client
        .post("https://api.minecraftservices.com/authentication/login_with_xbox")
        .json(&json!({ "identityToken": format!("XBL3.0 x={};{}", xui.uhs, xsts.token) }))
        .send()
        .await?
        .error_for_status()
        .context("Minecraft sign-in failed. Please try again later.")?
        .json()
        .await?;

    let profile = fetch_profile(&mc.access_token).await?;
    let (skin_url, skin_slim) = active_skin(&profile);
    let account = Account {
        id: profile.id.clone(),
        name: profile.name.clone(),
        kind: AccountKind::Microsoft,
        skin_url,
        skin_slim,
        token: Some(mc.access_token),
        token_expires: now() + mc.expires_in,
        xuid: xui.xid.clone().unwrap_or_default(),
    };
    store_secret(&account.id, &ms.refresh_token).context(
        "saving the sign-in to the system keyring (Windows Credential Manager, or a Secret Service such as GNOME Keyring on Linux)",
    )?;
    Ok(account)
}

pub async fn fetch_profile(token: &str) -> Result<Profile> {
    let resp = net::client().get("https://api.minecraftservices.com/minecraft/profile").bearer_auth(token).send().await?;
    if resp.status().as_u16() == 404 {
        bail!("This Microsoft account does not own Minecraft: Java Edition.");
    }
    Ok(resp.error_for_status()?.json().await?)
}

pub async fn upload_skin(account: &Account, png: Vec<u8>, slim: bool) -> Result<Account> {
    let token = account.token.as_deref().context("account is not signed in")?;
    let form = reqwest::multipart::Form::new()
        .text("variant", if slim { "slim" } else { "classic" })
        .part("file", reqwest::multipart::Part::bytes(png).file_name("skin.png").mime_str("image/png")?);
    net::client()
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await?
        .error_for_status()
        .context("uploading the skin failed")?;
    let profile = fetch_profile(token).await?;
    let mut updated = account.clone();
    (updated.skin_url, updated.skin_slim) = active_skin(&profile);
    Ok(updated)
}

/// The active skin's URL (upgraded to HTTPS) and whether it uses the slim arm model.
fn active_skin(profile: &Profile) -> (Option<String>, bool) {
    let skin = profile.skins.iter().find(|s| s.state == "ACTIVE");
    (skin.map(|s| s.url.replace("http://", "https://")), skin.is_some_and(|s| s.variant.eq_ignore_ascii_case("slim")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, kind: AccountKind) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
            kind,
            skin_url: None,
            skin_slim: false,
            token: None,
            token_expires: 0,
            xuid: String::new(),
        }
    }

    #[test]
    fn offline_play_needs_a_microsoft_account() {
        let mut store = AccountStore::default();
        assert!(!store.owns_game());
        store.accounts.push(account("legacy", AccountKind::Offline));
        assert!(!store.owns_game(), "an old offline account alone must not unlock offline play");
        store.accounts.push(account("steve", AccountKind::Microsoft));
        assert!(store.owns_game());
    }

    #[test]
    fn secret_chunks_keep_characters_whole() {
        let ascii = "a".repeat(CHUNK * 2 + 1);
        let chunks = split_chunks(&ascii);
        assert_eq!(chunks.iter().map(String::len).collect::<Vec<_>>(), [CHUNK, CHUNK, 1]);
        let mixed = "é".repeat(CHUNK);
        let chunks = split_chunks(&mixed);
        assert!(chunks.iter().all(|c| c.len() <= CHUNK));
        assert_eq!(chunks.concat(), mixed);
    }
}
