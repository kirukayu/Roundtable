//! The levers outside the game.
//!
//! Graphics settings and the frame cap are only half of how smooth a game feels.
//! The other half is Windows, and every one of these is a switch buried three
//! menus deep or in no menu at all:
//!
//! * **Flip model.** A borderless window is composited by the desktop by
//!   default, which costs a frame of latency and blocks variable refresh. Windows
//!   can upgrade the window to the same presentation path exclusive fullscreen
//!   uses. This is the single biggest thing for a borderless game, and it is the
//!   reason Roundtable can recommend borderless without giving anything up.
//! * **Which card.** A laptop with two GPUs will happily run the game on the one
//!   built into the processor unless it is told otherwise.
//! * **Background recording.** Game DVR keeps the last thirty seconds of every
//!   game encoded, whether or not anybody ever saves a clip.
//! * **GPU scheduling.** Hands the queue to the card instead of the driver.
//! * **Pointer acceleration.** Windows accelerating the mouse turns a fast flick
//!   into a jump, which reads as the camera teleporting.
//!
//! Everything here is read before it is written, reported as a before and after,
//! and kept so it can be put back.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result};

/// Where the graphics preferences live, per user.
const GPU_PREFS: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";
/// The switch for background recording.
const GAME_STORE: &str = r"System\GameConfigStore";
const MOUSE: &str = r"Control Panel\Mouse";
/// Hardware-accelerated GPU scheduling, which is machine-wide.
const GRAPHICS_DRIVERS: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lever {
    pub id: String,
    pub title: String,
    /// Why it is worth changing, in one sentence.
    pub detail: String,
    pub current: String,
    pub wanted: String,
    /// True when it is already where it should be.
    pub done: bool,
    /// True when the change only takes effect after a restart of the machine.
    pub needs_reboot: bool,
    /// True when it cannot be written without administrator rights.
    pub needs_admin: bool,
    /// Where to click, for the ones no program can set.
    ///
    /// Enabling G-Sync for a monitor the driver has not certified is a checkbox
    /// in NVIDIA's own panel and there is no supported way in. Saying so beats
    /// pretending the button did something.
    pub by_hand: Option<String>,
}

/// Everything Roundtable will have changed, so it can be put back.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Kept {
    /// Registry values as they were, keyed by `hive\path\value`.
    pub before: std::collections::BTreeMap<String, Option<String>>,
}

fn kept_path(app_data: &Path) -> PathBuf {
    app_data.join("tuning-before.json")
}

fn load_kept(app_data: &Path) -> Kept {
    std::fs::read(kept_path(app_data))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_kept(app_data: &Path, kept: &Kept) -> Result<()> {
    let path = kept_path(app_data);
    std::fs::create_dir_all(app_data).at(app_data)?;
    std::fs::write(&path, serde_json::to_vec_pretty(kept)?).at(&path)?;
    Ok(())
}

/// Reads a string value, whatever numeric shape it is stored in.
#[cfg(windows)]
fn read_value(root: winreg::HKEY, path: &str, name: &str) -> Option<String> {
    use winreg::RegKey;
    let key = RegKey::predef(root).open_subkey(path).ok()?;
    key.get_value::<String, _>(name)
        .ok()
        .or_else(|| key.get_value::<u32, _>(name).ok().map(|n| n.to_string()))
}

/// The per-app preferences string, e.g. `GpuPreference=2;SwapEffectUpgradeEnable=1;`.
///
/// It is a semicolon list, and other tools write into the same value, so a key
/// is replaced in place rather than the whole string being overwritten.
fn merge_pairs(existing: &str, pairs: &[(&str, &str)]) -> String {
    let mut out: Vec<(String, String)> = existing
        .split(';')
        .filter(|part| !part.trim().is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect();

    for (key, value) in pairs {
        match out.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(key)) {
            Some((_, slot)) => *slot = (*value).to_string(),
            None => out.push(((*key).to_string(), (*value).to_string())),
        }
    }

    out.iter()
        .map(|(key, value)| format!("{key}={value};"))
        .collect()
}

/// True when the list already contains this pair.
fn has_pair(existing: &str, key: &str, value: &str) -> bool {
    existing.split(';').any(|part| {
        part.split_once('=').is_some_and(|(name, got)| {
            name.trim().eq_ignore_ascii_case(key) && got.trim() == value
        })
    })
}

/// What each lever is, what it is set to, and what it should be.
#[cfg(windows)]
pub fn survey(executables: &[PathBuf]) -> Vec<Lever> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut levers = Vec::new();

    // The per-app entry, which is where flip model and the card are chosen.
    for exe in executables {
        let name = exe.to_string_lossy().to_string();
        let current = read_value(HKEY_CURRENT_USER, GPU_PREFS, &name).unwrap_or_default();
        let done = has_pair(&current, "SwapEffectUpgradeEnable", "1")
            && has_pair(&current, "GpuPreference", "2");
        let wanted = merge_pairs(
            &current,
            &[("GpuPreference", "2"), ("SwapEffectUpgradeEnable", "1")],
        );
        levers.push(Lever {
            id: format!("gpu-prefs:{name}"),
            title: format!(
                "Flip model and the fast card for {}",
                exe.file_name().map_or_else(|| name.clone(), |n| n.to_string_lossy().to_string())
            ),
            detail: "A borderless window goes through the desktop compositor, which \
                     costs a frame and blocks variable refresh. This puts it on the same \
                     path exclusive fullscreen uses."
                .to_string(),
            current: if current.is_empty() { "not set".into() } else { current },
            wanted,
            done,
            needs_reboot: false,
            needs_admin: false,
            by_hand: None,
        });
    }

    // The same thing as a default, for anything Roundtable did not launch.
    let global =
        read_value(HKEY_CURRENT_USER, GPU_PREFS, "DirectXUserGlobalSettings").unwrap_or_default();
    levers.push(Lever {
        id: "windowed-optimisations".into(),
        title: "Optimizations for windowed games".into(),
        detail: "The same flip-model upgrade applied to every windowed game.".into(),
        current: if global.is_empty() { "not set".into() } else { global.clone() },
        // Flip model only.
        //
        // `VRROptimizeEnable` used to be set here too and it has been taken out.
        // Variable refresh is a real win on hardware that implements it well and
        // a real problem on hardware that does not: on the monitor this was
        // developed against it made the pointer stutter under Windows and made
        // the screen flicker under Linux. Turning it on for everybody, sight
        // unseen, is not a trade a launcher gets to make — it belongs in the
        // driver panel, where it can be seen and undone.
        wanted: merge_pairs(&global, &[("SwapEffectUpgradeEnable", "1")]),
        done: has_pair(&global, "SwapEffectUpgradeEnable", "1"),
        needs_reboot: false,
        needs_admin: false,
        by_hand: None,
    });

    // Background recording, which encodes the last half minute of every game.
    let dvr = read_value(HKEY_CURRENT_USER, GAME_STORE, "GameDVR_Enabled").unwrap_or_default();
    levers.push(Lever {
        id: "game-dvr".into(),
        title: "Background recording".into(),
        detail: "Game DVR keeps encoding the last thirty seconds whether or not a clip \
                 is ever saved."
            .into(),
        current: if dvr == "0" { "off".into() } else { "on".into() },
        wanted: "off".into(),
        done: dvr == "0",
        needs_reboot: false,
        needs_admin: false,
        by_hand: None,
    });

    // Pointer acceleration, which is what makes a fast flick overshoot.
    let accel = read_value(HKEY_CURRENT_USER, MOUSE, "MouseSpeed").unwrap_or_default();
    levers.push(Lever {
        id: "mouse-acceleration".into(),
        title: "Pointer acceleration".into(),
        detail: "Windows scaling the mouse by how fast it moves turns a flick into a \
                 jump, which reads as the camera teleporting."
            .into(),
        current: if accel == "0" { "off".into() } else { "on".into() },
        wanted: "off".into(),
        done: accel == "0",
        needs_reboot: false,
        needs_admin: false,
        by_hand: None,
    });

    // Hardware scheduling, which is machine-wide and needs a restart.
    let sched =
        read_value(HKEY_LOCAL_MACHINE, GRAPHICS_DRIVERS, "HwSchMode").unwrap_or_default();
    levers.push(Lever {
        id: "gpu-scheduling".into(),
        title: "Hardware-accelerated GPU scheduling".into(),
        detail: "Hands the render queue to the card instead of the driver, which \
                 steadies frame times."
            .into(),
        current: match sched.as_str() {
            "2" => "on".into(),
            "1" => "off".into(),
            _ => "default".into(),
        },
        wanted: "on".into(),
        done: sched == "2",
        needs_reboot: true,
        needs_admin: true,
        by_hand: None,
    });

    // Variable refresh, which is the one thing that beats every setting here and
    // the one thing no program can turn on.
    //
    // A fixed 180 Hz panel shows every frame for one refresh or two, whichever
    // the frame took — and that uneven cadence is what tearing and a juddering
    // camera actually are. Variable refresh makes the panel wait for the frame
    // instead, and then a rate that moves about stops mattering at all. NVIDIA
    // only enables it by default for monitors it has certified; for the rest it
    // is a checkbox, and there is no supported way to tick it from outside.
    if let Some((_, _, hz)) = crate::perf::display_geometry() {
        if hz > 60 {
            levers.push(Lever {
                id: "variable-refresh".into(),
                title: "Variable refresh (G-Sync / FreeSync)".into(),
                detail: format!(
                    "Your screen runs at {hz} Hz on a fixed cadence, so a frame that \
                     takes a moment too long is shown twice. Variable refresh removes \
                     that entirely and matters more than every other setting here."
                ),
                current: "cannot be read from here".into(),
                wanted: "on".into(),
                done: false,
                needs_reboot: false,
                needs_admin: false,
                by_hand: Some(
                    "NVIDIA Control Panel → Display → Set up G-SYNC → tick Enable for \
                     windowed and full screen mode, pick this monitor, then Enable \
                     settings for the selected display model. AMD: Radeon Software → \
                     Display → FreeSync."
                        .into(),
                ),
            });
        }
    }

    // Frame generation, which ELDEN RING has no support for and does not need to.
    //
    // The game has no upscaler to hook, so DLSS cannot be added to it: the
    // wrappers that swap one upscaler for another need one to be there already,
    // and nothing outside the engine can produce the motion vectors DLSS wants.
    // Frame generation is a different matter, because the driver can do it from
    // the finished frames without the game knowing. On a 40 or 50 series card
    // that is a switch in NVIDIA's own app, and it doubles the frame rate of a
    // game that has never heard of it.
    //
    // It is worth saying what it costs: a generated frame is guessed from the two
    // around it, so the base rate has to be high enough that a guess is close. At
    // 60 real frames it is convincing. At 30 it is not, and the extra frames make
    // the latency worse rather than better.
    if let Some((Some(card), _)) = crate::perf::primary_adapter() {
        let lower = card.to_ascii_lowercase();
        let modern = lower.contains("rtx")
            && ["40", "50"].iter().any(|series| {
                lower
                    .split_whitespace()
                    .any(|word| word.len() == 4 && word.starts_with(series))
            });
        if modern {
            levers.push(Lever {
                id: "frame-generation".into(),
                title: "Frame generation (NVIDIA Smooth Motion)".into(),
                detail: format!(
                    "{card} can generate a frame between every two the game draws, in \
                     the driver, for games with no support of their own — and this game \
                     has none. It roughly doubles the frame rate."
                ),
                current: "cannot be read from here".into(),
                wanted: "on".into(),
                done: false,
                needs_reboot: false,
                needs_admin: false,
                by_hand: Some(
                    "NVIDIA App → Graphics → ELDEN RING → Smooth Motion → On. Cap the \
                     game at 60 first: generated frames are guessed from real ones, so \
                     a steady 60 doubled to 120 looks right, and an unsteady 40 does not."
                        .into(),
                ),
            });
        }
    }

    levers
}

#[cfg(not(windows))]
pub fn survey(_executables: &[PathBuf]) -> Vec<Lever> {
    Vec::new()
}

/// Applies every lever that is not already where it should be.
///
/// Reports one line per change. A lever that cannot be written — the machine-wide
/// one without administrator rights — is skipped and said so rather than failing
/// the whole run.
#[cfg(windows)]
pub fn apply(app_data: &Path, executables: &[PathBuf]) -> Result<Vec<String>> {
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WRITE,
    };
    use winreg::RegKey;

    let mut kept = load_kept(app_data);
    let mut changed = Vec::new();

    // Nothing here can tick the variable-refresh box, and saying it applied
    // "everything" while quietly skipping the largest one would be a lie.
    if let Some((_, _, hz)) = crate::perf::display_geometry() {
        if hz > 60 {
            changed.push(format!(
                "Variable refresh is still off or unknown — worth more than all of this on a {hz} Hz screen"
            ));
        }
    }

    let mut remember = |slot: String, before: Option<String>| {
        kept.before.entry(slot).or_insert(before);
    };

    // Per-app preferences.
    {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(GPU_PREFS)
            .map(|(key, _)| key);
        if let Ok(key) = key {
            for exe in executables {
                let name = exe.to_string_lossy().to_string();
                let current = key.get_value::<String, _>(&name).unwrap_or_default();
                let wanted = merge_pairs(
                    &current,
                    &[("GpuPreference", "2"), ("SwapEffectUpgradeEnable", "1")],
                );
                if wanted == current {
                    continue;
                }
                remember(
                    format!("HKCU\\{GPU_PREFS}\\{name}"),
                    (!current.is_empty()).then_some(current),
                );
                if key.set_value(&name, &wanted).is_ok() {
                    changed.push(format!(
                        "Flip model and the fast card for {}",
                        exe.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string())
                    ));
                }
            }

            let global = key
                .get_value::<String, _>("DirectXUserGlobalSettings")
                .unwrap_or_default();
            // Flip model only — see the note beside the lever above for why
            // variable refresh is no longer forced on.
            let wanted = merge_pairs(&global, &[("SwapEffectUpgradeEnable", "1")]);
            if wanted != global {
                remember(
                    format!("HKCU\\{GPU_PREFS}\\DirectXUserGlobalSettings"),
                    (!global.is_empty()).then_some(global),
                );
                if key.set_value("DirectXUserGlobalSettings", &wanted).is_ok() {
                    changed.push("Optimizations for windowed games turned on".into());
                }
            }
        }
    }

    // Background recording.
    if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(GAME_STORE) {
        let before = key.get_value::<u32, _>("GameDVR_Enabled").ok();
        if before != Some(0) {
            remember(
                format!("HKCU\\{GAME_STORE}\\GameDVR_Enabled"),
                before.map(|n| n.to_string()),
            );
            if key.set_value("GameDVR_Enabled", &0u32).is_ok() {
                changed.push("Background recording turned off".into());
            }
        }
    }

    // Pointer acceleration. All three go together: the thresholds mean nothing
    // on their own, and leaving them behind is how it half-works.
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(MOUSE, KEY_SET_VALUE | KEY_WRITE) {
        let before = read_value(HKEY_CURRENT_USER, MOUSE, "MouseSpeed");
        if before.as_deref() != Some("0") {
            remember(format!("HKCU\\{MOUSE}\\MouseSpeed"), before);
            let ok = key.set_value("MouseSpeed", &"0".to_string()).is_ok()
                && key.set_value("MouseThreshold1", &"0".to_string()).is_ok()
                && key.set_value("MouseThreshold2", &"0".to_string()).is_ok();
            if ok {
                changed.push("Pointer acceleration turned off".into());
            }
        }
    }

    // Hardware scheduling, which is machine-wide and needs a restart. Roundtable
    // is not elevated, so if the plain write is refused it asks Windows for the
    // rights rather than printing an instruction and leaving it to the user.
    {
        let before = read_value(HKEY_LOCAL_MACHINE, GRAPHICS_DRIVERS, "HwSchMode");
        if before.as_deref() != Some("2") {
            let direct = RegKey::predef(HKEY_LOCAL_MACHINE)
                .open_subkey_with_flags(GRAPHICS_DRIVERS, KEY_SET_VALUE)
                .is_ok_and(|key| key.set_value("HwSchMode", &2u32).is_ok());

            let done = direct || elevate_gpu_scheduling().unwrap_or(false);
            if done {
                remember(format!("HKLM\\{GRAPHICS_DRIVERS}\\HwSchMode"), before);
                changed.push(
                    "GPU scheduling turned on — restart Windows for it, and DLSS frame \
                     generation needs it"
                        .into(),
                );
            } else {
                changed.push("GPU scheduling was refused at the administrator prompt".into());
            }
        }
    }

    save_kept(app_data, &kept)?;
    Ok(changed)
}

#[cfg(not(windows))]
pub fn apply(_app_data: &Path, _executables: &[PathBuf]) -> Result<Vec<String>> {
    Ok(Vec::new())
}

/// Asks Windows for administrator and writes the machine-wide switch.
///
/// Roundtable itself stays unelevated — a launcher that demands administrator to
/// start is a launcher people stop using. Instead this runs `reg.exe` through the
/// elevation prompt for the one value that needs it, waits for it, and then reads
/// the value back. A prompt that was dismissed and a prompt that was accepted
/// look identical from here otherwise.
#[cfg(windows)]
pub fn elevate_gpu_scheduling() -> Result<bool> {
    use std::os::windows::process::CommandExt;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    if read_value(HKEY_LOCAL_MACHINE, GRAPHICS_DRIVERS, "HwSchMode").as_deref() == Some("2") {
        return Ok(true);
    }

    // `RunAs` is what raises the prompt; `-Wait` is what makes the read below
    // mean anything. No console window: the user asked for a button, not a
    // flash of black.
    const NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "Start-Process reg.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList \
         'add',\"HKLM\\{GRAPHICS_DRIVERS}\",'/v','HwSchMode','/t','REG_DWORD','/d','2','/f'"
    );

    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(NO_WINDOW)
        .status();

    // Read it back rather than trusting that the prompt was accepted: a refused
    // prompt and an accepted one are the same exit code from out here.
    Ok(read_value(HKEY_LOCAL_MACHINE, GRAPHICS_DRIVERS, "HwSchMode").as_deref() == Some("2"))
}

#[cfg(not(windows))]
pub fn elevate_gpu_scheduling() -> Result<bool> {
    Ok(false)
}

/// True when hardware GPU scheduling is on, which DLSS frame generation needs.
#[cfg(windows)]
pub fn gpu_scheduling_on() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    read_value(HKEY_LOCAL_MACHINE, GRAPHICS_DRIVERS, "HwSchMode").as_deref() == Some("2")
}

#[cfg(not(windows))]
pub fn gpu_scheduling_on() -> bool {
    false
}

/// Puts every value back the way it was found.
#[cfg(windows)]
pub fn revert(app_data: &Path) -> Result<Vec<String>> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE};
    use winreg::RegKey;

    let kept = load_kept(app_data);
    let mut restored = Vec::new();

    for (slot, before) in &kept.before {
        let Some((hive, rest)) = slot.split_once('\\') else {
            continue;
        };
        let Some((path, name)) = rest.rsplit_once('\\') else {
            continue;
        };
        let root = match hive {
            "HKLM" => HKEY_LOCAL_MACHINE,
            _ => HKEY_CURRENT_USER,
        };
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(path, KEY_SET_VALUE) else {
            continue;
        };

        let ok = match before {
            // A value that did not exist is deleted rather than blanked.
            None => key.delete_value(name).is_ok(),
            Some(value) => match value.parse::<u32>() {
                Ok(number) if !value.contains('=') => key.set_value(name, &number).is_ok(),
                _ => key.set_value(name, value).is_ok(),
            },
        };
        if ok {
            restored.push(slot.clone());
        }
    }

    if !restored.is_empty() {
        let _ = std::fs::remove_file(kept_path(app_data));
    }
    Ok(restored)
}

#[cfg(not(windows))]
pub fn revert(_app_data: &Path) -> Result<Vec<String>> {
    Ok(Vec::new())
}

/// Anything holding the graphics card while the game runs.
///
/// Named rather than closed: a screen share is usually deliberate, and the point
/// is that the cost is visible, not that Roundtable decides.
pub fn competitors() -> Vec<String> {
    const HUNGRY: &[(&str, &str)] = &[
        ("discord", "Discord — a screen share encodes every frame on the same card"),
        ("obs64", "OBS is capturing"),
        ("obs32", "OBS is capturing"),
        ("streamlabs", "Streamlabs is capturing"),
        ("chrome", "Chrome with hardware acceleration on"),
        ("msedge", "Edge with hardware acceleration on"),
        ("firefox", "Firefox with hardware acceleration on"),
        ("zen", "Zen with hardware acceleration on"),
        ("brave", "Brave with hardware acceleration on"),
    ];

    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut seen: Vec<String> = Vec::new();
    for process in system.processes().values() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let stem = name.trim_end_matches(".exe");
        for (needle, note) in HUNGRY {
            if stem == *needle && !seen.iter().any(|line| line == note) {
                seen.push((*note).to_string());
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lever_nothing_can_set_says_where_to_click() {
        // Reporting it as done, or leaving it out, are both worse than saying so.
        let levers = survey(&[]);
        let Some(vrr) = levers.iter().find(|lever| lever.id == "variable-refresh") else {
            // Only offered above 60 Hz, and the machine running the tests may not be.
            return;
        };
        assert!(!vrr.done, "it cannot be known to be done");
        let by_hand = vrr.by_hand.as_ref().expect("it has to say where");
        assert!(by_hand.contains("G-SYNC"), "got {by_hand}");
        assert!(by_hand.contains("FreeSync"), "and the other card: {by_hand}");
    }

    #[test]
    fn a_pair_is_replaced_rather_than_appended() {
        // Other tools write into the same value, and two GpuPreference entries
        // is undefined behaviour rather than a preference.
        let out = merge_pairs("AppStatus=0;GpuPreference=1;", &[("GpuPreference", "2")]);
        assert_eq!(out.matches("GpuPreference").count(), 1, "got {out}");
        assert!(out.contains("GpuPreference=2;"));
        assert!(out.contains("AppStatus=0;"), "the other keys survive: {out}");
    }

    #[test]
    fn a_missing_pair_is_added_without_disturbing_the_rest() {
        // This is the exact value the game folder already has.
        let out = merge_pairs(
            "AppStatus=0;",
            &[("GpuPreference", "2"), ("SwapEffectUpgradeEnable", "1")],
        );
        assert_eq!(out, "AppStatus=0;GpuPreference=2;SwapEffectUpgradeEnable=1;");
    }

    #[test]
    fn an_empty_value_becomes_just_the_pairs() {
        assert_eq!(merge_pairs("", &[("GpuPreference", "2")]), "GpuPreference=2;");
    }

    #[test]
    fn applying_the_same_pairs_twice_changes_nothing() {
        // Otherwise every run reports changes it did not make.
        let once = merge_pairs("AppStatus=0;", &[("GpuPreference", "2")]);
        let twice = merge_pairs(&once, &[("GpuPreference", "2")]);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_pair_is_recognised_only_when_it_matches_exactly() {
        let value = "GpuPreference=2;SwapEffectUpgradeEnable=1;";
        assert!(has_pair(value, "GpuPreference", "2"));
        assert!(has_pair(value, "swapeffectupgradeenable", "1"));
        assert!(!has_pair(value, "GpuPreference", "1"));
        assert!(!has_pair(value, "VRROptimizeEnable", "1"));
        assert!(!has_pair("", "GpuPreference", "2"));
    }

    #[test]
    fn what_was_there_before_is_written_down_once() {
        // A second run must not overwrite the original with the tuned value, or
        // reverting puts the tuning back.
        let dir = std::env::temp_dir().join("roundtable-tune-kept");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let mut kept = Kept::default();
        kept.before.entry("a".into()).or_insert(Some("original".into()));
        kept.before.entry("a".into()).or_insert(Some("tuned".into()));
        save_kept(&dir, &kept).unwrap();

        let read = load_kept(&dir);
        assert_eq!(read.before.get("a"), Some(&Some("original".to_string())));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_value_that_did_not_exist_is_remembered_as_absent() {
        // So reverting deletes it rather than writing an empty string.
        let dir = std::env::temp_dir().join("roundtable-tune-absent");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let mut kept = Kept::default();
        kept.before.insert("HKCU\\Some\\Path\\Value".into(), None);
        save_kept(&dir, &kept).unwrap();
        assert_eq!(load_kept(&dir).before.get("HKCU\\Some\\Path\\Value"), Some(&None));

        std::fs::remove_dir_all(&dir).ok();
    }
}
