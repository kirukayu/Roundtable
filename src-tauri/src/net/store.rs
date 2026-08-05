//! Steam store metadata.
//!
//! The store API is unreachable on plenty of networks, so nothing here is on a
//! critical path: the interface already has pinned artwork and trailer URLs. This
//! only refreshes them when the network allows, and fails quietly when it does not.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::games::Game;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trailer {
    pub id: u64,
    pub name: String,
    pub thumbnail: String,
    /// Plain mp4 on the content CDN, which plays without an HLS or DASH shim.
    pub mp4: String,
}

#[derive(Debug, Deserialize)]
struct AppDetails {
    success: bool,
    data: Option<AppData>,
}

#[derive(Debug, Deserialize)]
struct AppData {
    #[serde(default)]
    movies: Vec<Movie>,
}

#[derive(Debug, Deserialize)]
struct Movie {
    id: u64,
    name: String,
    #[serde(default)]
    thumbnail: String,
    #[serde(default)]
    highlight: bool,
}

/// The CDN path a movie id resolves to.
pub fn mp4_for(movie_id: u64) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{movie_id}/movie480.mp4")
}

/// Fetches the trailer list for a game. Highlighted entries come first, because
/// those are the ones Steam itself features.
pub async fn trailers(client: &reqwest::Client, game: Game) -> Result<Vec<Trailer>> {
    let url = format!(
        "https://store.steampowered.com/api/appdetails?appids={}&filters=movies",
        game.steam_app_id()
    );

    let response = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(12))
        .send()
        .await
        .map_err(|e| Error::Network(format!("the Steam store is unreachable: {e}")))?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "the Steam store replied {}",
            response.status()
        )));
    }

    let body: std::collections::HashMap<String, AppDetails> = response.json().await?;
    let details = body
        .get(&game.steam_app_id().to_string())
        .ok_or_else(|| Error::Network("the store returned nothing for this app".into()))?;

    if !details.success {
        return Err(Error::Network("the store has no entry for this app".into()));
    }

    let mut movies = details
        .data
        .as_ref()
        .map(|data| data.movies.clone_vec())
        .unwrap_or_default();

    movies.sort_by(|a, b| b.highlight.cmp(&a.highlight));

    Ok(movies
        .into_iter()
        .map(|movie| Trailer {
            mp4: mp4_for(movie.id),
            id: movie.id,
            name: movie.name,
            thumbnail: movie.thumbnail,
        })
        .collect())
}

/// `Movie` is not `Clone` because it is only ever read once; this keeps the
/// borrow above readable without deriving Clone on the wire type.
trait CloneVec {
    fn clone_vec(&self) -> Vec<Movie>;
}

impl CloneVec for Vec<Movie> {
    fn clone_vec(&self) -> Vec<Movie> {
        self.iter()
            .map(|movie| Movie {
                id: movie.id,
                name: movie.name.clone(),
                thumbnail: movie.thumbnail.clone(),
                highlight: movie.highlight,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cdn_path_is_a_plain_mp4() {
        let url = mp4_for(256_875_477);
        assert!(url.starts_with("https://cdn.cloudflare.steamstatic.com/"));
        assert!(url.ends_with("/movie480.mp4"));
        assert!(url.contains("256875477"));
    }

    #[test]
    fn highlighted_trailers_sort_first() {
        let mut movies = vec![
            Movie { id: 1, name: "B roll".into(), thumbnail: String::new(), highlight: false },
            Movie { id: 2, name: "Launch".into(), thumbnail: String::new(), highlight: true },
        ];
        movies.sort_by(|a, b| b.highlight.cmp(&a.highlight));
        assert_eq!(movies[0].id, 2);
    }

    #[test]
    fn every_pinned_trailer_points_at_the_cdn() {
        for game in Game::ALL {
            let url = game.trailer_url();
            assert!(url.starts_with("https://cdn.cloudflare.steamstatic.com/"));
            assert!(url.ends_with(".mp4"));
        }
    }
}
