//! Proving two players can actually see each other.
//!
//! "Failed, no session found" is the most common thing that goes wrong with
//! Seamless Co-op, and the mod's own FAQ gives the reason: everyone has to be on
//! the same game build, the same co-op version, the same `regulation.bin` when a
//! mod alters it, and the same password. Four facts, none of which either player
//! can see, spread across an executable's version resource, a DLL, a two
//! megabyte binary and an ini file.
//!
//! So this reads all four and prints them as a short block you paste to a
//! friend. Paste theirs back and it says which line differs, rather than leaving
//! two people to guess at each other's setup over voice chat.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::coop;
use crate::game::Installation;

/// One line of the block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trait {
    pub key: String,
    pub label: String,
    pub value: String,
    /// What to say when this is the line that does not match.
    pub matters: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub traits: Vec<Trait>,
    /// The block to paste into chat.
    pub block: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Every line agreed.
    Match,
    /// At least one line differs.
    Differs,
    /// Their block was not readable.
    Unreadable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub label: String,
    pub mine: String,
    pub theirs: String,
    pub matters: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub verdict: Verdict,
    pub differences: Vec<Difference>,
    /// Lines only one side reported, which usually means different versions of
    /// Roundtable rather than a real mismatch.
    pub unknown: Vec<String>,
}

/// Eight hex characters of SHA-256. Enough to tell two files apart by eye,
/// short enough to read out loud.
fn digest(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())[..8].to_string()
}

fn digest_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| digest(&bytes))
}

/// True when Shadow of the Erdtree is installed.
///
/// Worth knowing because the FAQ is blunt about the consequence: if the players
/// do not agree, the session hands out infinite loading screens and destroyed
/// characters.
pub fn has_dlc(game_dir: &Path) -> bool {
    game_dir.join("DLC.bdt").is_file() || game_dir.join("DLC.bhd").is_file()
}

/// Reads everything both players have to agree on.
///
/// `regulation` is the file that will actually be loaded: an edition ships its
/// own and it overrides the game's, so hashing the one in the game folder would
/// report a match between two people running different overhauls.
pub fn fingerprint(install: &Installation, regulation: Option<&Path>) -> Fingerprint {
    let mut traits = Vec::new();

    traits.push(Trait {
        key: "build".into(),
        label: "Game build".into(),
        value: install.version.clone().unwrap_or_else(|| "unknown".into()),
        matters: "Different game builds cannot see each other. Update the older copy.".into(),
    });

    // ersc.dll carries no version resource, so the declared version is usually
    // absent. Falling back to "unknown" would make two players running different
    // releases compare equal, which is the one answer this must never give — so
    // the DLL itself is hashed instead.
    let coop_value = install
        .seamless_coop_version
        .clone()
        .or_else(|| digest_file(&coop::dll_path(&install.game_dir)).map(|d| format!("#{d}")))
        .unwrap_or_else(|| "absent".into());

    traits.push(Trait {
        key: "coop".into(),
        label: "Seamless Co-op".into(),
        value: coop_value,
        matters: "Both players need the same Seamless Co-op release.".into(),
    });

    let regulation_path = regulation
        .map(Path::to_path_buf)
        .unwrap_or_else(|| install.game_dir.join("regulation.bin"));
    traits.push(Trait {
        key: "reg".into(),
        label: "regulation.bin".into(),
        value: digest_file(&regulation_path).unwrap_or_else(|| "missing".into()),
        matters: "You are running different mod data. Whoever is out of date has to reinstall the mod."
            .into(),
    });

    traits.push(Trait {
        key: "dlc".into(),
        label: "Shadow of the Erdtree".into(),
        value: if has_dlc(&install.game_dir) { "yes" } else { "no" }.into(),
        matters: "Playing DLC content when only one side owns it destroys characters.".into(),
    });

    // The password is hashed rather than printed. It has to match, but pasting a
    // co-op password into a public channel is how a session ends up with company.
    let password = coop::read(&install.game_dir)
        .ok()
        .and_then(|settings| {
            settings
                .values
                .iter()
                .find(|(key, _)| key.to_ascii_lowercase().contains("password"))
                .map(|(_, value)| value.clone())
        })
        .unwrap_or_default();
    traits.push(Trait {
        key: "pw".into(),
        label: "Password".into(),
        value: if password.trim().is_empty() {
            "not set".into()
        } else {
            digest(password.trim().as_bytes())[..4].to_string()
        },
        matters: "The passwords differ. They must be identical, and the game restarted after a change."
            .into(),
    });

    let block = render(&traits);
    Fingerprint { traits, block }
}

/// The pasteable form. Deliberately plain text: it has to survive Discord.
fn render(traits: &[Trait]) -> String {
    let mut out = String::from("ROUNDTABLE MATCH\n");
    for entry in traits {
        out.push_str(&format!("{:<6}{}\n", entry.key, entry.value));
    }
    out
}

/// Reads a block back, ignoring anything it does not recognise.
pub fn parse(text: &str) -> std::collections::BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.eq_ignore_ascii_case("ROUNDTABLE MATCH") {
                return None;
            }
            let mut parts = line.splitn(2, char::is_whitespace);
            let key = parts.next()?.trim().to_ascii_lowercase();
            let value = parts.next()?.trim().to_string();
            (!key.is_empty() && !value.is_empty()).then_some((key, value))
        })
        .collect()
}

/// Compares a friend's block against this machine.
pub fn compare(mine: &Fingerprint, theirs_text: &str) -> Comparison {
    let theirs = parse(theirs_text);

    // A block has to contain at least one line we know about. Any two words
    // parse into a key and a value, so merely getting something back is not
    // evidence of a block — and reporting "you match" because a paste of
    // nothing at all disagreed with nothing at all is the worst answer here.
    let recognised = mine
        .traits
        .iter()
        .filter(|t| theirs.contains_key(&t.key))
        .count();

    if recognised == 0 {
        return Comparison {
            verdict: Verdict::Unreadable,
            differences: Vec::new(),
            unknown: Vec::new(),
        };
    }

    let mut differences = Vec::new();
    let mut unknown = Vec::new();

    for entry in &mine.traits {
        match theirs.get(&entry.key) {
            Some(value) if value.eq_ignore_ascii_case(&entry.value) => {}
            Some(value) => differences.push(Difference {
                label: entry.label.clone(),
                mine: entry.value.clone(),
                theirs: value.clone(),
                matters: entry.matters.clone(),
            }),
            None => unknown.push(entry.label.clone()),
        }
    }

    Comparison {
        verdict: if differences.is_empty() {
            Verdict::Match
        } else {
            Verdict::Differs
        },
        differences,
        unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::InstallKind;
    use crate::games::Game;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-match-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn install(dir: &Path, dlc: bool) -> Installation {
        std::fs::write(dir.join("regulation.bin"), b"vanilla rules").unwrap();
        if dlc {
            std::fs::write(dir.join("DLC.bdt"), b"x").unwrap();
        }
        Installation {
            game: Game::EldenRing,
            root: dir.to_path_buf(),
            game_dir: dir.to_path_buf(),
            executable: dir.join("eldenring.exe"),
            kind: InstallKind::Standalone,
            version: Some("1.16.0.0".into()),
            has_eac: false,
            eac_bypassed: false,
            has_seamless_coop: true,
            seamless_coop_version: Some("1.8.2".into()),
            size_bytes: None,
            markers: Vec::new(),
        }
    }

    #[test]
    fn the_block_carries_every_line_that_has_to_agree() {
        let dir = temp("block");
        let print = fingerprint(&install(&dir, true), None);

        for key in ["build", "coop", "reg", "dlc", "pw"] {
            assert!(
                print.traits.iter().any(|t| t.key == key),
                "missing {key} in {:?}",
                print.traits
            );
        }
        assert!(print.block.starts_with("ROUNDTABLE MATCH"));
        assert!(print.block.contains("1.16.0.0"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_identical_setups_agree() {
        let dir = temp("same");
        let print = fingerprint(&install(&dir, true), None);
        let result = compare(&print, &print.block);
        assert_eq!(result.verdict, Verdict::Match);
        assert!(result.differences.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_different_regulation_is_named_and_explained() {
        let mine = temp("reg-mine");
        let theirs = temp("reg-theirs");

        let a = fingerprint(&install(&mine, true), None);
        // Written after the layout, which seeds a vanilla regulation.bin.
        let their_install = install(&theirs, true);
        std::fs::write(theirs.join("regulation.bin"), b"the convergence").unwrap();
        let b = fingerprint(&their_install, None);

        let result = compare(&a, &b.block);
        assert_eq!(result.verdict, Verdict::Differs);
        let diff = result
            .differences
            .iter()
            .find(|d| d.label == "regulation.bin")
            .expect("the mod data difference is reported");
        assert!(diff.matters.contains("reinstall"));

        std::fs::remove_dir_all(&mine).ok();
        std::fs::remove_dir_all(&theirs).ok();
    }

    #[test]
    fn a_dlc_mismatch_is_reported_because_it_destroys_characters() {
        let mine = temp("dlc-mine");
        let theirs = temp("dlc-theirs");
        let a = fingerprint(&install(&mine, true), None);
        let b = fingerprint(&install(&theirs, false), None);

        let result = compare(&a, &b.block);
        let diff = result
            .differences
            .iter()
            .find(|d| d.label.contains("Erdtree"))
            .expect("reported");
        assert_eq!(diff.mine, "yes");
        assert_eq!(diff.theirs, "no");
        assert!(diff.matters.contains("destroys"));

        std::fs::remove_dir_all(&mine).ok();
        std::fs::remove_dir_all(&theirs).ok();
    }

    #[test]
    fn an_edition_regulation_overrides_the_games_own() {
        let dir = temp("edition");
        let edition = dir.join("ConvergenceER");
        std::fs::create_dir_all(&edition).unwrap();
        std::fs::write(edition.join("regulation.bin"), b"the convergence").unwrap();

        let plain = fingerprint(&install(&dir, false), None);
        let modded = fingerprint(&install(&dir, false), Some(&edition.join("regulation.bin")));

        let reg = |f: &Fingerprint| f.traits.iter().find(|t| t.key == "reg").unwrap().value.clone();
        assert_ne!(
            reg(&plain),
            reg(&modded),
            "hashing the game's file would call two different overhauls a match"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_password_is_hashed_rather_than_printed() {
        let dir = temp("pw");
        std::fs::create_dir_all(dir.join(coop::COOP_DIR)).unwrap();
        std::fs::write(
            dir.join(coop::COOP_DIR).join(coop::SETTINGS_FILE),
            "[PASSWORD]\ncooppassword = hunter2\n",
        )
        .unwrap();

        let print = fingerprint(&install(&dir, false), None);
        assert!(
            !print.block.contains("hunter2"),
            "a co-op password pasted into a public channel invites company"
        );
        let pw = print.traits.iter().find(|t| t.key == "pw").unwrap();
        assert_ne!(pw.value, "not set");
        assert_eq!(pw.value.len(), 4);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_coop_builds_without_a_version_resource_still_differ() {
        let mine = temp("coop-mine");
        let theirs = temp("coop-theirs");

        for (dir, bytes) in [(&mine, &b"ersc 1.8.2"[..]), (&theirs, &b"ersc 1.7.9"[..])] {
            std::fs::create_dir_all(dir.join(coop::COOP_DIR)).unwrap();
            std::fs::write(dir.join(coop::COOP_DIR).join(coop::DLL_FILE), bytes).unwrap();
        }

        let strip = |dir: &Path| {
            let mut i = install(dir, false);
            i.seamless_coop_version = None; // what the real DLL reports
            i
        };

        let a = fingerprint(&strip(&mine), None);
        let b = fingerprint(&strip(&theirs), None);
        let result = compare(&a, &b.block);

        assert_eq!(
            result.verdict,
            Verdict::Differs,
            "two unknown versions must not compare equal"
        );
        assert!(result.differences.iter().any(|d| d.label == "Seamless Co-op"));

        std::fs::remove_dir_all(&mine).ok();
        std::fs::remove_dir_all(&theirs).ok();
    }

    #[test]
    fn nonsense_pasted_in_is_reported_as_unreadable() {
        let dir = temp("junk");
        let print = fingerprint(&install(&dir, false), None);
        assert_eq!(compare(&print, "hello there").verdict, Verdict::Unreadable);
        assert_eq!(compare(&print, "").verdict, Verdict::Unreadable);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_block_from_an_older_release_lists_what_it_did_not_say() {
        let dir = temp("partial");
        let print = fingerprint(&install(&dir, false), None);
        let result = compare(&print, "ROUNDTABLE MATCH\nbuild 1.16.0.0\n");
        assert!(result.unknown.contains(&"Seamless Co-op".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parsing_tolerates_the_mangling_chat_clients_apply() {
        let parsed = parse("  ROUNDTABLE MATCH  \n\nbuild   1.16.0.0\n  COOP  1.8.2  \n\n");
        assert_eq!(parsed.get("build").map(String::as_str), Some("1.16.0.0"));
        assert_eq!(parsed.get("coop").map(String::as_str), Some("1.8.2"));
        assert_eq!(parsed.len(), 2);
    }
}
