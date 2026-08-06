//! What is wrong, before it goes wrong.
//!
//! Seamless Co-op fails in a small number of ways, and its FAQ maps each error
//! message to a cause. That table is the community's entire troubleshooting
//! flow and it lives on a website, so people meet it only after the game has
//! already refused to start.
//!
//! Everything here is a check Roundtable can run against the machine in front
//! of it. A finding either says "this is broken and here is the fix", or it
//! stays quiet.

use std::path::Path;

use serde::Serialize;

use crate::coop;
use crate::game::{InstallKind, Installation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    /// Will stop the game or the session.
    Blocker,
    /// Works, but will surprise you.
    Warning,
    /// Worth knowing.
    Note,
    /// Checked and fine.
    Pass,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub level: Level,
    pub title: String,
    pub detail: String,
    /// The error text this would produce, so somebody who has already seen it
    /// can recognise their own problem.
    pub symptom: Option<String>,
    /// What to do, in one line.
    pub fix: Option<String>,
}

impl Finding {
    fn new(id: &str, level: Level, title: &str, detail: impl Into<String>) -> Finding {
        Finding {
            id: id.into(),
            level,
            title: title.into(),
            detail: detail.into(),
            symptom: None,
            fix: None,
        }
    }

    fn symptom(mut self, text: &str) -> Finding {
        self.symptom = Some(text.into());
        self
    }

    fn fix(mut self, text: &str) -> Finding {
        self.fix = Some(text.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub findings: Vec<Finding>,
    pub blockers: usize,
    pub warnings: usize,
}

/// True when this process is running with administrator rights.
///
/// It matters because of how launching works: a child process inherits the
/// parent's elevation, and Seamless Co-op refuses to start an elevated game
/// with "Failed to launch eldenring.exe (Error = 740)". A launcher started as
/// administrator produces that error in a program the user never elevated.
#[cfg(windows)]
pub fn running_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        // A single DWORD: non-zero when the token is elevated.
        let mut elevation: u32 = 0;
        let mut size: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
            &mut size,
        );
        CloseHandle(token);
        ok != 0 && elevation != 0
    }
}

#[cfg(not(windows))]
pub fn running_elevated() -> bool {
    false
}

/// Reads the co-op password out of the settings file, if one is set.
fn coop_password(game_dir: &Path) -> Option<String> {
    let settings = coop::read(game_dir).ok()?;
    settings
        .values
        .iter()
        .find(|(key, _)| key.to_ascii_lowercase().contains("password"))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The save extension Seamless Co-op is configured to use.
fn coop_extension(game_dir: &Path) -> Option<String> {
    let settings = coop::read(game_dir).ok()?;
    settings
        .values
        .iter()
        .find(|(key, _)| key.to_ascii_lowercase().contains("extension"))
        .map(|(_, value)| value.trim().trim_matches('.').to_ascii_lowercase())
}

/// Runs every check against an installation.
pub fn run(install: &Installation, edition_regulation: Option<&Path>) -> Report {
    let mut findings = Vec::new();
    let dir = &install.game_dir;

    // ── Elevation ────────────────────────────────────────────────────
    if running_elevated() {
        findings.push(
            Finding::new(
                "elevated",
                Level::Blocker,
                "Roundtable is running as administrator",
                "The game inherits that, and Seamless Co-op refuses to start an elevated copy.",
            )
            .symptom("Failed to launch eldenring.exe (Error = 740)")
            .fix("Close Roundtable and start it normally, without \"Run as administrator\"."),
        );
    }

    // ── Seamless Co-op ───────────────────────────────────────────────
    let dll = coop::dll_path(dir);
    if !dll.is_file() {
        findings.push(
            Finding::new(
                "no-ersc",
                Level::Warning,
                "Seamless Co-op is not installed",
                format!("{} is missing.", dll.display()),
            )
            .symptom("Failed to find SeamlessCoop//ersc.dll")
            .fix("Put the mod's SeamlessCoop folder inside the game directory."),
        );
    } else {
        match coop_extension(dir).as_deref() {
            // The one setting that makes co-op overwrite a solo character.
            Some("sl2") => findings.push(
                Finding::new(
                    "coop-sl2",
                    Level::Blocker,
                    "Co-op is set to write over your solo save",
                    "The save extension is sl2, which is the vanilla game's own file.",
                )
                .fix("Set the extension to co2 on the Co-op tab."),
            ),
            Some(other) => findings.push(Finding::new(
                "coop-ext",
                Level::Pass,
                "Co-op saves are separate",
                format!("Characters go to .{other}, away from the solo save."),
            )),
            None => {}
        }

        match coop_password(dir) {
            Some(_) => findings.push(Finding::new(
                "coop-pw",
                Level::Pass,
                "A co-op password is set",
                "It has to match your friend's exactly, and the game restarted after a change.",
            )),
            None => findings.push(
                Finding::new(
                    "coop-pw-missing",
                    Level::Warning,
                    "No co-op password",
                    "Without one you can be matched with strangers running the same setup.",
                )
                .fix("Generate one on the Co-op tab and send it to whoever you are playing with."),
            ),
        }
    }

    // ── regulation.bin ───────────────────────────────────────────────
    let regulation = edition_regulation
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join("regulation.bin"));
    match std::fs::metadata(&regulation) {
        Ok(meta) if meta.len() > 1024 => {}
        Ok(meta) => findings.push(
            Finding::new(
                "regulation-small",
                Level::Blocker,
                "regulation.bin looks corrupted",
                format!("It is only {} bytes; a real one is around two megabytes.", meta.len()),
            )
            .symptom("YKRegulationManager errors on startup")
            .fix("Delete it and verify the game files."),
        ),
        Err(_) => findings.push(
            Finding::new(
                "regulation-missing",
                Level::Blocker,
                "regulation.bin is missing",
                format!("{} does not exist.", regulation.display()),
            )
            .symptom("YKRegulationManager errors on startup")
            .fix("Verify the game files."),
        ),
    }

    // ── Anti-cheat ───────────────────────────────────────────────────
    if install.has_eac && !install.eac_bypassed {
        findings.push(Finding::new(
            "eac-on",
            Level::Note,
            "Easy Anti-Cheat is active",
            "Mod loaders start the game directly and skip it, so this does not block anything here.",
        ));
    }

    // ── DLC ──────────────────────────────────────────────────────────
    if crate::matchup::has_dlc(dir) {
        findings.push(Finding::new(
            "dlc",
            Level::Pass,
            "Shadow of the Erdtree is installed",
            "Everyone in a session has to own it, or characters can be destroyed in DLC areas.",
        ));
    } else {
        findings.push(
            Finding::new(
                "no-dlc",
                Level::Warning,
                "No Shadow of the Erdtree",
                "Joining someone who has it and entering DLC content destroys characters and hangs on loading.",
            )
            .fix("Stay out of the DLC areas in co-op, or install it."),
        );
    }

    // ── Where the saves went ─────────────────────────────────────────
    // The configured extension is passed in so a co-op file the user renamed
    // still counts as a save rather than reading as "none found".
    let folders = crate::saves::discover(install.game, coop_extension(dir).as_deref());
    if folders.is_empty() {
        findings.push(Finding::new(
            "no-saves",
            Level::Note,
            "No saves found yet",
            "Start the game once so it creates a character.",
        ));
    } else {
        // Empty containers are their own state. A repack that writes somewhere
        // unexpected produces files with nothing in them, and "no characters"
        // is a much more useful thing to say than listing five empty files.
        let characters: usize = folders
            .iter()
            .flat_map(|folder| &folder.entries)
            .filter(|entry| entry.flavour != crate::saves::SaveFlavour::GameBackup)
            .filter_map(|entry| entry.summary.as_ref())
            .map(|summary| summary.slots.iter().filter(|slot| slot.active).count())
            .sum();

        if characters == 0 {
            findings.push(Finding::new(
                "empty-saves",
                Level::Note,
                "The save files are empty",
                "They exist but hold no characters. Create one in game and it will appear here.",
            ));
        }
    }

    // ── Steam, or not ────────────────────────────────────────────────
    if install.kind == InstallKind::Standalone {
        findings.push(Finding::new(
            "standalone",
            Level::Note,
            "This copy does not come from Steam",
            "Roundtable passes the executable to the loader directly and skips Steam init. Seamless Co-op's own matchmaking still needs a Steam connection to find players.",
        ));
    }

    let blockers = findings.iter().filter(|f| f.level == Level::Blocker).count();
    let warnings = findings.iter().filter(|f| f.level == Level::Warning).count();

    Report {
        findings,
        blockers,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::Game;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-diag-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn install(dir: &Path) -> Installation {
        std::fs::write(dir.join("regulation.bin"), vec![0u8; 2_000_000]).unwrap();
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
            seamless_coop_version: None,
            size_bytes: None,
            markers: Vec::new(),
        }
    }

    fn with_coop(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join(coop::COOP_DIR)).unwrap();
        std::fs::write(dir.join(coop::COOP_DIR).join(coop::DLL_FILE), b"dll").unwrap();
        std::fs::write(dir.join(coop::COOP_DIR).join(coop::SETTINGS_FILE), body).unwrap();
    }

    fn find<'a>(report: &'a Report, id: &str) -> Option<&'a Finding> {
        report.findings.iter().find(|f| f.id == id)
    }

    #[test]
    fn a_coop_save_extension_of_sl2_is_a_blocker() {
        let dir = temp("sl2");
        with_coop(
            &dir,
            "[SAVE]\nsave_file_extension = sl2\n[PASSWORD]\ncooppassword = x\n",
        );
        let report = run(&install(&dir), None);

        let hit = find(&report, "coop-sl2").expect("reported");
        assert_eq!(hit.level, Level::Blocker);
        assert!(hit.fix.as_ref().unwrap().contains("co2"));
        assert!(report.blockers >= 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn co2_is_reported_as_fine_rather_than_silently() {
        let dir = temp("co2");
        with_coop(
            &dir,
            "[SAVE]\nsave_file_extension = co2\n[PASSWORD]\ncooppassword = x\n",
        );
        let report = run(&install(&dir), None);
        assert_eq!(find(&report, "coop-ext").unwrap().level, Level::Pass);
        assert!(find(&report, "coop-sl2").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_password_is_flagged() {
        let dir = temp("nopw");
        with_coop(&dir, "[SAVE]\nsave_file_extension = co2\n[PASSWORD]\ncooppassword = \n");
        let report = run(&install(&dir), None);
        assert_eq!(find(&report, "coop-pw-missing").unwrap().level, Level::Warning);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_dll_carries_the_error_people_actually_see() {
        let dir = temp("nodll");
        let report = run(&install(&dir), None);
        let hit = find(&report, "no-ersc").expect("reported");
        assert!(hit.symptom.as_ref().unwrap().contains("ersc.dll"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_regulation_is_caught_before_the_game_complains() {
        let dir = temp("badreg");
        let mut i = install(&dir);
        std::fs::write(dir.join("regulation.bin"), b"nope").unwrap();
        i.has_seamless_coop = false;

        let report = run(&i, None);
        let hit = find(&report, "regulation-small").expect("reported");
        assert_eq!(hit.level, Level::Blocker);
        assert!(hit.symptom.as_ref().unwrap().contains("YKRegulationManager"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_editions_regulation_is_checked_instead_of_the_games() {
        let dir = temp("editionreg");
        let edition = dir.join("ConvergenceER");
        std::fs::create_dir_all(&edition).unwrap();
        // The game's own file is fine; the edition's is not, and the edition's
        // is the one that will load.
        std::fs::write(edition.join("regulation.bin"), b"truncated").unwrap();

        let report = run(&install(&dir), Some(&edition.join("regulation.bin")));
        assert!(find(&report, "regulation-small").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_dlc_warns_about_destroyed_characters() {
        let dir = temp("nodlc");
        let report = run(&install(&dir), None);
        let hit = find(&report, "no-dlc").expect("reported");
        assert!(hit.detail.contains("destroys"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_dlc_being_present_is_stated_too() {
        let dir = temp("dlc");
        std::fs::write(dir.join("DLC.bdt"), b"x").unwrap();
        let report = run(&install(&dir), None);
        assert_eq!(find(&report, "dlc").unwrap().level, Level::Pass);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_healthy_install_reports_no_blockers() {
        let dir = temp("clean");
        std::fs::write(dir.join("DLC.bdt"), b"x").unwrap();
        with_coop(
            &dir,
            "[SAVE]\nsave_file_extension = co2\n[PASSWORD]\ncooppassword = secret\n",
        );
        let report = run(&install(&dir), None);
        assert_eq!(report.blockers, 0, "got {:?}", report.findings);
        std::fs::remove_dir_all(&dir).ok();
    }
}
