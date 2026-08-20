//! Oodle, borrowed from the game that already has it.
//!
//! ELDEN RING packs most of its files with Oodle Kraken — `regulation.bin` is
//! the exception, and the reason the reader for that one got away with ZSTD.
//! Everything else, the message archives included, comes out `KRAK`.
//!
//! There is no open decoder for it, and shipping one would mean shipping
//! somebody's licensed library. What there is, on every machine this launcher
//! runs on, is `oo2core_6_win64.dll` sitting next to the game's own executable —
//! so the launcher borrows the copy the player already has rather than carrying
//! one. Nothing is written and nothing is injected: a library is loaded and a
//! function is called on a buffer.
//!
//! It has to be told where the game is. Guessing at directory layouts is how a
//! launcher ends up loading a stranger's DLL, so the caller registers the game
//! directory it already knows and this looks nowhere else.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::error::{Error, Result};

/// Names in the order they have shipped, newest first.
const NAMES: [&str; 3] = [
    "oo2core_6_win64.dll",
    "oo2core_9_win64.dll",
    "oo2core_8_win64.dll",
];

/// Directories the launcher has said contain a game.
fn known() -> &'static Mutex<Vec<PathBuf>> {
    static KNOWN: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    KNOWN.get_or_init(|| Mutex::new(Vec::new()))
}

/// Tells this where a game lives, so its copy of the library can be used.
///
/// Cheap and idempotent — the launcher calls it whenever it learns of an
/// installation, and nothing is loaded until something needs decompressing.
pub fn register(game_dir: &Path) {
    let Ok(mut dirs) = known().lock() else {
        return;
    };
    if !dirs.iter().any(|had| had == game_dir) {
        dirs.push(game_dir.to_path_buf());
    }
}

/// The first copy of the library among the registered directories.
pub fn library_path() -> Option<PathBuf> {
    let dirs = known().lock().ok()?;
    for dir in dirs.iter() {
        for name in NAMES {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Whether anything can be decompressed at all, for a message that says why
/// rather than a failure that does not.
pub fn available() -> bool {
    entry().is_some()
}

#[cfg(windows)]
type Decompress = unsafe extern "system" fn(
    *const u8,   // the compressed bytes
    isize,       // how many
    *mut u8,     // where they go
    isize,       // how much room
    i32,         // fuzz safe
    i32,         // check the crc
    i32,         // verbosity
    *mut u8,     // decoder buffer
    isize,       // its size
    *mut u8,     // progress callback
    *mut u8,     // its argument
    *mut u8,     // scratch
    isize,       // its size
    i32,         // thread phase
) -> isize;

/// The loaded function, found once.
///
/// The library is deliberately never freed: it is loaded at most once per run,
/// and unloading it under a caller that still holds the pointer is the kind of
/// bug that shows up as a crash somewhere else entirely.
#[cfg(windows)]
fn entry() -> Option<Decompress> {
    // Only success is remembered. A `OnceLock` here caches the failure too, and
    // the first caller is often the one that arrives before any game has been
    // registered — so one early miss killed every decompression for the rest of
    // the run, and the launcher went quiet about the game's text with no way
    // back short of restarting it.
    static ENTRY: Mutex<Option<usize>> = Mutex::new(None);

    let mut found = ENTRY.lock().ok()?;
    if found.is_none() {
        *found = load();
    }
    let address = (*found)?;

    // SAFETY: the address came from GetProcAddress for this exact name, and the
    // signature is Oodle's published one.
    Some(unsafe { std::mem::transmute::<usize, Decompress>(address) })
}

/// Finds and loads the library, or says it could not this time.
#[cfg(windows)]
fn load() -> Option<usize> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    let path = library_path()?;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: a null-terminated path to a file that exists; a failure comes
    // back as a null handle rather than as anything worse.
    let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: the handle is live, and the name is one byte string.
    let found = unsafe { GetProcAddress(handle, c"OodleLZ_Decompress".as_ptr().cast()) };
    found.map(|found| found as usize)
}

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(not(windows))]
fn entry() -> Option<()> {
    None
}

/// Expands a Kraken-packed block.
///
/// `expanded` is what the container promised, and it is also the size of the
/// buffer handed over — Oodle needs to be told exactly how much room it has.
pub fn decompress(packed: &[u8], expanded: usize) -> Result<Vec<u8>> {
    let fail = |detail: String| Error::Parse {
        what: "oodle".into(),
        detail,
    };

    if expanded == 0 {
        return Ok(Vec::new());
    }

    #[cfg(windows)]
    {
        let Some(decompress) = entry() else {
            return Err(fail(
                "the game's oo2core library was not found, so files packed with \
                 Kraken cannot be read"
                    .into(),
            ));
        };

        let mut out = vec![0u8; expanded];
        // SAFETY: both buffers are live and their lengths are passed honestly.
        // The trailing arguments are Oodle's documented defaults: fuzz-safe on,
        // no CRC, silent, no caller-supplied memory, unthreaded.
        let wrote = unsafe {
            decompress(
                packed.as_ptr(),
                packed.len() as isize,
                out.as_mut_ptr(),
                expanded as isize,
                1,
                0,
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                3,
            )
        };

        if wrote <= 0 {
            return Err(fail(format!(
                "refused {} packed bytes that should have become {expanded}",
                packed.len()
            )));
        }
        let wrote = wrote as usize;
        if wrote != expanded {
            return Err(fail(format!(
                "gave back {wrote} bytes where the container promised {expanded}"
            )));
        }
        out.truncate(wrote);
        Ok(out)
    }

    #[cfg(not(windows))]
    {
        let _ = packed;
        Err(fail("Oodle is only available on Windows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_loaded_from_a_directory_nobody_named() {
        // The point of registering rather than searching: a launcher that goes
        // looking for a DLL by name is a launcher that can be handed one.
        let nowhere = Path::new("Z:\\no such place");
        register(nowhere);
        assert!(
            library_path().is_none_or(|found| found.parent() != Some(nowhere)),
            "loaded something out of a directory that has no game in it"
        );
    }

    #[test]
    fn an_empty_block_expands_to_nothing() {
        assert!(decompress(&[], 0).expect("nothing is nothing").is_empty());
    }

    /// Asking too early must not settle the answer for good.
    ///
    /// The launcher registers a game when it learns of one, and something can
    /// easily ask for a decompression before that. Caching the miss meant the
    /// game's text stayed unreadable for the rest of the run, which showed up
    /// as a Russian answer to an English question and took a while to trace.
    #[test]
    fn a_miss_before_any_game_is_known_is_not_remembered_as_the_answer() {
        // Whatever the state now, asking with nothing registered must not stop
        // a later, properly registered attempt from working.
        let _ = available();

        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        register(&game);
        assert!(
            available(),
            "the library is next to the game and was still not found"
        );
    }

    #[test]
    fn rubbish_is_refused_rather_than_returned() {
        if !available() {
            return;
        }
        let refused = decompress(&[0u8; 64], 4096);
        assert!(refused.is_err(), "{refused:?}");
    }
}
