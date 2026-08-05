//! Nexus Mods API client.
//!
//! Uses the public v1 API with a personal API key, which the user pastes from their
//! Nexus account page. The key is sent only to `api.nexusmods.com`.
//!
//! Download links are only issued to Premium accounts by the API. For everyone else
//! Nexus expects the browser "Mod Manager Download" button, which hands the app an
//! `nxm://` link containing a short-lived key. Both paths are supported.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const BASE: &str = "https://api.nexusmods.com/v1";
/// Nexus keys every game by a slug rather than a Steam id.
pub const ELDEN_RING: &str = "eldenring";

fn domain_for(game: crate::games::Game) -> &'static str {
    use crate::games::Game;
    match game {
        Game::EldenRing => ELDEN_RING,
        Game::Nightreign => "eldenringnightreign",
        Game::DarkSouls3 => "darksouls3",
        Game::Sekiro => "sekiro",
        Game::ArmoredCore6 => "armoredcore6firesofrubicon",
    }
}

fn request(
    client: &reqwest::Client,
    api_key: &str,
    url: String,
) -> reqwest::RequestBuilder {
    client
        .get(url)
        .header("apikey", api_key)
        .header("accept", "application/json")
        .header("application-name", "Roundtable")
        .header("application-version", env!("CARGO_PKG_VERSION"))
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    api_key: &str,
    url: String,
) -> Result<T> {
    let response = request(client, api_key, url).send().await?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Nexus(match status.as_u16() {
            401 => "the API key was rejected. Paste a fresh one from your Nexus account page.".into(),
            403 => "this account is not allowed to do that. Download links through the API need Nexus Premium.".into(),
            404 => "not found on Nexus.".into(),
            429 => "rate limited by Nexus. Wait a few minutes and try again.".into(),
            _ => format!("{status}: {}", body.chars().take(200).collect::<String>()),
        }));
    }

    Ok(response.json().await?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    #[serde(rename = "user_id")]
    pub user_id: u64,
    pub name: String,
    #[serde(rename = "is_premium")]
    pub is_premium: bool,
    #[serde(rename = "is_supporter")]
    pub is_supporter: bool,
}

/// Confirms a key works and reports what it can do.
pub async fn validate(client: &reqwest::Client, api_key: &str) -> Result<Account> {
    if api_key.trim().is_empty() {
        return Err(Error::Nexus("no API key has been set".into()));
    }
    get_json(client, api_key, format!("{BASE}/users/validate.json")).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub mod_id: u32,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub picture_url: Option<String>,
    pub available: bool,
    pub endorsement_count: Option<u32>,
}

pub async fn mod_info(
    client: &reqwest::Client,
    api_key: &str,
    game: crate::games::Game,
    mod_id: u32,
) -> Result<ModInfo> {
    let domain = domain_for(game);
    get_json(
        client,
        api_key,
        format!("{BASE}/games/{domain}/mods/{mod_id}.json"),
    )
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
    pub category_name: Option<String>,
    pub size_kb: Option<u64>,
    pub uploaded_time: Option<String>,
    pub is_primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FileList {
    files: Vec<ModFile>,
}

pub async fn mod_files(
    client: &reqwest::Client,
    api_key: &str,
    game: crate::games::Game,
    mod_id: u32,
) -> Result<Vec<ModFile>> {
    let domain = domain_for(game);
    let list: FileList = get_json(
        client,
        api_key,
        format!("{BASE}/games/{domain}/mods/{mod_id}/files.json"),
    )
    .await?;
    Ok(list.files)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLink {
    #[serde(rename = "URI")]
    pub uri: String,
    pub name: Option<String>,
    pub short_name: Option<String>,
}

/// Asks Nexus for a download URL.
///
/// Without `nxm` parameters this only succeeds for Premium accounts; free accounts
/// must go through the site's "Mod Manager Download" button, which produces the
/// key and expiry this function then passes through.
pub async fn download_links(
    client: &reqwest::Client,
    api_key: &str,
    game: crate::games::Game,
    mod_id: u32,
    file_id: u64,
    nxm_key: Option<&str>,
    expires: Option<u64>,
) -> Result<Vec<DownloadLink>> {
    let domain = domain_for(game);
    let mut url = format!(
        "{BASE}/games/{domain}/mods/{mod_id}/files/{file_id}/download_link.json"
    );
    if let (Some(key), Some(expires)) = (nxm_key, expires) {
        url.push_str(&format!("?key={key}&expires={expires}"));
    }
    get_json(client, api_key, url).await
}

/// A parsed `nxm://` link, which is what the Nexus website hands to a mod manager.
///
/// Shape: `nxm://<domain>/mods/<mod_id>/files/<file_id>?key=<key>&expires=<unix>`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxmLink {
    pub domain: String,
    pub mod_id: u32,
    pub file_id: u64,
    pub key: Option<String>,
    pub expires: Option<u64>,
}

pub fn parse_nxm(link: &str) -> Result<NxmLink> {
    let rest = link
        .strip_prefix("nxm://")
        .ok_or_else(|| Error::Nexus("not an nxm:// link".into()))?;

    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };

    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // domain / "mods" / id / "files" / id
    if parts.len() < 5 || parts[1] != "mods" || parts[3] != "files" {
        return Err(Error::Nexus(format!("malformed nxm link: {link}")));
    }

    let mod_id = parts[2]
        .parse::<u32>()
        .map_err(|_| Error::Nexus(format!("bad mod id in {link}")))?;
    let file_id = parts[4]
        .parse::<u64>()
        .map_err(|_| Error::Nexus(format!("bad file id in {link}")))?;

    let mut key = None;
    let mut expires = None;
    if let Some(query) = query {
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("key", value)) => key = Some(value.to_string()),
                Some(("expires", value)) => expires = value.parse::<u64>().ok(),
                _ => {}
            }
        }
    }

    Ok(NxmLink {
        domain: parts[0].to_string(),
        mod_id,
        file_id,
        key,
        expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::Game;

    #[test]
    fn every_supported_game_has_a_nexus_domain() {
        for game in Game::ALL {
            let domain = domain_for(game);
            assert!(!domain.is_empty());
            // Nexus slugs are lowercase alphanumeric: "darksouls3", "armoredcore6…".
            assert!(
                domain.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "unexpected characters in {domain}"
            );
        }
        assert_eq!(domain_for(Game::EldenRing), "eldenring");
        assert_eq!(domain_for(Game::DarkSouls3), "darksouls3");
    }

    #[test]
    fn a_full_nxm_link_parses() {
        let link = parse_nxm(
            "nxm://eldenring/mods/541/files/12345?key=abc123&expires=1800000000",
        )
        .unwrap();
        assert_eq!(link.domain, "eldenring");
        assert_eq!(link.mod_id, 541);
        assert_eq!(link.file_id, 12345);
        assert_eq!(link.key.as_deref(), Some("abc123"));
        assert_eq!(link.expires, Some(1_800_000_000));
    }

    #[test]
    fn a_link_without_a_key_still_parses() {
        let link = parse_nxm("nxm://eldenring/mods/1/files/2").unwrap();
        assert_eq!(link.mod_id, 1);
        assert_eq!(link.file_id, 2);
        assert!(link.key.is_none());
        assert!(link.expires.is_none());
    }

    #[test]
    fn malformed_links_are_rejected_rather_than_guessed_at() {
        for bad in [
            "https://nexusmods.com/eldenring/mods/541",
            "nxm://eldenring/mods/541",
            "nxm://eldenring/files/541/mods/1",
            "nxm://eldenring/mods/abc/files/1",
            "nxm://eldenring/mods/1/files/xyz",
            "nxm://",
        ] {
            assert!(parse_nxm(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn an_empty_key_is_reported_before_any_request() {
        // Catching this locally avoids a pointless round trip and a confusing 401.
        let client = reqwest::Client::new();
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(validate(&client, "   "));
        assert!(matches!(outcome, Err(Error::Nexus(_))));
    }
}
