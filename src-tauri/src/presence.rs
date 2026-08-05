//! Discord Rich Presence.
//!
//! Opt-in and deliberately vague: it reports the game and, at most, the profile
//! name. It never says which mods are loaded, because that is nobody else's
//! business and a list of mods is exactly the sort of thing that gets screenshotted
//! into a ban report.

use std::sync::Mutex;

use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

use crate::games::Game;

/// A public application id is required for artwork; without one Discord shows a
/// blank card. This is Roundtable's own.
const APP_ID: &str = "1301992310890004521";

pub struct Presence {
    client: Mutex<Option<DiscordIpcClient>>,
    started: Mutex<Option<i64>>,
}

impl Default for Presence {
    fn default() -> Self {
        Presence {
            client: Mutex::new(None),
            started: Mutex::new(None),
        }
    }
}

impl Presence {
    /// Opens the IPC connection. Failure is fine: Discord simply may not be running.
    pub fn connect(&self) -> bool {
        let mut guard = self.client.lock().unwrap();
        if guard.is_some() {
            return true;
        }
        let Ok(mut client) = DiscordIpcClient::new(APP_ID) else {
            return false;
        };
        match client.connect() {
            Ok(()) => {
                *guard = Some(client);
                true
            }
            Err(_) => false,
        }
    }

    pub fn disconnect(&self) {
        let mut guard = self.client.lock().unwrap();
        if let Some(mut client) = guard.take() {
            let _ = client.close();
        }
        *self.started.lock().unwrap() = None;
    }

    /// Shows a game as being played, with an elapsed timer.
    pub fn set_playing(&self, game: Game, profile: Option<&str>) {
        let mut guard = self.client.lock().unwrap();
        let Some(client) = guard.as_mut() else {
            return;
        };

        let mut stamps = self.started.lock().unwrap();
        let since = *stamps.get_or_insert_with(|| chrono::Utc::now().timestamp());

        let details = game.display_name().to_string();
        let state = profile.map(str::to_string);
        let key = asset_key(game);

        let assets = Assets::new().large_image(key).large_text(game.display_name());
        let mut activity = Activity::new().details(&details).assets(assets);
        if let Some(state) = state.as_deref() {
            activity = activity.state(state);
        }
        activity = activity.timestamps(Timestamps::new().start(since));

        let _ = client.set_activity(activity);
    }

    /// Shows the launcher itself, without a timer.
    pub fn set_browsing(&self) {
        let mut guard = self.client.lock().unwrap();
        let Some(client) = guard.as_mut() else {
            return;
        };
        *self.started.lock().unwrap() = None;

        let assets = Assets::new().large_image("roundtable").large_text("Roundtable");
        let _ = client.set_activity(Activity::new().details("In the launcher").assets(assets));
    }

    pub fn clear(&self) {
        let mut guard = self.client.lock().unwrap();
        if let Some(client) = guard.as_mut() {
            let _ = client.clear_activity();
        }
        *self.started.lock().unwrap() = None;
    }
}

fn asset_key(game: Game) -> &'static str {
    match game {
        Game::EldenRing => "eldenring",
        Game::Nightreign => "nightreign",
        Game::DarkSouls3 => "darksouls3",
        Game::Sekiro => "sekiro",
        Game::ArmoredCore6 => "armoredcore6",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_an_asset_key() {
        for game in Game::ALL {
            let key = asset_key(game);
            assert!(!key.is_empty());
            assert!(key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        }
    }

    #[test]
    fn disconnecting_without_connecting_is_safe() {
        let presence = Presence::default();
        presence.disconnect();
        presence.clear();
        // Setting activity with no client must not panic either.
        presence.set_playing(Game::EldenRing, Some("Test"));
        presence.set_browsing();
    }
}
