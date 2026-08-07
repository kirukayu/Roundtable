//! The frame rate, and the two things that ruin it.
//!
//! ELDEN RING caps at 60. When somebody reports 30 the cap is not what they hit
//! — exclusive fullscreen forces the display to 60 Hz with vsync that cannot be
//! turned off, and a frame that misses the deadline halves the rate to exactly
//! 30. Borderless avoids the forced mode entirely.
//!
//! The second is the settings themselves. `AutoDetectBestRenderingSettings` off
//! with everything on MAX is what the menu leaves behind after one click, and at
//! 1440p that is more than most cards hold.
//!
//! `GraphicsConfig.xml` is UTF-16 with a BOM. Writing it as UTF-8 makes the game
//! discard it and start over, so the encoding is preserved exactly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::games::Game;

/// Settings worth showing, in the order the game's own menu lists them.
const SETTINGS: &[&str] = &[
    "ScreenMode",
    "QualitySetting",
    "TextureQuality",
    "Antialiasing",
    "SSAO",
    "DepthOfField",
    "MotionBlur",
    "ShadowQuality",
    "LightingQuality",
    "EffectsQuality",
    "ReflectionQuality",
    "WaterSurfaceQuality",
    "ShadeQuality",
    "VolumetricEffectQuality",
    "RaytracingQuality",
    "GIDataQuality",
    "GrassQuality",
];

/// What a preset changes, and why each one is in the list.
#[derive(Clone, Copy)]
struct Tweak {
    key: &'static str,
    value: &'static str,
    reason: &'static str,
}

/// What the machine can actually push, worked out rather than assumed.
///
/// The same settings are wrong in both directions: MAX everything on a 1650 is
/// a slideshow, and dropping a 4080 to HIGH throws away picture for frames it
/// already had. So the preset is built per machine from the card, the panel and
/// the pixel count, and each tier is the one the hardware holds a steady frame
/// rate at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    /// Integrated or old, and the target is playability.
    Weak,
    /// A card that runs the game well at 1080p.
    Modest,
    /// Comfortable at 1440p.
    Strong,
    /// Everything on, and frames to spare.
    Ample,
}

/// A rough score for a card, read off its name.
///
/// Reading real capability needs a device and a benchmark. The generation and
/// the tier digits are enough to place a card in one of four bands, and being
/// one band out costs a setting rather than a broken game.
///
/// The scale is NVIDIA's: `generation × 4 + tier`, so a 4080 is 240 and a 1060
/// is 100. Generation is worth about four tier steps, which is why a 4060 lands
/// beside a 3070 rather than beside a 3060. AMD is mapped onto the same scale by
/// the generation it competes with.
fn score_gpu(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();

    // Integrated parts say so in their name and never reach a band.
    if lower.contains("uhd graphics")
        || lower.contains("hd graphics")
        || lower.contains("iris")
        || lower.contains("radeon graphics")
    {
        return Some(0);
    }

    // The model number: the longest run of digits, e.g. 4070 or 7900.
    let model: u32 = lower
        .split(|c: char| !c.is_ascii_digit())
        .max_by_key(|part| part.len())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);

    if lower.contains("geforce") || lower.contains("nvidia") || lower.contains("rtx") || lower.contains("gtx") {
        if model == 0 {
            return None;
        }
        // 4080 → 40 and 80. 960 → 9 and 60. The same split works for both.
        return Some((model / 100) * 4 + (model % 100));
    }

    if lower.contains("radeon") || lower.contains("rx") {
        if model >= 1000 {
            // RX 7900 → the 7000 series, tier 900. Each AMD series lines up with
            // an NVIDIA generation, so it is scored as that one.
            let series = model / 1000;
            let generation = match series {
                5 => 20,
                6 => 30,
                7 => 40,
                8 | 9 => 50,
                _ => series * 5,
            };
            return Some(generation * 4 + (model % 1000) / 10);
        }
        // RX 580 and older, all well below the bands.
        return Some(model / 10);
    }

    if lower.contains("arc") {
        // A340 through B580: the letter is the generation and the rest the tier.
        return Some(if lower.contains('b') { 180 } else { 140 });
    }

    None
}

/// Where this machine lands, from the card, its memory and the panel.
///
/// The bands are calibrated at 1440p, which is what most people who ask about
/// this are running. 4K costs a band and 1080p gains one.
pub fn tier_for(gpu: Option<&str>, vram_mb: u64, pixels: u64) -> Tier {
    let score = gpu.and_then(score_gpu);

    let mut tier = match score {
        Some(s) if s >= 230 => Tier::Ample,
        Some(s) if s >= 165 => Tier::Strong,
        Some(s) if s >= 130 => Tier::Modest,
        Some(_) => Tier::Weak,
        // An unknown card is judged on its memory alone, which is the one
        // number that is always available and never a guess.
        None => match vram_mb {
            0..=3_500 => Tier::Weak,
            3_501..=7_000 => Tier::Modest,
            7_001..=11_000 => Tier::Strong,
            _ => Tier::Ample,
        },
    };

    // Pixels are the other half of the load. 4K is 2.25× of 1440p; 1080p is a
    // little over half.
    let up = pixels > 0 && pixels <= 2_400_000;
    let down = pixels > 5_000_000;
    if up {
        tier = match tier {
            Tier::Weak => Tier::Modest,
            Tier::Modest => Tier::Strong,
            _ => Tier::Ample,
        };
    }
    if down {
        tier = match tier {
            Tier::Ample => Tier::Strong,
            Tier::Strong => Tier::Modest,
            _ => Tier::Weak,
        };
    }

    // Memory is a hard floor and has the last word: no amount of core holds MAX
    // textures on 4 GB, and running out of it stutters far worse than a lower
    // setting ever looks.
    if vram_mb > 0 && vram_mb <= 4_500 && tier > Tier::Modest {
        tier = Tier::Modest;
    }
    if vram_mb > 0 && vram_mb <= 3_000 {
        tier = Tier::Weak;
    }

    tier
}

/// Frames this machine can hold without the rate moving about.
///
/// A cap is only worth setting at a number the card reaches every single frame.
/// Half the frames at 180 and half at 90 looks far worse than all of them at 90,
/// which is what "it tears and the mouse judders" actually is.
fn holdable(tier: Tier) -> u32 {
    match tier {
        Tier::Weak | Tier::Modest => 60,
        Tier::Strong => 90,
        Tier::Ample => 120,
    }
}

/// The cap to offer: the highest clean division of the panel this machine holds.
///
/// The division has to be clean. 100 frames on a 180 Hz screen means most frames
/// are shown for one refresh and some for two, and that uneven cadence is the
/// tearing and the jerky camera. 90 into 180 is two refreshes every frame,
/// exactly, and it is smooth.
pub fn suggested_cap(refresh_hz: u32, tier: Tier) -> u32 {
    cap_under(refresh_hz, holdable(tier))
}

/// The same, for a rate a frame generator is producing.
///
/// A generator roughly doubles the finished rate, and a limit set on finished
/// frames therefore allows twice what the card renders. 180 Hz on a card that
/// holds 90 is 90 rendered and 180 shown, every frame landing on its refresh —
/// which is the smoothest this panel goes.
pub fn suggested_cap_generated(refresh_hz: u32, tier: Tier) -> u32 {
    cap_under(refresh_hz, holdable(tier) * 2)
}

fn cap_under(refresh_hz: u32, ceiling: u32) -> u32 {
    if refresh_hz <= 60 {
        return 60;
    }
    (1..=6)
        .map(|divisor| refresh_hz / divisor)
        .filter(|fps| refresh_hz % fps == 0 && *fps >= 60 && *fps <= ceiling)
        .max()
        .unwrap_or(60)
}

/// The two settings nobody should be running, whatever the machine.
///
/// Borderless leads because it is the one that turns 30 back into 60: exclusive
/// fullscreen makes the game ask Windows for a 60 Hz mode and then hold vsync to
/// it, so one late frame halves the rate. Auto-detect is pinned off or the game
/// overwrites everything below on the next start.
const ALWAYS: &[Tweak] = &[
    Tweak {
        key: "ScreenMode",
        value: "BORDERLESS",
        reason: "Exclusive fullscreen forces 60 Hz with vsync that halves to 30 on a missed frame",
    },
    Tweak {
        key: "MotionBlur",
        value: "OFF",
        reason: "Free, and it hides the frame rate rather than fixing it",
    },
    Tweak {
        key: "AutoDetectBestRenderingSettings",
        value: "OFF",
        reason: "Otherwise the game overwrites these on the next start",
    },
];

/// One row of the table: what each tier sets this key to.
struct Scaled {
    key: &'static str,
    weak: &'static str,
    modest: &'static str,
    strong: &'static str,
    ample: &'static str,
    reason: &'static str,
}

/// The settings worth scaling, cheapest first in what they cost to give up.
///
/// Ray tracing is off at every tier on purpose. In this game it is a reflection
/// pass that costs a third of the frame rate and is invisible in motion, and no
/// card makes that trade worthwhile.
const SCALED: &[Scaled] = &[
    Scaled {
        key: "RaytracingQuality",
        weak: "DISABLE", modest: "DISABLE", strong: "DISABLE", ample: "DISABLE",
        reason: "Costs a third of the frame rate for reflections nobody sees in motion",
    },
    Scaled {
        key: "ShadowQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "The largest ordinary cost in the frame",
    },
    Scaled {
        key: "GrassQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Hurts most in the open world, which is most of the game",
    },
    Scaled {
        key: "EffectsQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Drops frames in exactly the fights where it matters",
    },
    Scaled {
        key: "VolumetricEffectQuality",
        weak: "LOW", modest: "LOW", strong: "MEDIUM", ample: "HIGH",
        reason: "Fog and god rays, expensive and subtle",
    },
    Scaled {
        key: "ReflectionQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Screen-space reflections scale badly with resolution",
    },
    Scaled {
        key: "LightingQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Barely visible above HIGH",
    },
    Scaled {
        key: "DepthOfField",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "A full-screen blur pass every frame",
    },
    Scaled {
        key: "SSAO",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Contact shadows, cheap at MEDIUM and expensive at MAX",
    },
    Scaled {
        key: "TextureQuality",
        weak: "MEDIUM", modest: "HIGH", strong: "MAX", ample: "MAX",
        reason: "Free until the card runs out of memory, then catastrophic",
    },
    Scaled {
        key: "Antialiasing",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Cheap, and the picture falls apart without it",
    },
    Scaled {
        key: "WaterSurfaceQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Only costs anything near water",
    },
    Scaled {
        key: "ShadeQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Shader detail, steady cost across the frame",
    },
    Scaled {
        key: "GIDataQuality",
        weak: "LOW", modest: "MEDIUM", strong: "HIGH", ample: "MAX",
        reason: "Bounced light, loaded rather than computed",
    },
];

impl Scaled {
    fn at(&self, tier: Tier) -> &'static str {
        match tier {
            Tier::Weak => self.weak,
            Tier::Modest => self.modest,
            Tier::Strong => self.strong,
            Tier::Ample => self.ample,
        }
    }
}

/// What the tier is wrong about once frames are being generated.
///
/// Both of these scale up with the card, and both of them are worth giving up
/// the moment an interpolator is in the pipeline — the first because it is the
/// mod's one known artefact and the author's own workaround is to turn it down,
/// the second because a full-screen blur that tracks the camera rather than the
/// geometry is precisely what interpolation gets wrong. Without this the
/// optimiser and the frame-generation pane would undo one another, which is
/// worse than either being wrong on its own.
const GENERATED_FRAMES: &[Tweak] = &[
    Tweak {
        key: "GIDataQuality",
        value: "LOW",
        reason: "Frame generation flickers the light in shaded rooms, and turning global \
                 illumination down is the mod author's own workaround",
    },
    Tweak {
        key: "DepthOfField",
        value: "LOW",
        reason: "A blur that moves with the camera and not with the geometry, which is \
                 what an interpolator guesses wrong",
    },
];

/// The preset for one machine.
///
/// `framegen` is whether a frame-generation mod is installed, which changes two
/// of the answers regardless of how strong the card is.
fn preset(tier: Tier, framegen: bool) -> Vec<Tweak> {
    let mut out: Vec<Tweak> = ALWAYS.to_vec();
    out.extend(SCALED.iter().map(|row| Tweak {
        key: row.key,
        value: row.at(tier),
        reason: row.reason,
    }));

    if framegen {
        for override_ in GENERATED_FRAMES {
            match out.iter_mut().find(|tweak| tweak.key == override_.key) {
                Some(existing) => *existing = *override_,
                None => out.push(*override_),
            }
        }
    }
    out
}

/// What the preset would set one key to, with frame generation in the picture.
///
/// Exposed so the frame-generation pane can check the two agree rather than
/// hope: they write the same file, and a disagreement means whichever button was
/// pressed last wins.
pub fn preset_value(tier: Tier, key: &str) -> Option<&'static str> {
    preset(tier, true)
        .into_iter()
        .find(|tweak| tweak.key == key)
        .map(|tweak| tweak.value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: String,
    /// What this would become, when it is costing frames.
    pub suggested: Option<String>,
    pub reason: Option<String>,
}

/// What the preset was decided from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub gpu: Option<String>,
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub cores: usize,
    /// Desktop resolution, which is what the game will render at in borderless.
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub tier: Tier,
    /// The frame cap worth setting: the highest clean division of the panel this
    /// machine holds every frame.
    pub suggested_cap: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfStatus {
    pub path: Option<PathBuf>,
    pub settings: Vec<Setting>,
    /// The display's resolution and refresh, for comparison.
    pub display: Option<String>,
    /// True while the game is set to exclusive fullscreen, which caps it at 60
    /// and halves to 30 on any dip.
    pub exclusive_fullscreen: bool,
    /// How many settings the preset would change.
    pub improvable: usize,
    /// An FPS unlocker DLL sitting in the game or edition folder, from before
    /// Roundtable could do it itself.
    pub unlocker: Option<PathBuf>,
    /// What the preset was worked out from.
    pub machine: Machine,
    /// True while the game is running and can have its cap rewritten.
    pub game_running: bool,
}

/// The card, its memory, and the panel it is driving.
#[cfg(windows)]
fn machine() -> Machine {
    let (width, height, refresh_hz) = display_geometry().unwrap_or((0, 0, 0));

    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let ram_mb = system.total_memory() / (1024 * 1024);
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);

    let (gpu, vram_mb) = primary_adapter().unwrap_or((None, 0));
    let pixels = u64::from(width) * u64::from(height);
    let tier = tier_for(gpu.as_deref(), vram_mb, pixels);

    Machine {
        suggested_cap: suggested_cap(refresh_hz, tier),
        tier,
        gpu,
        vram_mb,
        ram_mb,
        cores,
        width,
        height,
        refresh_hz,
    }
}

#[cfg(not(windows))]
fn machine() -> Machine {
    Machine {
        gpu: None,
        vram_mb: 0,
        ram_mb: 0,
        cores: 0,
        width: 0,
        height: 0,
        refresh_hz: 0,
        tier: Tier::Modest,
        suggested_cap: 60,
    }
}

/// The display adapter's name and memory, out of the driver's own registry key.
///
/// `HardwareInformation.qwMemorySize` is what the driver reports to Windows, so
/// it is the same number the control panel shows.
#[cfg(windows)]
pub(crate) fn primary_adapter() -> Option<(Option<String>, u64)> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let class = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}")
        .ok()?;

    let mut best: Option<(Option<String>, u64)> = None;
    for index in 0..8u32 {
        let Ok(key) = class.open_subkey(format!("{index:04}")) else {
            continue;
        };
        let name: Option<String> = key.get_value("DriverDesc").ok();
        // The 64-bit value is the reliable one; the 32-bit one wraps above 4 GB.
        let bytes: u64 = key
            .get_value::<u64, _>("HardwareInformation.qwMemorySize")
            .or_else(|_| key.get_value::<u32, _>("HardwareInformation.MemorySize").map(u64::from))
            .unwrap_or(0);
        let mb = bytes / (1024 * 1024);

        // Several adapters are listed on a laptop. The one with the most memory
        // is the one the game will run on.
        if best.as_ref().is_none_or(|(_, seen)| mb > *seen) {
            best = Some((name, mb));
        }
    }
    best
}

#[cfg(not(windows))]
pub(crate) fn primary_adapter() -> Option<(Option<String>, u64)> {
    None
}

pub fn config_path(game: Game) -> Option<PathBuf> {
    let path = dirs::data_dir()?.join(game.appdata_folder()).join("GraphicsConfig.xml");
    path.is_file().then_some(path)
}

/// How the file was encoded, so it can be written back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8,
    /// UTF-16 LE with a byte-order mark.
    Utf16Bom,
    /// UTF-16 LE with no mark, which is what the game actually writes.
    Utf16Bare,
}

/// True when the bytes look like UTF-16 LE without a mark.
///
/// The game writes the declaration `encoding="UTF-16"` but no BOM, so there is
/// nothing to key off but the shape of the data: ASCII text encoded as UTF-16 LE
/// puts a zero in every second byte. Checking a sample is enough, and it cannot
/// misfire on real UTF-8 because that never contains a zero byte at all.
fn looks_like_utf16(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }
    let sample = bytes.len().min(512);
    let odd_zeroes = bytes[..sample]
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    odd_zeroes * 4 > sample
}

fn read(path: &Path) -> Result<(String, Encoding)> {
    let bytes = std::fs::read(path).at(path)?;

    let (body, encoding) = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        (&bytes[2..], Encoding::Utf16Bom)
    } else if looks_like_utf16(&bytes) {
        (&bytes[..], Encoding::Utf16Bare)
    } else {
        return Ok((String::from_utf8_lossy(&bytes).to_string(), Encoding::Utf8));
    };

    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Ok((String::from_utf16_lossy(&units), encoding))
}

fn write(path: &Path, text: &str, encoding: Encoding) -> Result<()> {
    let bytes = match encoding {
        Encoding::Utf8 => return std::fs::write(path, text).at(path),
        Encoding::Utf16Bom => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf16Bare => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    };
    std::fs::write(path, bytes).at(path)
}

/// Pulls `<Key>value</Key>` out of the document.
fn tag_value(text: &str, key: &str) -> Option<String> {
    let open = format!("<{key}>");
    let close = format!("</{key}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_string())
}

/// Replaces a tag's value, leaving the rest of the document alone.
fn set_tag(text: &str, key: &str, value: &str) -> Option<String> {
    let open = format!("<{key}>");
    let close = format!("</{key}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(format!("{}{}{}", &text[..start], value, &text[end..]))
}

/// An FPS unlocker DLL, if one has been dropped in.
fn find_unlocker(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.ends_with(".dll") && (name.contains("unlockthefps") || name.contains("fpsunlock"))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

pub fn status(game: Game, roots: &[PathBuf], framegen: bool) -> PerfStatus {
    let path = config_path(game);
    let machine = machine();
    let wanted = preset(machine.tier, framegen);
    let mut settings = Vec::new();
    let mut exclusive = false;

    if let Some(path) = &path {
        if let Ok((text, _)) = read(path) {
            exclusive = tag_value(&text, "ScreenMode")
                .is_some_and(|mode| mode.eq_ignore_ascii_case("FULLSCREEN"));

            for key in SETTINGS {
                let Some(value) = tag_value(&text, key) else {
                    continue;
                };
                let tweak = wanted.iter().find(|t| t.key == *key);
                let suggested = tweak
                    .filter(|t| !t.value.eq_ignore_ascii_case(&value))
                    .map(|t| t.value.to_string());
                settings.push(Setting {
                    key: (*key).to_string(),
                    reason: suggested.as_ref().and(tweak.map(|t| t.reason.to_string())),
                    suggested,
                    value,
                });
            }
        }
    }

    let improvable = settings.iter().filter(|s| s.suggested.is_some()).count();

    PerfStatus {
        display: display_mode(),
        settings,
        exclusive_fullscreen: exclusive,
        improvable,
        unlocker: find_unlocker(roots),
        game_running: crate::unlock::running_pid(game.executable()).is_some(),
        machine,
        path,
    }
}

/// Applies the preset this machine warrants, and reports what it changed.
pub fn smooth(game: Game, framegen: bool) -> Result<Vec<String>> {
    let path = config_path(game).ok_or_else(|| {
        Error::msg("No GraphicsConfig.xml yet. Start the game once so it writes one.")
    })?;

    let (mut text, encoding) = read(&path)?;
    let mut changed = Vec::new();

    for tweak in preset(machine().tier, framegen) {
        let Some(current) = tag_value(&text, tweak.key) else {
            continue;
        };
        if current.eq_ignore_ascii_case(tweak.value) {
            continue;
        }
        if let Some(updated) = set_tag(&text, tweak.key, tweak.value) {
            text = updated;
            changed.push(format!("{}: {current} to {}", tweak.key, tweak.value));
        }
    }

    // Borderless renders at the desktop resolution, so the file's borderless
    // size has to be the desktop's or the game letterboxes itself.
    if let Some((width, height, _)) = display_geometry() {
        for (key, value) in [
            ("Resolution-BorderlessScreenWidth", width),
            ("Resolution-BorderlessScreenHeight", height),
        ] {
            let Some(current) = tag_value(&text, key) else {
                continue;
            };
            if current == value.to_string() {
                continue;
            }
            if let Some(updated) = set_tag(&text, key, &value.to_string()) {
                text = updated;
                changed.push(format!("{key}: {current} to {value}"));
            }
        }
    }

    if !changed.is_empty() {
        let backup = path.with_extension("xml.roundtable-bak");
        if !backup.exists() {
            let _ = std::fs::copy(&path, &backup);
        }
        write(&path, &text, encoding)?;
    }

    Ok(changed)
}

/// Sets one value by hand.
pub fn set(game: Game, key: &str, value: &str) -> Result<()> {
    if !SETTINGS.contains(&key) && key != "AutoDetectBestRenderingSettings" {
        return Err(Error::msg(format!("{key} is not a graphics setting")));
    }
    let path = config_path(game).ok_or_else(|| Error::msg("No GraphicsConfig.xml yet"))?;
    let (text, encoding) = read(&path)?;
    let updated = set_tag(&text, key, value)
        .ok_or_else(|| Error::msg(format!("{key} is not in the file")))?;
    write(&path, &updated, encoding)
}

/// The desktop's current mode: width, height, refresh.
#[cfg(windows)]
pub(crate) fn display_geometry() -> Option<(u32, u32, u32)> {
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    };

    unsafe {
        let mut mode: DEVMODEW = std::mem::zeroed();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
            return None;
        }
        Some((mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency))
    }
}

#[cfg(not(windows))]
pub(crate) fn display_geometry() -> Option<(u32, u32, u32)> {
    None
}

/// What the game itself is set to render at, for the screen mode it is in.
///
/// The file keeps a separate size per mode, so reading the wrong pair reports a
/// mismatch that is not there.
pub fn game_resolution(game: Game, screen_mode: Option<&str>) -> Option<(u32, u32)> {
    let (text, _) = read(&config_path(game)?).ok()?;
    let prefix = match screen_mode.map(str::to_ascii_uppercase).as_deref() {
        Some("FULLSCREEN") => "FullScreen",
        Some("WINDOW") => "WindowScreen",
        _ => "BorderlessScreen",
    };
    let width = tag_value(&text, &format!("Resolution-{prefix}Width"))?.parse().ok()?;
    let height = tag_value(&text, &format!("Resolution-{prefix}Height"))?.parse().ok()?;
    Some((width, height))
}

/// The same thing, written the way it reads in the interface.
fn display_mode() -> Option<String> {
    let (width, height, hz) = display_geometry()?;
    Some(format!("{width}x{height} at {hz} Hz"))
}

/// Re-applies the display mode, which is what unsticks a juddering pointer.
///
/// Windows sometimes leaves the desktop nominally at its full refresh while the
/// cursor is still being drawn at sixty — usually after a game exits, after
/// sleep, or after a mode change went badly. The pointer then moves in visible
/// steps and nothing in the interface looks wrong, because nothing is: the
/// number in Settings is correct.
///
/// The cure people find by hand is to pick another refresh rate and pick the
/// original back, which tears the mode down and rebuilds it. That is exactly
/// what this does, so it takes a button instead of four menus.
#[cfg(windows)]
pub fn bounce_refresh() -> Result<String> {
    use windows_sys::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplaySettingsW, CDS_TYPE, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
        DM_DISPLAYFREQUENCY, ENUM_CURRENT_SETTINGS,
    };

    unsafe {
        let mut current: DEVMODEW = std::mem::zeroed();
        current.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut current) == 0 {
            return Err(Error::msg("could not read the display mode".to_string()));
        }
        let wanted = current.dmDisplayFrequency;

        // Another refresh rate at exactly this resolution and depth. Changing
        // the size as well would rearrange every window on the desktop.
        let mut other = None;
        let mut index = 0u32;
        loop {
            let mut mode: DEVMODEW = std::mem::zeroed();
            mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
            if EnumDisplaySettingsW(std::ptr::null(), index, &mut mode) == 0 {
                break;
            }
            index += 1;
            if mode.dmPelsWidth == current.dmPelsWidth
                && mode.dmPelsHeight == current.dmPelsHeight
                && mode.dmBitsPerPel == current.dmBitsPerPel
                && mode.dmDisplayFrequency != wanted
                && mode.dmDisplayFrequency >= 24
            {
                // The nearest one, so the flicker is as brief as possible.
                let distance = mode.dmDisplayFrequency.abs_diff(wanted);
                match other {
                    Some((_, best)) if best <= distance => {}
                    _ => other = Some((mode.dmDisplayFrequency, distance)),
                }
            }
        }

        let Some((step, _)) = other else {
            return Err(Error::msg(
                "this display only offers one refresh rate, so there is nothing to bounce"
                    .to_string(),
            ));
        };

        let mut apply = |hz: u32| -> i32 {
            let mut mode = current;
            mode.dmDisplayFrequency = hz;
            mode.dmFields = DM_DISPLAYFREQUENCY;
            ChangeDisplaySettingsExW(
                std::ptr::null(),
                &mut mode,
                std::ptr::null_mut(),
                0 as CDS_TYPE,
                std::ptr::null(),
            )
        };

        if apply(step) != DISP_CHANGE_SUCCESSFUL {
            return Err(Error::msg("Windows refused the mode change".to_string()));
        }
        // Long enough for the mode to actually take before it is undone.
        std::thread::sleep(std::time::Duration::from_millis(900));

        if apply(wanted) != DISP_CHANGE_SUCCESSFUL {
            return Err(Error::msg(format!(
                "the display is stuck at {step} Hz — set it back to {wanted} in Windows"
            )));
        }

        Ok(format!("Display bounced through {step} Hz and back to {wanted}"))
    }
}

#[cfg(not(windows))]
pub fn bounce_refresh() -> Result<String> {
    Err(Error::msg("only on Windows".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<config>",
        "<ScreenMode>FULLSCREEN</ScreenMode>",
        "<AutoDetectBestRenderingSettings>OFF</AutoDetectBestRenderingSettings>",
        "<QualitySetting>MAX</QualitySetting>",
        "<ShadowQuality>MAX</ShadowQuality>",
        "<MotionBlur>HIGH</MotionBlur>",
        "<RaytracingQuality>DISABLE</RaytracingQuality>",
        "</config>"
    );

    #[test]
    fn a_value_is_read_out_of_the_document() {
        assert_eq!(tag_value(CONFIG, "ScreenMode").as_deref(), Some("FULLSCREEN"));
        assert_eq!(tag_value(CONFIG, "ShadowQuality").as_deref(), Some("MAX"));
        assert_eq!(tag_value(CONFIG, "Nonsense"), None);
    }

    #[test]
    fn setting_a_value_leaves_the_rest_of_the_file_alone() {
        let out = set_tag(CONFIG, "ScreenMode", "BORDERLESS").unwrap();
        assert!(out.contains("<ScreenMode>BORDERLESS</ScreenMode>"));
        assert!(out.contains("<ShadowQuality>MAX</ShadowQuality>"));
        assert!(out.contains("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
        assert_eq!(out.matches("<ScreenMode>").count(), 1);
    }

    #[test]
    fn utf16_survives_a_round_trip() {
        // The game writes UTF-16 with a BOM and discards the file if it comes
        // back as anything else.
        let dir = std::env::temp_dir().join("roundtable-perf-utf16");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("GraphicsConfig.xml");

        write(&path, CONFIG, Encoding::Utf16Bom).unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..2], &[0xFF, 0xFE], "the BOM must be there");

        let (text, encoding) = read(&path).unwrap();
        assert_eq!(encoding, Encoding::Utf16Bom);
        assert_eq!(text, CONFIG);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn utf16_without_a_byte_order_mark_is_still_utf16() {
        // What the game actually writes: `encoding="UTF-16"` in the declaration
        // and no BOM. Reading it as UTF-8 gives a string with a null between
        // every character, and then no tag is ever found.
        let dir = std::env::temp_dir().join("roundtable-perf-nobom");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("GraphicsConfig.xml");

        let bare: Vec<u8> = CONFIG.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(&bare[..2], &[0x3C, 0x00], "'<' as UTF-16 LE, no mark");
        std::fs::write(&path, &bare).unwrap();

        let (text, encoding) = read(&path).unwrap();
        assert_eq!(encoding, Encoding::Utf16Bare);
        assert_eq!(tag_value(&text, "ScreenMode").as_deref(), Some("FULLSCREEN"));

        // And it goes back the same way, or the game discards it.
        write(&path, &text, encoding).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bare);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_encoding_guess_does_not_misfire_on_utf8() {
        assert!(!looks_like_utf16(CONFIG.as_bytes()));
        assert!(!looks_like_utf16(b"<config><ScreenMode>X</ScreenMode></config>"));
        assert!(!looks_like_utf16(b""));
        assert!(!looks_like_utf16(b"abc"));
    }

    #[test]
    fn utf8_files_are_not_forced_into_utf16() {
        let dir = std::env::temp_dir().join("roundtable-perf-utf8");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("GraphicsConfig.xml");

        std::fs::write(&path, CONFIG).unwrap();
        let (text, encoding) = read(&path).unwrap();
        assert_eq!(encoding, Encoding::Utf8);
        assert_eq!(text, CONFIG);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_preset_leads_with_the_setting_that_causes_the_halving() {
        for tier in [Tier::Weak, Tier::Modest, Tier::Strong, Tier::Ample] {
            let built = preset(tier, false);
            assert_eq!(built[0].key, "ScreenMode");
            assert_eq!(built[0].value, "BORDERLESS");
            assert!(built[0].reason.contains("30"));
        }
    }

    #[test]
    fn a_setting_already_at_the_target_is_not_suggested() {
        // Ray tracing is off at every tier, and this config already has it off.
        let built = preset(Tier::Ample, false);
        let tweak = built.iter().find(|t| t.key == "RaytracingQuality").unwrap();
        let current = tag_value(CONFIG, "RaytracingQuality").unwrap();
        assert!(current.eq_ignore_ascii_case(tweak.value));
    }

    #[test]
    fn the_preset_pins_auto_detect_off() {
        // Otherwise the game rewrites everything the next time it starts.
        assert!(preset(Tier::Strong, false)
            .iter()
            .any(|t| t.key == "AutoDetectBestRenderingSettings" && t.value == "OFF"));
    }

    #[test]
    fn a_strong_card_is_not_dragged_down_to_the_same_preset_as_a_weak_one() {
        // The old fixed preset dropped a 4080 to HIGH for no reason.
        let weak = preset(Tier::Weak, false);
        let ample = preset(Tier::Ample, false);
        let shadow = |list: &[Tweak]| {
            list.iter().find(|t| t.key == "ShadowQuality").unwrap().value
        };
        assert_eq!(shadow(&weak), "LOW");
        assert_eq!(shadow(&ample), "MAX");
    }

    #[test]
    fn cards_land_in_the_band_they_belong_to() {
        // 1440p, so a little over 3.6 megapixels.
        let at_1440 = 2560 * 1440;
        assert_eq!(tier_for(Some("NVIDIA GeForce RTX 4080"), 16_384, at_1440), Tier::Ample);
        assert_eq!(tier_for(Some("NVIDIA GeForce RTX 3070"), 8_192, at_1440), Tier::Strong);
        assert_eq!(tier_for(Some("NVIDIA GeForce GTX 1060"), 6_144, at_1440), Tier::Weak);
        assert_eq!(tier_for(Some("Intel(R) UHD Graphics 630"), 0, at_1440), Tier::Weak);
    }

    #[test]
    fn four_k_costs_a_band() {
        // The same card has to give up a tier when it is pushing 2.25 times the
        // pixels.
        let card = Some("NVIDIA GeForce RTX 4080");
        let at_1440 = tier_for(card, 16_384, 2560 * 1440);
        let at_4k = tier_for(card, 16_384, 3840 * 2160);
        assert!(at_4k < at_1440, "{at_4k:?} should be below {at_1440:?}");
    }

    #[test]
    fn a_card_short_of_memory_is_held_back_whatever_its_name_says() {
        // A 3050 with 4 GB cannot hold MAX textures however new it is.
        let tier = tier_for(Some("NVIDIA GeForce RTX 3050"), 4_096, 1920 * 1080);
        assert!(tier <= Tier::Modest, "got {tier:?}");
        let textures = preset(tier, false)
            .iter()
            .find(|t| t.key == "TextureQuality")
            .unwrap()
            .value;
        assert_ne!(textures, "MAX");
    }

    #[test]
    fn the_cap_divides_evenly_into_the_panel() {
        // An uneven cadence is the tearing and the jerky camera: at 100 on a 180
        // Hz screen most frames are shown for one refresh and some for two.
        // An odd panel like 75 Hz has no clean division a card can also hold, and
        // there 60 is still the right answer: the rate the game was built for.
        for hz in [60u32, 75, 120, 144, 165, 180, 240] {
            for tier in [Tier::Weak, Tier::Modest, Tier::Strong, Tier::Ample] {
                let cap = suggested_cap(hz, tier);
                assert!(cap >= 60, "{hz} {tier:?} gave {cap}");
                assert!(
                    hz % cap == 0 || cap == 60,
                    "{hz} Hz {tier:?}: {cap} neither divides nor falls back to 60"
                );
            }
        }
    }

    #[test]
    fn the_cap_never_asks_for_more_than_the_machine_holds() {
        // A 4060 at 1440p on a 180 Hz panel gets 90, not 180: half the frames at
        // 180 and half at 90 looks worse than all of them at 90.
        assert_eq!(suggested_cap(180, Tier::Strong), 90);
        assert_eq!(suggested_cap(180, Tier::Weak), 60);
        assert_eq!(suggested_cap(240, Tier::Ample), 120);
        assert_eq!(suggested_cap(144, Tier::Strong), 72);
    }

    #[test]
    fn a_sixty_hertz_panel_is_left_at_sixty() {
        for tier in [Tier::Weak, Tier::Ample] {
            assert_eq!(suggested_cap(60, tier), 60);
            assert_eq!(suggested_cap(0, tier), 60);
        }
    }

    #[test]
    fn amd_is_scored_against_the_generation_it_competes_with() {
        let at_1440 = 2560 * 1440;
        // A 7900 XTX trades blows with a 4080, and a 6700 XT with a 3070.
        assert_eq!(tier_for(Some("AMD Radeon RX 7900 XTX"), 24_576, at_1440), Tier::Ample);
        assert_eq!(tier_for(Some("AMD Radeon RX 6700 XT"), 12_288, at_1440), Tier::Strong);
        // And an RX 580 is not a 58-series anything.
        assert_eq!(tier_for(Some("AMD Radeon RX 580"), 8_192, at_1440), Tier::Weak);
    }

    #[test]
    fn a_three_digit_geforce_is_not_read_as_a_new_one() {
        // 960 must not parse as generation 960.
        assert!(score_gpu("NVIDIA GeForce GTX 960").unwrap() < 130);
    }

    #[test]
    fn an_unknown_card_is_judged_on_its_memory() {
        assert_eq!(tier_for(Some("Some Unreleased Thing"), 24_576, 1920 * 1080), Tier::Ample);
        assert_eq!(tier_for(None, 2_048, 1920 * 1080), Tier::Weak);
    }

    #[test]
    fn every_scaled_row_goes_up_and_never_down() {
        // A higher tier must never be given a lower setting than a lower one, or
        // a better machine ends up looking worse.
        let order = ["OFF", "DISABLE", "LOW", "MEDIUM", "HIGH", "MAX"];
        let rank = |value: &str| order.iter().position(|v| *v == value).unwrap_or(99);
        for row in SCALED {
            let steps = [row.weak, row.modest, row.strong, row.ample];
            for pair in steps.windows(2) {
                assert!(
                    rank(pair[0]) <= rank(pair[1]),
                    "{}: {} then {}",
                    row.key,
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn every_scaled_key_is_one_the_game_has() {
        // A key the file does not contain is a setting that silently does nothing.
        for row in SCALED {
            assert!(SETTINGS.contains(&row.key), "{} is not in the file", row.key);
        }
    }

    #[test]
    fn an_unlocker_dll_is_recognised_by_name() {
        let dir = std::env::temp_dir().join("roundtable-perf-dll");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        assert!(find_unlocker(&[dir.clone()]).is_none());
        std::fs::write(dir.join("UnlockTheFps.dll"), b"x").unwrap();
        assert!(find_unlocker(&[dir.clone()]).is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unrelated_dll_is_not_mistaken_for_one() {
        let dir = std::env::temp_dir().join("roundtable-perf-other");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("unlock_wwise_states_er.dll"), b"x").unwrap();
        assert!(
            find_unlocker(&[dir.clone()]).is_none(),
            "the Convergence ships an audio DLL with unlock in the name"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
