//! The archives the game ships everything inside, for a player who never
//! unpacked them.
//!
//! A total conversion leaves the files it replaces loose on disk, so this
//! launcher could read a modded install from the start. A plain one has them in
//! `Data0.bhd` and its siblings, and until this existed anything that wanted a
//! map or a message had nothing to open.
//!
//! Four layers, and every one was settled against real bytes before it was
//! written — see `assets/param-layout.md` under "The packed archives":
//!
//! ```text
//! Data2.bhd   RSA, 256 bytes in and 255 out, no padding scheme
//!   └ BHD5    buckets of records, each naming a file by the hash of its path
//! Data2.bdt   the files themselves, at the offsets the records give
//!   └ AES     128-bit, ECB, over the ranges the record lists
//! ```
//!
//! What comes out is an ordinary DCX, which [`super::dcx`] already opens.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use num_bigint::BigUint;
use parking_lot::Mutex;

use crate::error::{Error, Result};

/// The keys are not labelled and do not need to be: the right one is the one
/// whose first block comes out saying `BHD5`, and no wrong key can fake four
/// bytes of a 2048-bit result.
const KEYS: &str = include_str!("../../assets/archive-keys.txt");

/// Bytes in per block, and bytes out. The odd 255 is the RSA result with its
/// leading byte dropped, which is what the game wrote and not a choice here.
const LOCKED: usize = 256;
const OPENED: usize = 255;

/// Refuse a record claiming a file larger than this. A corrupt or hostile index
/// should cost a message rather than the machine's memory, and the largest
/// thing any of these archives actually holds is far below it.
const BIGGEST: usize = 512 * 1024 * 1024;

/// Where the index says its own shape is. Read off the real header, whose first
/// bytes are `BHD5`, then `ff 01 00 00`, then a one that is always a one.
mod head {
    pub const SIZE: usize = 0x0c;
    pub const BUCKETS: usize = 0x10;
    pub const BUCKETS_AT: usize = 0x14;
    pub const SALT_LEN: usize = 0x18;
}

/// A record, forty bytes of it.
mod record {
    pub const WIDE: usize = 40;
    pub const HASH: usize = 0x00;
    /// Rounded up to the encryption's block; the real length is beside it, and
    /// is zero when the file was not padded at all.
    pub const PADDED: usize = 0x08;
    pub const SIZE: usize = 0x0c;
    pub const AT: usize = 0x10;
    pub const AES_AT: usize = 0x20;
}

/// A key entry: the key, then how many ranges of the file it covers, then the
/// ranges. The spacing between one record's key and its neighbour's is 36 or
/// 100 bytes, which is exactly one range or five — that is how the shape was
/// settled, before any of it was read as a key.
mod lock {
    pub const KEY: usize = 16;
    pub const RANGES: usize = 0x10;
    pub const FIRST: usize = 0x14;
    pub const WIDE: usize = 16;
}

/// A file's key, and the ranges of it that were encrypted with it.
type Keyed = ([u8; 16], Vec<(usize, usize)>);

/// One file, as the index describes it.
#[derive(Debug, Clone)]
struct Entry {
    /// Where it starts in the `.bdt`.
    at: u64,
    /// What to read: the padded length, which is what is actually written.
    padded: usize,
    /// What to keep. Zero in the index means the file was never padded, in
    /// which case the padded length is the real one.
    size: usize,
    /// Where its key sits in the index, resolved only if the file is read.
    key_at: usize,
}

/// An opened index, and the data file beside it.
pub struct Archive {
    /// `Data2.bdt` for `Data2.bhd`. Held as a path rather than a handle: it is
    /// eleven gigabytes and the launcher may sit for hours between questions.
    data: PathBuf,
    files: HashMap<u64, Entry>,
    index: Mutex<Index>,
}

/// The index, decrypted as far as it has had to be.
///
/// A block at a time, because opening one file needs a few hundred of the
/// twenty-eight thousand `Data2.bhd` holds, and the difference is most of a
/// second every time the launcher starts.
struct Index {
    locked: Vec<u8>,
    plain: Vec<u8>,
    opened: Vec<bool>,
    modulus: BigUint,
    exponent: BigUint,
}

impl Index {
    /// Decrypts whatever covers this range, and hands it back.
    fn range(&mut self, at: usize, len: usize) -> Option<&[u8]> {
        let last = at.checked_add(len)?.checked_sub(1)?;
        for block in at / OPENED..=last / OPENED {
            self.open(block)?;
        }
        self.plain.get(at..at + len)
    }

    fn open(&mut self, block: usize) -> Option<()> {
        if *self.opened.get(block)? {
            return Some(());
        }
        let from = block * LOCKED;
        let bytes = BigUint::from_bytes_be(self.locked.get(from..from + LOCKED)?)
            .modpow(&self.exponent, &self.modulus)
            .to_bytes_be();
        // A 2048-bit result is 256 bytes; anything shorter lost leading zeros
        // on the way out of the bigint and has to have them put back, or every
        // byte after it lands one place early.
        let mut whole = [0u8; LOCKED];
        let from = LOCKED.checked_sub(bytes.len())?;
        whole.get_mut(from..)?.copy_from_slice(&bytes);
        self.plain
            .get_mut(block * OPENED..(block + 1) * OPENED)?
            .copy_from_slice(&whole[1..]);
        self.opened[block] = true;
        Some(())
    }
}

/// Every key, read once.
fn keys() -> &'static Vec<(BigUint, BigUint)> {
    static KEYS_ONCE: OnceLock<Vec<(BigUint, BigUint)>> = OnceLock::new();
    KEYS_ONCE.get_or_init(|| {
        KEYS.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                let (modulus, exponent) = line.split_once(' ')?;
                Some((
                    BigUint::from_bytes_be(&hex::decode(modulus).ok()?),
                    BigUint::from(exponent.trim().parse::<u32>().ok()?),
                ))
            })
            .collect()
    })
}

/// What the game calls a file, folded into what the index stores.
///
/// Lowered, with the separators turned round and a leading one added, then
/// folded by 0x85. Checked against every record in every archive: each one
/// lands in the bucket its own hash names, which no wrong fold would manage
/// a hundred and twenty thousand times running.
pub fn hash(path: &str) -> u64 {
    let path = path.trim();
    let lead = u64::from(!path.starts_with('/') && !path.starts_with('\\'));
    fold(lead * u64::from(b'/'), path)
}

/// Folds more of a path into a hash already begun.
///
/// Public because it is a left fold, so everything sharing a prefix shares the
/// work. The index stores no names, and the only way to learn what is in one is
/// to ask after every name a file could have — millions of them, all starting
/// `/map/mapstudio/`. Folding that part once turns an impractical sweep into an
/// unnoticeable one.
pub fn fold(from: u64, text: &str) -> u64 {
    let mut value = from;
    for ch in text.chars() {
        let ch = if ch == '\\' { '/' } else { ch.to_ascii_lowercase() };
        value = value.wrapping_mul(0x85).wrapping_add(ch as u64);
    }
    value
}

/// How many buckets the index has and where they start.
struct Shape {
    buckets: usize,
    at: usize,
}

/// Unlocks an index far enough to know its shape, and no further.
///
/// This is the whole cost of answering "is this file in this archive" — a
/// handful of blocks, against the several thousand a full index needs.
fn begin(bhd: &Path) -> Result<(Index, Shape)> {
    let what = bhd.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
    let fail = |detail: String| Error::Parse { what: what.clone(), detail };

    let locked = std::fs::read(bhd).map_err(|e| fail(e.to_string()))?;
    if locked.len() < LOCKED || locked.len() % LOCKED != 0 {
        return Err(fail(format!("{} bytes, which is not whole blocks", locked.len())));
    }

    let first = BigUint::from_bytes_be(&locked[..LOCKED]);
    let (modulus, exponent) = keys()
        .iter()
        .find(|(modulus, exponent)| {
            let opened = first.modpow(exponent, modulus).to_bytes_be();
            // The game wrote 255 bytes and the encryption put a zero in front,
            // so the right key gives exactly 255 back and they open with the
            // magic. A wrong one gives 2048 random bits.
            opened.len() == OPENED && opened.starts_with(b"BHD5")
        })
        .ok_or_else(|| fail("no key opens it".into()))?;

    let blocks = locked.len() / LOCKED;
    let mut index = Index {
        locked,
        plain: vec![0; blocks * OPENED],
        opened: vec![false; blocks],
        modulus: modulus.clone(),
        exponent: exponent.clone(),
    };

    let header = index.range(0, 36).ok_or_else(|| fail("no header".into()))?.to_vec();
    if header.first_chunk::<4>() != Some(b"BHD5") {
        return Err(fail("the key opened it into something that is not an index".into()));
    }
    let word = |at: usize| -> usize {
        u32::from_le_bytes(header[at..at + 4].try_into().unwrap_or_default()) as usize
    };
    let (size, buckets, at, salt) =
        (word(head::SIZE), word(head::BUCKETS), word(head::BUCKETS_AT), word(head::SALT_LEN));
    // The salt is the last thing in the header and the buckets follow it.
    if buckets == 0 || at < head::SALT_LEN + 4 + salt {
        return Err(fail(format!("{buckets} buckets said to be at {at}")));
    }
    if size > index.plain.len() {
        return Err(fail(format!("says it is {size} bytes and it is {}", index.plain.len())));
    }
    Ok((index, Shape { buckets, at }))
}

/// The records of the one bucket a name falls in: how many, and where.
fn bucket_for(index: &mut Index, shape: &Shape, name: u64) -> Option<(usize, usize)> {
    let which = (name % shape.buckets as u64) as usize;
    let row = index.range(shape.at + which * 8, 8)?;
    let count = u32::from_le_bytes(row[..4].try_into().ok()?) as usize;
    let at = u32::from_le_bytes(row[4..].try_into().ok()?) as usize;
    Some((count, at))
}

/// Reads one record, whatever it turns out to describe.
fn entry_of(row: &[u8]) -> (u64, Entry) {
    let long = |at: usize| u64::from_le_bytes(row[at..at + 8].try_into().unwrap_or_default());
    let word = |at: usize| u32::from_le_bytes(row[at..at + 4].try_into().unwrap_or_default()) as usize;
    let padded = word(record::PADDED);
    (
        long(record::HASH),
        Entry {
            at: long(record::AT),
            padded,
            // Zero means it was never padded, so the padded length is the real
            // one. Taking it literally truncates the file to nothing.
            size: match word(record::SIZE) {
                0 => padded,
                real => real,
            },
            key_at: long(record::AES_AT) as usize,
        },
    )
}

/// Whether an archive holds any of these paths, without building its index.
pub fn glance(bhd: &Path, names: &[u64]) -> bool {
    let Ok((mut index, shape)) = begin(bhd) else {
        return false;
    };
    names.iter().any(|&name| {
        let Some((count, at)) = bucket_for(&mut index, &shape, name) else {
            return false;
        };
        let Some(rows) = index.range(at, count.saturating_mul(record::WIDE)) else {
            return false;
        };
        rows.chunks_exact(record::WIDE).any(|row| entry_of(row).0 == name)
    })
}

impl Archive {
    /// Opens one `.bhd`, or says why not.
    ///
    /// Only the header, the bucket table and the records are decrypted. The
    /// keys are left locked until somebody asks for a file that needs one.
    pub fn open(bhd: &Path) -> Result<Self> {
        let what = bhd.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
        let fail = |detail: String| Error::Parse { what: what.clone(), detail };

        let (mut index, shape) = begin(bhd)?;
        let table = index
            .range(shape.at, shape.buckets.checked_mul(8).ok_or_else(|| fail("absurd bucket count".into()))?)
            .ok_or_else(|| fail("the bucket table runs past the file".into()))?
            .to_vec();

        let mut files = HashMap::new();
        for bucket in table.chunks_exact(8) {
            let count = u32::from_le_bytes(bucket[..4].try_into().unwrap_or_default()) as usize;
            let at = u32::from_le_bytes(bucket[4..].try_into().unwrap_or_default()) as usize;
            let Some(rows) = index.range(at, count.saturating_mul(record::WIDE)) else {
                continue;
            };
            for row in rows.chunks_exact(record::WIDE) {
                let (name, entry) = entry_of(row);
                files.insert(name, entry);
            }
        }

        Ok(Self {
            data: bhd.with_extension("bdt"),
            files,
            index: Mutex::new(index),
        })
    }

    /// How many files it holds.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Whether a path is in here, without reading it.
    pub fn has(&self, path: &str) -> bool {
        self.holds(hash(path))
    }

    /// The same for a path already folded, which a sweep has and a caller does
    /// not need to spell back out.
    pub fn holds(&self, name: u64) -> bool {
        self.files.contains_key(&name)
    }

    /// One file, decrypted, exactly as long as it should be.
    ///
    /// `None` covers a path that is not here, a `.bdt` that has been moved and
    /// a record pointing past the end of it. All three are the same to a
    /// caller, and none is worth failing a whole answer over.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.fetch(hash(path))
    }

    /// The same, by folded name.
    pub fn fetch(&self, name: u64) -> Option<Vec<u8>> {
        let entry = self.files.get(&name)?.clone();
        if entry.padded > BIGGEST {
            return None;
        }

        let mut body = vec![0u8; entry.padded];
        let mut data = std::fs::File::open(&self.data).ok()?;
        data.seek(SeekFrom::Start(entry.at)).ok()?;
        data.read_exact(&mut body).ok()?;

        if let Some((key, spans)) = self.key_for(&entry) {
            use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
            let cipher = aes::Aes128::new(GenericArray::from_slice(&key));
            for (from, upto) in spans {
                let from = from.min(body.len());
                let upto = upto.min(body.len());
                let Some(span) = body.get_mut(from..upto) else {
                    continue;
                };
                // Whole blocks only: the game leaves any tail in the clear.
                for block in span.chunks_exact_mut(16) {
                    cipher.decrypt_block(GenericArray::from_mut_slice(block));
                }
            }
        }

        body.truncate(entry.size);
        Some(body)
    }

    /// The key for one file, decrypted now that it is wanted.
    fn key_for(&self, entry: &Entry) -> Option<Keyed> {
        if entry.key_at == 0 {
            return None;
        }
        let mut index = self.index.lock();
        let head = index.range(entry.key_at, lock::FIRST)?.to_vec();
        let key: [u8; 16] = head[..lock::KEY].try_into().ok()?;
        let count =
            i32::from_le_bytes(head[lock::RANGES..lock::FIRST].try_into().ok()?).max(0) as usize;
        // A file can be listed with no range at all, which means it is plain.
        let listed = index.range(entry.key_at + lock::FIRST, count.checked_mul(lock::WIDE)?)?;
        let spans: Vec<(usize, usize)> = listed
            .chunks_exact(lock::WIDE)
            .filter_map(|span| {
                let from = i64::from_le_bytes(span[..8].try_into().ok()?);
                let upto = i64::from_le_bytes(span[8..].try_into().ok()?);
                (from >= 0 && upto > from).then_some((from as usize, upto as usize))
            })
            .collect();
        if spans.is_empty() {
            return None;
        }
        Some((key, spans))
    }
}

/// The archive in a game folder holding any of these files, if one does.
///
/// The archive names are not fixed and are not assumed: anything ending `.bhd`
/// beside a `.bdt` is tried, including the ones under `sd`. Each is only
/// glanced at, so the six that do not have what was asked for cost a handful of
/// blocks between them, and only the one that does is opened in full.
pub fn holding(game_dir: &Path, names: &[u64]) -> Option<Archive> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(game_dir)
        .max_depth(2)
        .into_iter()
        .flatten()
        .map(|one| one.into_path())
        .filter(|one| one.extension().is_some_and(|end| end.eq_ignore_ascii_case("bhd")))
        .filter(|one| one.with_extension("bdt").is_file())
        .collect();
    found.sort();
    found
        .iter()
        .find(|one| glance(one, names))
        .and_then(|one| Archive::open(one).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Option<PathBuf> {
        crate::testing::game_dir(crate::games::Game::EldenRing)
    }

    /// The fold, against the only thing that can judge it.
    ///
    /// Every record in an archive sits in the bucket its own hash names. Get
    /// the fold wrong, or the record stride, or the bucket table, and the
    /// agreement collapses at once rather than degrading — which is what makes
    /// this worth more than any single path checked by hand.
    #[test]
    fn every_record_sits_in_the_bucket_its_hash_names() {
        let Some(dir) = game() else {
            return;
        };
        // The smallest archive, because this runs unoptimised.
        let bhd = dir.join("Data3.bhd");
        if !bhd.is_file() {
            return;
        }
        let locked = std::fs::read(&bhd).expect("read");
        let archive = Archive::open(&bhd).expect("open Data3");
        assert!(archive.len() > 100, "only {} files", archive.len());

        // Re-read the header the long way to get at the buckets, since the
        // archive quite rightly does not expose them.
        let mut index = archive.index.lock();
        let header = index.range(0, 36).expect("header").to_vec();
        let buckets =
            u32::from_le_bytes(header[head::BUCKETS..head::BUCKETS + 4].try_into().unwrap()) as usize;
        let at_buckets =
            u32::from_le_bytes(header[head::BUCKETS_AT..head::BUCKETS_AT + 4].try_into().unwrap())
                as usize;
        let table = index.range(at_buckets, buckets * 8).expect("buckets").to_vec();

        let mut seen = 0usize;
        for bucket in 0..buckets {
            let count =
                u32::from_le_bytes(table[bucket * 8..bucket * 8 + 4].try_into().unwrap()) as usize;
            let at =
                u32::from_le_bytes(table[bucket * 8 + 4..bucket * 8 + 8].try_into().unwrap()) as usize;
            let rows = index.range(at, count * record::WIDE).expect("records").to_vec();
            for row in rows.chunks_exact(record::WIDE) {
                let name = u64::from_le_bytes(row[..8].try_into().unwrap());
                assert_eq!(
                    name as usize % buckets,
                    bucket,
                    "a record in bucket {bucket} hashes to {}",
                    name as usize % buckets
                );
                seen += 1;
            }
        }
        assert_eq!(seen, archive.len(), "the index lost records on the way in");
        assert_eq!(locked.len() % LOCKED, 0);
    }

    /// A path a player would name, turned into bytes that are what they claim.
    ///
    /// The map is in `Data2`, comes out AES-encrypted, and what is underneath
    /// has to be a DCX that really expands to a real map. Nothing short of the
    /// whole chain being right produces that.
    #[test]
    fn a_packed_map_comes_out_a_map() {
        let Some(dir) = game() else {
            return;
        };
        let bhd = dir.join("Data2.bhd");
        if !bhd.is_file() {
            return;
        }
        let archive = Archive::open(&bhd).expect("open Data2");

        let want = "/map/mapstudio/m10_00_00_00.msb.dcx";
        assert!(archive.has(want), "the index does not have {want}");
        assert!(!archive.has("/map/mapstudio/m99_99_99_99.msb.dcx"));

        let packed = archive.read(want).expect("read the map");
        assert!(
            crate::formats::dcx::wraps(&packed),
            "came out {:?} rather than a DCX",
            packed.get(..4).map(String::from_utf8_lossy)
        );

        // And it is a map, not merely something shaped like a wrapper. The
        // packed ones are Kraken where the loose ones are not, so this needs
        // the game's own library and will not run without it.
        crate::formats::oodle::register(&dir);
        if !crate::formats::oodle::available() {
            return;
        }
        let plain = crate::formats::dcx::expand(&packed, "m10_00_00_00").expect("expand the map");
        assert_eq!(plain.first_chunk::<4>(), Some(b"MSB "), "not a map inside");

        // The whole point of the chain: real enemies come out of the far end,
        // through the same call the bestiary will make.
        let placed = crate::formats::msb::enemies_in(&packed, "m10_00_00_00.msb.dcx");
        assert!(placed.len() > 10, "only {} enemies in Stormveil", placed.len());
    }

    /// Nothing here may panic on a file that is not what it says.
    #[test]
    fn rubbish_is_refused_rather_than_followed() {
        let dir = std::env::temp_dir().join("roundtable-bhd5-test");
        let _ = std::fs::create_dir_all(&dir);

        let empty = dir.join("empty.bhd");
        std::fs::write(&empty, []).expect("write");
        assert!(Archive::open(&empty).is_err());

        // Whole blocks, so it gets as far as trying every key, and none opens it.
        let noise = dir.join("noise.bhd");
        std::fs::write(&noise, vec![0x5au8; LOCKED * 3]).expect("write");
        let refused = Archive::open(&noise).err().map(|why| why.to_string()).unwrap_or_default();
        assert!(refused.contains("no key opens it"), "{refused}");

        let odd = dir.join("odd.bhd");
        std::fs::write(&odd, vec![0u8; LOCKED + 7]).expect("write");
        assert!(Archive::open(&odd).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fold itself, on the cases that are easy to get wrong.
    #[test]
    fn the_fold_normalises_before_it_folds() {
        assert_eq!(hash("/map/mapstudio/m10_00_00_00.msb.dcx"), hash("MAP\\MapStudio\\M10_00_00_00.MSB.DCX"));
        assert_eq!(hash("/regulation.bin"), hash("regulation.bin"));
        assert_ne!(hash("/a"), hash("/b"));
        // An empty path folds to the leading separator and nothing else.
        assert_eq!(hash(""), u64::from(b'/'));
    }
}
