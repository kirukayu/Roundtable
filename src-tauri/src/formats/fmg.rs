//! The game's own words, read off the disk instead of out of the process.
//!
//! [`crate::text`] already reads these while the game runs, which is the better
//! source when there is one: it is the copy the player is actually looking at.
//! But the game is not always running — and a marker has to be written with it
//! closed — so the same words have to be readable from the files as well.
//!
//! An FMG is a flat table of numbered strings. Ids are not dense, so instead of
//! one entry each there are runs:
//!
//! ```text
//! 0x0c u32   how many runs
//! 0x10 u32   how many strings
//! 0x18 u64   where the offset table starts
//! 0x28       the runs, sixteen bytes each:
//!              u32 the offset table index this run starts at
//!              u32 the first id in the run
//!              u32 the last id
//!              u32 --
//! ```
//!
//! and a string's offset is `table[run.index + (id - run.first)]`, counted from
//! the start of the file. Zero means the id exists with nothing behind it,
//! which the game uses for entries it has removed.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};

/// A table's worth of strings.
pub type Strings = HashMap<u32, String>;

/// Refuse anything claiming more than this. A wrong header costs a message
/// rather than the machine's memory.
const MOST_RUNS: usize = 100_000;
const MOST_STRINGS: usize = 500_000;

/// Reads one FMG.
pub fn parse(bytes: &[u8]) -> Result<Strings> {
    let fail = |detail: String| Error::Parse {
        what: "message table".into(),
        detail,
    };

    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let long = |at: usize| -> Option<u64> {
        Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
    };

    let runs = word(0x0c).ok_or_else(|| fail("too short to hold a header".into()))? as usize;
    let strings = word(0x10).unwrap_or(0) as usize;
    let table = long(0x18).ok_or_else(|| fail("no offset table".into()))? as usize;

    if runs > MOST_RUNS || strings > MOST_STRINGS {
        return Err(fail(format!("claims {runs} runs and {strings} strings")));
    }
    if table >= bytes.len() {
        return Err(fail(format!(
            "the offset table starts at {table}, past the end of {} bytes",
            bytes.len()
        )));
    }

    let mut out = Strings::with_capacity(strings);
    for run in 0..runs {
        let at = 0x28 + run * 16;
        let (Some(index), Some(first), Some(last)) =
            (word(at), word(at + 4), word(at + 8))
        else {
            break;
        };
        if last < first || (last - first) as usize > MOST_STRINGS {
            continue;
        }

        for id in first..=last {
            let slot = index as usize + (id - first) as usize;
            let Some(offset) = long(table + slot * 8) else {
                break;
            };
            // Zero is an id the game knows about and has nothing to say for.
            if offset == 0 {
                continue;
            }
            let Ok(offset) = usize::try_from(offset) else {
                continue;
            };
            if offset >= bytes.len() {
                continue;
            }
            let text = wide_at(bytes, offset);
            if !text.is_empty() {
                out.insert(id, text);
            }
        }
    }

    if out.is_empty() {
        return Err(fail("no strings came out of it".into()));
    }
    Ok(out)
}

/// Every table in one of the game's `.msgbnd.dcx` archives, by file name.
///
/// The archive is Kraken-packed, so [`super::oodle::register`] must have been
/// told where the game is first.
pub fn archive(path: &Path) -> Result<HashMap<String, Strings>> {
    let raw = std::fs::read(path).map_err(|problem| Error::Parse {
        what: path.display().to_string(),
        detail: problem.to_string(),
    })?;
    tables(&raw, &path.display().to_string())
}

/// The same, for an archive that came out of the game's own packed files rather
/// than off the disk — where a plain installation keeps every one of them.
pub fn tables(raw: &[u8], what: &str) -> Result<HashMap<String, Strings>> {
    let unpacked = if super::dcx::wraps(raw) {
        super::dcx::expand(raw, "message archive")?
    } else {
        raw.to_vec()
    };

    let mut out = HashMap::new();
    for (name, bytes) in super::regulation::unpack(&unpacked)? {
        // The names come through as full paths inside the archive.
        let short = name
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&name)
            .trim_end_matches(".fmg")
            .to_string();
        if let Ok(strings) = parse(&bytes) {
            out.insert(short, strings);
        }
    }
    if out.is_empty() {
        return Err(Error::Parse {
            what: what.to_string(),
            detail: "held no message tables".into(),
        });
    }
    Ok(out)
}

/// The message archives a plain installation keeps packed, for one language.
///
/// Where a modded copy has loose `.msgbnd.dcx` files under `msg/<locale>`, an
/// untouched one has the same archives inside `Data0.bhd` and nothing on disk
/// to read. `item` carries every name, effect line and paragraph the launcher
/// answers with; `menu` carries the interface's own words.
///
/// The caller must have registered the game with [`super::oodle`] first, since
/// what comes out is Kraken-packed.
pub fn packed(game_dir: &Path, locale: &str) -> HashMap<String, Strings> {
    let wanted: Vec<String> = ["item", "menu"]
        .iter()
        .map(|which| format!("/msg/{locale}/{which}.msgbnd.dcx"))
        .collect();
    let names: Vec<u64> = wanted.iter().map(|path| super::bhd5::hash(path)).collect();

    let Some(archive) = super::bhd5::holding(game_dir, &names) else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for path in &wanted {
        let Some(raw) = archive.read(path) else {
            continue;
        };
        let Ok(read) = tables(&raw, path) else {
            continue;
        };
        for (name, strings) in read {
            out.entry(name).or_insert(strings);
        }
    }
    out
}

/// A UTF-16 string, to its terminator.
fn wide_at(data: &[u8], at: usize) -> String {
    super::regulation::wide_at(data, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an FMG the way the game writes one: header, runs, offset table,
    /// then the strings.
    fn table(entries: &[(u32, &str)]) -> Vec<u8> {
        let mut runs: Vec<(u32, u32, u32)> = Vec::new();
        for (slot, (id, _)) in entries.iter().enumerate() {
            match runs.last_mut() {
                Some(last) if last.2 + 1 == *id => last.2 = *id,
                _ => runs.push((slot as u32, *id, *id)),
            }
        }

        let table_at = 0x28 + runs.len() * 16;
        let strings_at = table_at + entries.len() * 8;

        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for (_, text) in entries {
            if text.is_empty() {
                offsets.push(0u64);
                continue;
            }
            offsets.push((strings_at + body.len()) as u64);
            for unit in text.encode_utf16() {
                body.extend_from_slice(&unit.to_le_bytes());
            }
            body.extend_from_slice(&[0, 0]);
        }

        let mut out = vec![0u8; 0x28];
        out[0x0c..0x10].copy_from_slice(&(runs.len() as u32).to_le_bytes());
        out[0x10..0x14].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        out[0x18..0x20].copy_from_slice(&(table_at as u64).to_le_bytes());
        for (index, first, last) in &runs {
            out.extend_from_slice(&index.to_le_bytes());
            out.extend_from_slice(&first.to_le_bytes());
            out.extend_from_slice(&last.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        for offset in offsets {
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn numbered_strings_come_back_by_their_number() {
        let bytes = table(&[
            (1000, "Reduvia"),
            (1001, "Blood Cleric"),
            (5000, "Stormveil Castle"),
        ]);
        let found = parse(&bytes).expect("a table this file wrote");
        assert_eq!(found.get(&1000).map(String::as_str), Some("Reduvia"));
        assert_eq!(found.get(&1001).map(String::as_str), Some("Blood Cleric"));
        assert_eq!(found.get(&5000).map(String::as_str), Some("Stormveil Castle"));
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn ids_the_game_removed_are_absent_rather_than_empty() {
        // Offset zero is how the game marks an id it no longer has words for.
        // Returning "" for those would put blanks in front of the player.
        let bytes = table(&[(10, "kept"), (11, ""), (12, "also kept")]);
        let found = parse(&bytes).expect("a table this file wrote");
        assert_eq!(found.len(), 2);
        assert!(!found.contains_key(&11));
    }

    #[test]
    fn cyrillic_survives() {
        // Half the point of reading these is the Russian text.
        let bytes = table(&[(1, "Редувия"), (2, "Замок Штормвейл")]);
        let found = parse(&bytes).expect("a table this file wrote");
        assert_eq!(found.get(&1).map(String::as_str), Some("Редувия"));
        assert_eq!(found.get(&2).map(String::as_str), Some("Замок Штормвейл"));
    }

    #[test]
    fn a_header_pointing_past_the_end_is_refused() {
        let mut bytes = table(&[(1, "x")]);
        bytes[0x18..0x20].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&bytes).is_err());

        bytes = table(&[(1, "x")]);
        bytes[0x0c..0x10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&bytes).is_err());

        assert!(parse(&[]).is_err());
        assert!(parse(&[0u8; 8]).is_err());
    }

    /// What the packed archives actually call their tables.
    ///
    /// `cargo test --lib formats::fmg::tests::show_the_packed_tables -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_the_packed_tables() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        super::super::oodle::register(&game);
        for name in ["item", "menu", "ngword"] {
            for locale in ["engus", "rusru"] {
                let want = format!("/msg/{locale}/{name}.msgbnd.dcx");
                let Some(found) = crate::formats::bhd5::holding(
                    &game,
                    &[crate::formats::bhd5::hash(&want)],
                ) else {
                    println!("{want}: in no archive");
                    continue;
                };
                let Some(bytes) = found.read(&want) else {
                    println!("{want}: indexed but unreadable");
                    continue;
                };
                match tables(&bytes, &want) {
                    Ok(got) => {
                        let mut named: Vec<(&String, usize)> =
                            got.iter().map(|(name, table)| (name, table.len())).collect();
                        named.sort();
                        println!("\n{want}: {} tables", named.len());
                        for (table, count) in named {
                            println!("    {table:<32} {count:>6}");
                        }
                    }
                    Err(why) => println!("{want}: {why}"),
                }
            }
        }
    }

    /// An installed game, if this machine has one.
    ///
    /// Vanilla keeps its text inside the packed archives, so what this finds in
    /// practice is a mod's loose `msg` folder — which is the case that matters
    /// anyway, since the mod's words are the ones the player sees.
    #[test]
    fn a_real_archive_reads() {
        let Some((game, installed)) =
            crate::testing::installed(crate::games::Game::EldenRing)
        else {
            return;
        };
        let msg = installed.join("msg");
        if !msg.is_dir() {
            return;
        }
        super::super::oodle::register(&game);
        if !super::super::oodle::available() {
            return;
        }

        // English and Russian, because the launcher answers in both and a
        // wrong encoding shows up in one before the other.
        let mut read = 0;
        for language in ["engus", "rusru"] {
            let Ok(files) = std::fs::read_dir(msg.join(language)) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if !path.to_string_lossy().ends_with(".msgbnd.dcx") {
                    continue;
                }
                let tables = archive(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let total: usize = tables.values().map(HashMap::len).sum();
                assert!(total > 100, "only {total} strings in {}", path.display());

                // Words, not markup and not mojibake.
                let sample = tables
                    .values()
                    .flat_map(|table| table.values())
                    .find(|text| text.chars().count() > 3)
                    .expect("something with words in it");
                assert!(
                    sample.chars().any(char::is_alphabetic),
                    "read back {sample:?}"
                );
                if language == "rusru" {
                    let cyrillic = tables
                        .values()
                        .flat_map(|table| table.values())
                        .any(|text| text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)));
                    assert!(cyrillic, "the Russian archive came back without Russian");
                }
                read += 1;
            }
        }
        assert!(read > 0, "no message archive was read");
    }
}
