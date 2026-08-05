//! Reader/writer for ELDEN RING `.sl2` save containers (and the `.co2` variant that
//! Seamless Co-op writes, which is byte-identical and only differs by extension).
//!
//! Layout, verified against two independent implementations
//! (BenGrn/EldenRingSaveCopier and ClayAmore/ER-Save-Editor):
//!
//! ```text
//! 0x0000000  BND4 header
//! 0x0000300  slot 0 MD5 (0x10)      \
//! 0x0000310  slot 0 data (0x280000) | repeated 10x, stride 0x280010
//! ...                               /
//! 0x19003A0  USER_DATA_10 MD5 (0x10)
//! 0x19003B0  USER_DATA_10 data (0x60000)
//!   +0x00004   SteamID64, u64 LE          -> absolute 0x19003B4
//!   +0x01954   per-slot active flags, 10x u8 -> absolute 0x1901D04
//!   +0x0195E   per-slot summaries, 10x 0x24C -> absolute 0x1901D0E
//! 0x19603B0  USER_DATA_11 MD5 (0x10)
//! 0x19603C0  USER_DATA_11 data
//! ```
//!
//! Every 0x10 checksum is the MD5 of the block that follows it. The game refuses a
//! save whose checksums do not match, so any write path must recompute them.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const SLOT_COUNT: usize = 10;

const BND4_MAGIC: &[u8; 4] = b"BND4";

const SLOT_DATA_START: usize = 0x0000310;
const SLOT_DATA_LEN: usize = 0x0280000;
const SLOT_STRIDE: usize = SLOT_DATA_LEN + 0x10;

const USER_DATA_10_CHECKSUM: usize = 0x19003A0;
const USER_DATA_10_START: usize = 0x19003B0;
const USER_DATA_10_LEN: usize = 0x0060000;

const STEAM_ID_OFFSET: usize = 0x19003B4;
const ACTIVE_FLAGS_OFFSET: usize = 0x1901D04;
const SUMMARY_OFFSET: usize = 0x1901D0E;
const SUMMARY_STRIDE: usize = 0x24C;

/// Name field is 0x22 bytes of UTF-16LE, i.e. 16 characters plus a terminator.
const SUMMARY_NAME_LEN: usize = 0x22;
const SUMMARY_LEVEL_OFFSET: usize = 0x22;
const SUMMARY_SECONDS_OFFSET: usize = 0x26;

/// Smallest file we are willing to treat as a full ELDEN RING container.
const MIN_SAVE_LEN: usize = USER_DATA_10_START + USER_DATA_10_LEN;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotSummary {
    pub index: usize,
    pub active: bool,
    pub name: String,
    pub level: u32,
    pub seconds_played: u32,
    /// SteamID64 found inside the slot payload, when the slot carries one.
    pub steam_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSummary {
    pub steam_id: u64,
    pub slots: Vec<SlotSummary>,
    pub byte_len: usize,
    /// True when every stored checksum matches the data it covers.
    pub checksums_valid: bool,
}

/// An in-memory ELDEN RING save container.
#[derive(Clone)]
pub struct SaveFile {
    bytes: Vec<u8>,
}

/// Printing 26 MB of save data would be useless, so only the shape is shown.
impl std::fmt::Debug for SaveFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveFile")
            .field("bytes", &self.bytes.len())
            .field("steam_id", &self.steam_id())
            .finish()
    }
}

impl SaveFile {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < 4 || &bytes[0..4] != BND4_MAGIC {
            return Err(Error::NotASave);
        }
        if bytes.len() < MIN_SAVE_LEN {
            return Err(Error::SaveTruncated {
                needed: MIN_SAVE_LEN,
                actual: bytes.len(),
            });
        }
        Ok(SaveFile { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn slot_checksum_range(index: usize) -> (usize, usize) {
        let checksum = SLOT_DATA_START - 0x10 + index * SLOT_STRIDE;
        (checksum, checksum + 0x10)
    }

    fn slot_data_range(index: usize) -> (usize, usize) {
        let start = SLOT_DATA_START + index * SLOT_STRIDE;
        (start, start + SLOT_DATA_LEN)
    }

    fn check_slot(index: usize) -> Result<()> {
        if index >= SLOT_COUNT {
            return Err(Error::SlotOutOfRange(index));
        }
        Ok(())
    }

    /// The account-wide SteamID64 stored in USER_DATA_10.
    pub fn steam_id(&self) -> u64 {
        u64::from_le_bytes(
            self.bytes[STEAM_ID_OFFSET..STEAM_ID_OFFSET + 8]
                .try_into()
                .expect("bounds checked in from_bytes"),
        )
    }

    pub fn set_steam_id(&mut self, steam_id: u64) {
        self.bytes[STEAM_ID_OFFSET..STEAM_ID_OFFSET + 8].copy_from_slice(&steam_id.to_le_bytes());
    }

    pub fn is_slot_active(&self, index: usize) -> Result<bool> {
        Self::check_slot(index)?;
        Ok(self.bytes[ACTIVE_FLAGS_OFFSET + index] == 1)
    }

    pub fn set_slot_active(&mut self, index: usize, active: bool) -> Result<()> {
        Self::check_slot(index)?;
        self.bytes[ACTIVE_FLAGS_OFFSET + index] = u8::from(active);
        Ok(())
    }

    fn summary_range(index: usize) -> (usize, usize) {
        let start = SUMMARY_OFFSET + index * SUMMARY_STRIDE;
        (start, start + SUMMARY_STRIDE)
    }

    pub fn slot_summary(&self, index: usize) -> Result<SlotSummary> {
        Self::check_slot(index)?;
        let (start, _) = Self::summary_range(index);

        let name_bytes = &self.bytes[start..start + SUMMARY_NAME_LEN];
        let name = decode_utf16le(name_bytes);

        let level = u32::from_le_bytes(
            self.bytes[start + SUMMARY_LEVEL_OFFSET..start + SUMMARY_LEVEL_OFFSET + 4]
                .try_into()
                .expect("summary block is 0x24C bytes"),
        );
        let seconds_played = u32::from_le_bytes(
            self.bytes[start + SUMMARY_SECONDS_OFFSET..start + SUMMARY_SECONDS_OFFSET + 4]
                .try_into()
                .expect("summary block is 0x24C bytes"),
        );

        let active = self.is_slot_active(index)?;
        let account_id = self.steam_id();
        let steam_id = if active && self.slot_contains_id(index, account_id)? {
            Some(account_id)
        } else {
            None
        };

        Ok(SlotSummary {
            index,
            active,
            name,
            level,
            seconds_played,
            steam_id,
        })
    }

    pub fn summary(&self) -> Result<SaveSummary> {
        let slots = (0..SLOT_COUNT)
            .map(|i| self.slot_summary(i))
            .collect::<Result<Vec<_>>>()?;

        Ok(SaveSummary {
            steam_id: self.steam_id(),
            slots,
            byte_len: self.bytes.len(),
            checksums_valid: self.verify_checksums(),
        })
    }

    fn slot_contains_id(&self, index: usize, id: u64) -> Result<bool> {
        Self::check_slot(index)?;
        let (start, end) = Self::slot_data_range(index);
        let needle = id.to_le_bytes();
        Ok(!find_all(&self.bytes[start..end], &needle).is_empty())
    }

    pub fn slot_bytes(&self, index: usize) -> Result<Vec<u8>> {
        Self::check_slot(index)?;
        let (start, end) = Self::slot_data_range(index);
        Ok(self.bytes[start..end].to_vec())
    }

    pub fn summary_bytes(&self, index: usize) -> Result<Vec<u8>> {
        Self::check_slot(index)?;
        let (start, end) = Self::summary_range(index);
        Ok(self.bytes[start..end].to_vec())
    }

    /// Copies one character from `source` into `self`, rebinding it to this file's
    /// account so the game accepts it.
    ///
    /// The account id is embedded in several places inside the slot payload, so every
    /// occurrence of the source id is rewritten rather than a single known offset.
    pub fn import_slot(
        &mut self,
        source: &SaveFile,
        from_slot: usize,
        to_slot: usize,
    ) -> Result<()> {
        Self::check_slot(from_slot)?;
        Self::check_slot(to_slot)?;

        let source_id = source.steam_id();
        let target_id = self.steam_id();

        let mut slot = source.slot_bytes(from_slot)?;
        if source_id != target_id {
            replace_all(&mut slot, &source_id.to_le_bytes(), &target_id.to_le_bytes());
        }

        let (dst_start, dst_end) = Self::slot_data_range(to_slot);
        self.bytes[dst_start..dst_end].copy_from_slice(&slot);

        let mut summary = source.summary_bytes(from_slot)?;
        if source_id != target_id {
            replace_all(
                &mut summary,
                &source_id.to_le_bytes(),
                &target_id.to_le_bytes(),
            );
        }
        let (sum_start, sum_end) = Self::summary_range(to_slot);
        self.bytes[sum_start..sum_end].copy_from_slice(&summary);

        let active = source.is_slot_active(from_slot)?;
        self.set_slot_active(to_slot, active)?;

        self.recompute_checksums();
        Ok(())
    }

    /// Marks a slot empty. The payload is zeroed so a later import cannot resurrect
    /// fragments of the old character.
    pub fn clear_slot(&mut self, index: usize) -> Result<()> {
        Self::check_slot(index)?;
        let (start, end) = Self::slot_data_range(index);
        self.bytes[start..end].fill(0);
        let (sum_start, sum_end) = Self::summary_range(index);
        self.bytes[sum_start..sum_end].fill(0);
        self.set_slot_active(index, false)?;
        self.recompute_checksums();
        Ok(())
    }

    /// Rebinds the whole container to a different Steam account.
    pub fn rebind_to_account(&mut self, new_id: u64) {
        let old_id = self.steam_id();
        if old_id == new_id {
            return;
        }
        let from = old_id.to_le_bytes();
        let to = new_id.to_le_bytes();

        for index in 0..SLOT_COUNT {
            let (start, end) = Self::slot_data_range(index);
            replace_all(&mut self.bytes[start..end], &from, &to);
        }
        let ud10_end = USER_DATA_10_START + USER_DATA_10_LEN;
        replace_all(&mut self.bytes[USER_DATA_10_START..ud10_end], &from, &to);

        self.set_steam_id(new_id);
        self.recompute_checksums();
    }

    pub fn recompute_checksums(&mut self) {
        for index in 0..SLOT_COUNT {
            let (data_start, data_end) = Self::slot_data_range(index);
            let digest = md5_of(&self.bytes[data_start..data_end]);
            let (sum_start, sum_end) = Self::slot_checksum_range(index);
            self.bytes[sum_start..sum_end].copy_from_slice(&digest);
        }

        let ud10_end = USER_DATA_10_START + USER_DATA_10_LEN;
        let digest = md5_of(&self.bytes[USER_DATA_10_START..ud10_end]);
        self.bytes[USER_DATA_10_CHECKSUM..USER_DATA_10_CHECKSUM + 0x10].copy_from_slice(&digest);
    }

    pub fn verify_checksums(&self) -> bool {
        for index in 0..SLOT_COUNT {
            let (data_start, data_end) = Self::slot_data_range(index);
            let (sum_start, sum_end) = Self::slot_checksum_range(index);
            if md5_of(&self.bytes[data_start..data_end]) != self.bytes[sum_start..sum_end] {
                return false;
            }
        }
        let ud10_end = USER_DATA_10_START + USER_DATA_10_LEN;
        md5_of(&self.bytes[USER_DATA_10_START..ud10_end])
            == self.bytes[USER_DATA_10_CHECKSUM..USER_DATA_10_CHECKSUM + 0x10]
    }
}

fn md5_of(data: &[u8]) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Start index of every non-overlapping occurrence of `needle`.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let mut cursor = 0usize;
    while cursor + needle.len() <= haystack.len() {
        if &haystack[cursor..cursor + needle.len()] == needle {
            hits.push(cursor);
            cursor += needle.len();
        } else {
            cursor += 1;
        }
    }
    hits
}

/// In-place replacement of equal-length byte patterns.
fn replace_all(buffer: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    debug_assert_eq!(from.len(), to.len());
    if from == to || from.is_empty() {
        return 0;
    }
    let positions = find_all(buffer, from);
    for at in &positions {
        buffer[*at..*at + to.len()].copy_from_slice(to);
    }
    positions.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a container with the real geometry but zeroed payloads.
    fn blank_save(steam_id: u64) -> SaveFile {
        let total = USER_DATA_10_START + USER_DATA_10_LEN;
        let mut bytes = vec![0u8; total];
        bytes[0..4].copy_from_slice(BND4_MAGIC);
        let mut save = SaveFile::from_bytes(bytes).expect("geometry is valid");
        save.set_steam_id(steam_id);
        save.recompute_checksums();
        save
    }

    #[test]
    fn rejects_non_bnd4() {
        let err = SaveFile::from_bytes(vec![0u8; MIN_SAVE_LEN]).unwrap_err();
        assert!(matches!(err, Error::NotASave));
    }

    #[test]
    fn rejects_truncated_container() {
        let mut bytes = vec![0u8; 1024];
        bytes[0..4].copy_from_slice(BND4_MAGIC);
        let err = SaveFile::from_bytes(bytes).unwrap_err();
        assert!(matches!(err, Error::SaveTruncated { .. }));
    }

    #[test]
    fn slot_geometry_matches_known_offsets() {
        assert_eq!(SaveFile::slot_data_range(0).0, 0x310);
        assert_eq!(SaveFile::slot_checksum_range(0).0, 0x300);
        // Second slot sits one full stride later, matching the reference tools'
        // `0x310 + i*0x10 + i*0x280000`.
        assert_eq!(SaveFile::slot_data_range(1).0, 0x310 + 0x10 + 0x280000);
        // The ten slots end exactly where USER_DATA_10's checksum begins.
        assert_eq!(SaveFile::slot_data_range(9).1, USER_DATA_10_CHECKSUM);
    }

    #[test]
    fn steam_id_round_trips() {
        let mut save = blank_save(76561198000000001);
        assert_eq!(save.steam_id(), 76561198000000001);
        save.set_steam_id(76561198999999999);
        assert_eq!(save.steam_id(), 76561198999999999);
    }

    #[test]
    fn checksums_validate_and_detect_tampering() {
        let mut save = blank_save(7656119800000000);
        assert!(save.verify_checksums());

        let (start, _) = SaveFile::slot_data_range(3);
        save.bytes[start] ^= 0xFF;
        assert!(!save.verify_checksums());

        save.recompute_checksums();
        assert!(save.verify_checksums());
    }

    #[test]
    fn slot_active_flags_are_independent() {
        let mut save = blank_save(1);
        save.set_slot_active(4, true).unwrap();
        assert!(save.is_slot_active(4).unwrap());
        assert!(!save.is_slot_active(3).unwrap());
        assert!(!save.is_slot_active(5).unwrap());
    }

    #[test]
    fn out_of_range_slot_is_rejected() {
        let save = blank_save(1);
        assert!(matches!(
            save.slot_summary(10).unwrap_err(),
            Error::SlotOutOfRange(10)
        ));
    }

    #[test]
    fn import_rewrites_every_account_id_occurrence() {
        let source_id = 76561198000000001u64;
        let target_id = 76561198123456789u64;

        let mut source = blank_save(source_id);
        // Scatter the source account id through the slot the way the game does.
        let (start, _) = SaveFile::slot_data_range(0);
        for step in 0..5 {
            let at = start + step * 4096;
            source.bytes[at..at + 8].copy_from_slice(&source_id.to_le_bytes());
        }
        source.set_slot_active(0, true).unwrap();
        source.recompute_checksums();

        let mut target = blank_save(target_id);
        target.import_slot(&source, 0, 2).unwrap();

        let imported = target.slot_bytes(2).unwrap();
        assert_eq!(
            find_all(&imported, &source_id.to_le_bytes()).len(),
            0,
            "no trace of the source account may remain"
        );
        assert_eq!(find_all(&imported, &target_id.to_le_bytes()).len(), 5);
        assert!(target.is_slot_active(2).unwrap());
        assert!(target.verify_checksums());
        // Importing must not disturb the destination's own account id.
        assert_eq!(target.steam_id(), target_id);
    }

    #[test]
    fn import_into_same_account_is_a_plain_copy() {
        let id = 76561198000000077u64;
        let mut source = blank_save(id);
        let (start, _) = SaveFile::slot_data_range(1);
        source.bytes[start..start + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        source.set_slot_active(1, true).unwrap();
        source.recompute_checksums();

        let mut target = blank_save(id);
        target.import_slot(&source, 1, 1).unwrap();
        assert_eq!(&target.slot_bytes(1).unwrap()[0..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(target.verify_checksums());
    }

    #[test]
    fn clearing_a_slot_leaves_no_residue() {
        let id = 5u64;
        let mut save = blank_save(id);
        let (start, _) = SaveFile::slot_data_range(6);
        save.bytes[start..start + 8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        save.set_slot_active(6, true).unwrap();
        save.recompute_checksums();

        save.clear_slot(6).unwrap();
        assert!(save.slot_bytes(6).unwrap().iter().all(|&b| b == 0));
        assert!(!save.is_slot_active(6).unwrap());
        assert!(save.verify_checksums());
    }

    #[test]
    fn rebinding_moves_the_whole_container() {
        let old = 76561198000000010u64;
        let new = 76561198000000020u64;
        let mut save = blank_save(old);
        let (start, _) = SaveFile::slot_data_range(0);
        save.bytes[start..start + 8].copy_from_slice(&old.to_le_bytes());
        save.recompute_checksums();

        save.rebind_to_account(new);
        assert_eq!(save.steam_id(), new);
        assert_eq!(&save.slot_bytes(0).unwrap()[0..8], &new.to_le_bytes());
        assert!(save.verify_checksums());
    }

    #[test]
    fn utf16_names_stop_at_the_terminator() {
        let mut raw = vec![0u8; SUMMARY_NAME_LEN];
        for (i, unit) in "Tarnished".encode_utf16().enumerate() {
            raw[i * 2..i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_utf16le(&raw), "Tarnished");
    }

    #[test]
    fn find_all_does_not_overlap_matches() {
        let haystack = [1u8, 1, 1, 1];
        assert_eq!(find_all(&haystack, &[1, 1]), vec![0, 2]);
    }

    #[test]
    fn replace_all_reports_the_replacement_count() {
        let mut buf = vec![0xAAu8, 0xBB, 0xAA, 0xBB, 0x00];
        assert_eq!(replace_all(&mut buf, &[0xAA, 0xBB], &[0xCC, 0xDD]), 2);
        assert_eq!(buf, vec![0xCC, 0xDD, 0xCC, 0xDD, 0x00]);
    }
}
