//! The 60 frame cap, removed in the running game.
//!
//! Two separate things hold ELDEN RING at 60, and only one of them is the frame
//! limiter.
//!
//! The limiter is a single float compiled into the code — `mov [rbx+1C],
//! 3C888889`, which is 1/60 of a second per frame. Writing a different delta
//! there is the whole of the frame unlock.
//!
//! The second is worse and is what turns 60 into 30. Every time the game changes
//! display mode it asks Windows for 60 Hz, hardcoded, ignoring what the monitor
//! is actually set to. On a 144 or 180 Hz screen the game therefore runs against
//! a 60 Hz mode with vsync it will not let go of, and one late frame halves it to
//! exactly 30. Zeroing the hardcoded 60 and the flag beside it leaves the display
//! at whatever the desktop is using.
//!
//! Patches are written into the running process and nothing on disk is touched,
//! so closing the game undoes all of it.
//!
//! The byte patterns come from uberhalit's EldenRingFpsUnlockAndMore (MIT), and
//! were re-checked against 1.16.1 rather than trusted: both match exactly once,
//! and the bytes under them are the same instructions the original documented.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// `mov dword ptr [rbx+1C], 3C888889` — the seconds-per-frame the game runs at.
///
/// The last byte of the float is 88, 89 or 90 depending on how the compiler
/// rounded, so it is a wildcard.
const FRAMELOCK: &str = "C7 ?? ?? ?? 88 88 3C EB";
const FRAMELOCK_OFFSET: usize = 3;

/// The same instruction reached from its neighbours, for builds where the float
/// itself moved.
const FRAMELOCK_FUZZY: &str = "89 73 ?? C7 ?? ?? ?? ?? ?? ?? EB ?? 89 73";
const FRAMELOCK_FUZZY_OFFSET: usize = 6;

/// `mov [rbp-11], 3C` then `mov [rbp-D], 1` — ask Windows for 60 Hz, and mean it.
const HERTZLOCK: &str = "EB ?? C7 ?? ?? 3C 00 00 00 C7 ?? ?? 01 00 00 00";
/// The same instruction pair after Roundtable has been through it.
///
/// The patch overwrites the two immediates the pattern matches on, so a second
/// look never finds the original again — which would leave the cap impossible to
/// put back. Both forms have to be searched for.
const HERTZLOCK_PATCHED: &str = "EB ?? C7 ?? ?? 00 00 00 00 C7 ?? ?? 00 00 00 00";
const HERTZLOCK_OFFSET: usize = 2;
/// Where the 60 sits, relative to the first `mov`.
const HERTZLOCK_HZ: usize = 3;
/// Where the "this is a refresh rate change" flag sits.
const HERTZLOCK_FLAG: usize = 10;

/// The game will not run below this, and above it physics start to drift.
const MIN_FPS: u32 = 30;
const MAX_FPS: u32 = 360;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockReport {
    /// What the cap now is.
    pub fps: u32,
    /// True when the frame limiter itself was rewritten.
    pub framelock: bool,
    /// True when the hardcoded 60 Hz display request was cleared. This is the
    /// one that stops the halving to 30.
    pub hertz: bool,
}

/// Turns `"C7 ?? 3C"` into bytes and a mask.
pub(crate) fn parse(pattern: &str) -> Vec<Option<u8>> {
    pattern
        .split_whitespace()
        .map(|token| {
            if token == "??" {
                None
            } else {
                u8::from_str_radix(token, 16).ok()
            }
        })
        .collect()
}

/// The one place a pattern occurs, or nothing.
///
/// A pattern that matches twice is a pattern that is no longer specific enough,
/// and guessing which hit to patch is how a launcher corrupts somebody's game.
/// Both of these match exactly once, so a second hit means the build changed and
/// the right answer is to do nothing.
pub(crate) fn find_only(haystack: &[u8], pattern: &[Option<u8>]) -> Option<usize> {
    let Some(first) = pattern.first().copied() else {
        return None;
    };
    if haystack.len() < pattern.len() {
        return None;
    }

    let mut found = None;
    let last = haystack.len() - pattern.len();
    for at in 0..=last {
        if let Some(byte) = first {
            if haystack[at] != byte {
                continue;
            }
        }
        let hit = pattern
            .iter()
            .enumerate()
            .all(|(index, want)| want.is_none_or(|byte| haystack[at + index] == byte));
        if hit {
            if found.is_some() {
                return None;
            }
            found = Some(at);
        }
    }
    found
}

#[cfg(windows)]
pub(crate) mod win {
    use super::{Error, Result};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        ReadProcessMemory, WriteProcessMemory,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        EnumProcessModules, GetModuleInformation, MODULEINFO,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};

    /// An open handle that closes itself.
    pub struct Process(HANDLE);

    impl Drop for Process {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Process {
        pub fn handle(&self) -> HANDLE {
            self.0
        }

        pub fn open(pid: u32) -> Result<Self> {
            let handle = unsafe { OpenProcess(PROCESS_ALL_ACCESS, 0, pid) };
            if handle.is_null() {
                return Err(Error::msg(
                    "could not open the game. Run Roundtable as administrator and try again."
                        .to_string(),
                ));
            }
            Ok(Self(handle))
        }

        /// Base address and size of the executable's own module.
        pub fn main_module(&self) -> Result<(usize, usize)> {
            let mut modules = [std::ptr::null_mut(); 1];
            let mut needed = 0u32;
            let ok = unsafe {
                EnumProcessModules(
                    self.0,
                    modules.as_mut_ptr(),
                    std::mem::size_of_val(&modules) as u32,
                    &mut needed,
                )
            };
            if ok == 0 || modules[0].is_null() {
                return Err(Error::msg("could not read the game's modules".to_string()));
            }

            let mut info: MODULEINFO = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                GetModuleInformation(
                    self.0,
                    modules[0],
                    &mut info,
                    std::mem::size_of::<MODULEINFO>() as u32,
                )
            };
            if ok == 0 {
                return Err(Error::msg("could not measure the game's module".to_string()));
            }
            Ok((info.lpBaseOfDll as usize, info.SizeOfImage as usize))
        }

        /// Copies a span of the game's memory out.
        ///
        /// Read in pages rather than one call: an image has holes in it, and one
        /// unreadable page would otherwise fail the whole read. A hole reads back
        /// as zeroes, which matches nothing.
        pub fn read(&self, at: usize, len: usize) -> Vec<u8> {
            const CHUNK: usize = 1 << 20;
            let mut out = vec![0u8; len];
            let mut done = 0usize;
            while done < len {
                let size = CHUNK.min(len - done);
                let mut got = 0usize;
                unsafe {
                    ReadProcessMemory(
                        self.0,
                        (at + done) as *const _,
                        out[done..].as_mut_ptr().cast(),
                        size,
                        &mut got,
                    );
                }
                done += size;
            }
            out
        }

        /// Writes, then reads the same bytes back.
        ///
        /// `WriteProcessMemory` reporting success is not the same as the bytes
        /// being there: the page can be copy-on-write, or something else can be
        /// writing to the same address. Reading it back is the only way to say
        /// the patch is applied rather than attempted.
        pub fn write(&self, at: usize, bytes: &[u8]) -> bool {
            let mut written = 0usize;
            let ok = unsafe {
                WriteProcessMemory(
                    self.0,
                    at as *const _,
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    &mut written,
                )
            };
            if ok == 0 || written != bytes.len() {
                return false;
            }
            self.read(at, bytes.len()) == bytes
        }
    }
}

/// The running game, if it is running.
pub fn running_pid(executable: &str) -> Option<u32> {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    system
        .processes()
        .values()
        .find(|process| {
            process
                .name()
                .to_string_lossy()
                .eq_ignore_ascii_case(executable)
        })
        .map(|process| process.pid().as_u32())
}

/// Rewrites the cap in the running game.
///
/// `fps` of 0 puts it back to 60, which is what the game shipped with.
#[cfg(windows)]
pub fn unlock(executable: &str, fps: u32) -> Result<UnlockReport> {
    let fps = if fps == 0 { 60 } else { fps.clamp(MIN_FPS, MAX_FPS) };

    let pid = running_pid(executable)
        .ok_or_else(|| Error::msg("the game is not running".to_string()))?;
    let process = win::Process::open(pid)?;
    let (base, size) = process.main_module()?;
    let image = process.read(base, size);

    // Seconds per frame, which is what the instruction actually holds.
    let delta = (1000.0f32 / fps as f32) / 1000.0;

    let framelock = find_only(&image, &parse(FRAMELOCK))
        .map(|at| at + FRAMELOCK_OFFSET)
        .or_else(|| find_only(&image, &parse(FRAMELOCK_FUZZY)).map(|at| at + FRAMELOCK_FUZZY_OFFSET))
        .is_some_and(|at| process.write(base + at, &delta.to_le_bytes()));

    // Clearing both the 60 and the flag beside it leaves the display alone. Put
    // them back when the cap goes back to 60, or the game stops asking for a
    // mode at all.
    let hertz = find_only(&image, &parse(HERTZLOCK))
        .or_else(|| find_only(&image, &parse(HERTZLOCK_PATCHED)))
        .map(|at| at + HERTZLOCK_OFFSET)
        .is_some_and(|at| {
            let (hz, flag) = if fps == 60 {
                (60u32, 1u32)
            } else {
                (0u32, 0u32)
            };
            process.write(base + at + HERTZLOCK_HZ, &hz.to_le_bytes())
                && process.write(base + at + HERTZLOCK_FLAG, &flag.to_le_bytes())
        });

    if !framelock && !hertz {
        return Err(Error::msg(
            "this build of the game does not match any known frame cap".to_string(),
        ));
    }

    Ok(UnlockReport {
        fps,
        framelock,
        hertz,
    })
}

#[cfg(not(windows))]
pub fn unlock(_executable: &str, _fps: u32) -> Result<UnlockReport> {
    Err(Error::msg("only on Windows".to_string()))
}

/// Puts the game above the browsers and chat clients in the scheduler.
///
/// Above normal rather than high: high starves the audio thread and the mouse,
/// which trades a stutter for a worse one. Above normal is enough to stop a
/// browser tab taking a slice at the wrong moment.
#[cfg(windows)]
pub fn raise_priority(executable: &str) -> Result<()> {
    use windows_sys::Win32::System::Threading::{
        SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
    };

    let pid = running_pid(executable)
        .ok_or_else(|| Error::msg("the game is not running".to_string()))?;
    let process = win::Process::open(pid)?;
    if unsafe { SetPriorityClass(process.handle(), ABOVE_NORMAL_PRIORITY_CLASS) } == 0 {
        return Err(Error::msg("could not change the game's priority".to_string()));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn raise_priority(_executable: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instruction the frame limiter lives in, as it appears in 1.16.1.
    const FRAME_SITE: &[u8] = &[
        0xC7, 0x43, 0x1C, 0x89, 0x88, 0x88, 0x3C, 0xEB, 0x6D, 0x89, 0x73, 0x18,
    ];
    /// The 60 Hz request, as it appears in 1.16.1.
    const HERTZ_SITE: &[u8] = &[
        0xEB, 0x0E, 0xC7, 0x45, 0xEF, 0x3C, 0x00, 0x00, 0x00, 0xC7, 0x45, 0xF3, 0x01, 0x00,
        0x00, 0x00, 0x8B, 0x87,
    ];

    #[test]
    fn a_pattern_reads_as_bytes_and_wildcards() {
        let parsed = parse("C7 ?? 3C");
        assert_eq!(parsed, vec![Some(0xC7), None, Some(0x3C)]);
    }

    #[test]
    fn the_frame_limiter_is_found_where_the_float_is() {
        let at = find_only(FRAME_SITE, &parse(FRAMELOCK)).unwrap() + FRAMELOCK_OFFSET;
        let float = f32::from_le_bytes(FRAME_SITE[at..at + 4].try_into().unwrap());
        // 1/60 of a second per frame is the cap being removed.
        assert!((1.0 / float - 60.0).abs() < 0.01, "got {float}");
    }

    #[test]
    fn the_fuzzy_pattern_lands_on_the_same_float() {
        // Both routes have to agree, or one of them is patching the wrong bytes.
        let mut padded = vec![0x89u8, 0x73, 0x18];
        padded.extend_from_slice(FRAME_SITE);
        padded.extend_from_slice(&[0x89, 0x73]);

        let direct = find_only(&padded, &parse(FRAMELOCK)).unwrap() + FRAMELOCK_OFFSET;
        let fuzzy = find_only(&padded, &parse(FRAMELOCK_FUZZY)).unwrap() + FRAMELOCK_FUZZY_OFFSET;
        assert_eq!(direct, fuzzy);
    }

    #[test]
    fn the_hardcoded_sixty_hertz_is_found() {
        let at = find_only(HERTZ_SITE, &parse(HERTZLOCK)).unwrap() + HERTZLOCK_OFFSET;
        let hz = u32::from_le_bytes(
            HERTZ_SITE[at + HERTZLOCK_HZ..at + HERTZLOCK_HZ + 4]
                .try_into()
                .unwrap(),
        );
        let flag = u32::from_le_bytes(
            HERTZ_SITE[at + HERTZLOCK_FLAG..at + HERTZLOCK_FLAG + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(hz, 60, "the hardcoded refresh rate");
        assert_eq!(flag, 1, "the flag that says a mode change is wanted");
    }

    #[test]
    fn the_patch_can_be_found_again_after_it_has_been_applied() {
        // Otherwise the cap can be removed but never put back, because the
        // pattern matches on the very bytes the patch overwrites.
        let mut patched = HERTZ_SITE.to_vec();
        let at = find_only(HERTZ_SITE, &parse(HERTZLOCK)).unwrap() + HERTZLOCK_OFFSET;
        patched[at + HERTZLOCK_HZ..at + HERTZLOCK_HZ + 4].copy_from_slice(&0u32.to_le_bytes());
        patched[at + HERTZLOCK_FLAG..at + HERTZLOCK_FLAG + 4].copy_from_slice(&0u32.to_le_bytes());

        assert!(
            find_only(&patched, &parse(HERTZLOCK)).is_none(),
            "the original form is gone, which is the whole problem"
        );
        let again = find_only(&patched, &parse(HERTZLOCK_PATCHED)).unwrap() + HERTZLOCK_OFFSET;
        assert_eq!(again, at, "and the patched form lands on the same place");
    }

    #[test]
    fn the_frame_limiter_can_be_found_again_after_it_has_been_applied() {
        // The float itself is in the pattern, so only the fuzzy route survives a
        // patch. It has to, or the cap cannot be changed twice.
        let mut padded = vec![0x89u8, 0x73, 0x18];
        padded.extend_from_slice(FRAME_SITE);
        padded.extend_from_slice(&[0x89, 0x73]);

        let at = find_only(&padded, &parse(FRAMELOCK)).unwrap() + FRAMELOCK_OFFSET;
        let delta = (1000.0f32 / 90.0) / 1000.0;
        padded[at..at + 4].copy_from_slice(&delta.to_le_bytes());

        assert!(find_only(&padded, &parse(FRAMELOCK)).is_none());
        let again = find_only(&padded, &parse(FRAMELOCK_FUZZY)).unwrap() + FRAMELOCK_FUZZY_OFFSET;
        assert_eq!(again, at);
    }

    #[test]
    fn a_pattern_that_matches_twice_is_refused() {
        // Two hits means the pattern no longer identifies one instruction, and
        // patching either would be a guess.
        let mut twice = FRAME_SITE.to_vec();
        twice.extend_from_slice(FRAME_SITE);
        assert!(find_only(&twice, &parse(FRAMELOCK)).is_none());
    }

    #[test]
    fn nothing_is_found_in_bytes_that_do_not_hold_it() {
        assert!(find_only(&[0u8; 64], &parse(FRAMELOCK)).is_none());
        assert!(find_only(&[], &parse(FRAMELOCK)).is_none());
        assert!(find_only(&[0xC7], &parse(FRAMELOCK)).is_none());
    }

    #[test]
    fn the_delta_written_is_seconds_per_frame() {
        for fps in [60u32, 144, 180, 240] {
            let delta = (1000.0f32 / fps as f32) / 1000.0;
            assert!((1.0 / delta - fps as f32).abs() < 0.05, "{fps}");
        }
    }
}
