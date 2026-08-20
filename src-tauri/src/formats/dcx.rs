//! The wrapper FromSoftware puts around almost every file it ships.
//!
//! A DCX says how big the thing inside will be, how big it is packed, and which
//! compressor was used. `regulation.bin` is ZSTD; the message archives and most
//! everything else are Kraken, which needs the game's own library — see
//! [`super::oodle`].

use crate::error::{Error, Result};

/// Refuse anything claiming to expand past this. A wrong header should cost a
/// message, not the machine's memory.
const MOST: usize = 256 * 1024 * 1024;

/// Whether these bytes are a DCX at all.
pub fn wraps(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"DCX\0")
}

/// Unwraps one, whatever it was packed with.
///
/// `what` names the file for the error, since by the time this fails the caller
/// is several layers away from the thing the player chose.
pub fn expand(plain: &[u8], what: &str) -> Result<Vec<u8>> {
    let fail = |detail: String| Error::Parse {
        what: what.to_string(),
        detail,
    };

    let big =
        |at: usize| -> Option<u32> { Some(u32::from_be_bytes(plain.get(at..at + 4)?.try_into().ok()?)) };
    if !wraps(plain) {
        return Err(fail(format!(
            "starts {:?} rather than DCX",
            plain.get(..4).map(String::from_utf8_lossy)
        )));
    }

    // The header points at its own parts rather than fixing them, so follow it.
    let sizes = big(0x08).ok_or_else(|| fail("truncated header".into()))? as usize;
    let params = big(0x10).ok_or_else(|| fail("truncated header".into()))? as usize;
    if plain.get(sizes..sizes + 4) != Some(b"DCS\0") {
        return Err(fail("no size block where the header said".into()));
    }
    if plain.get(params..params + 4) != Some(b"DCA\0") {
        return Err(fail("no parameter block where the header said".into()));
    }

    let expanded = big(sizes + 4).ok_or_else(|| fail("no expanded size".into()))? as usize;
    let packed = big(sizes + 8).ok_or_else(|| fail("no packed size".into()))? as usize;
    if expanded > MOST {
        return Err(fail(format!("claims to expand to {expanded} bytes")));
    }

    let skip = big(params + 4).ok_or_else(|| fail("no parameter length".into()))? as usize;
    let from = params + skip;
    let payload = plain
        .get(from..from + packed)
        .ok_or_else(|| fail("the compressed part runs past the file".into()))?;

    let method = plain.get(0x28..0x2c).unwrap_or(b"????");
    let out = match method {
        b"ZSTD" => zstd::stream::decode_all(payload).map_err(|e| fail(e.to_string()))?,
        b"KRAK" => super::oodle::decompress(payload, expanded)?,
        // What a file repacked by a translation mod comes back as.
        b"DFLT" => {
            use std::io::Read;
            let mut out = Vec::with_capacity(expanded);
            flate2::read::ZlibDecoder::new(payload)
                .read_to_end(&mut out)
                .map_err(|e| fail(e.to_string()))?;
            out
        }
        other => {
            return Err(fail(format!(
                "compressed with {:?}, which this does not read",
                String::from_utf8_lossy(other)
            )))
        }
    };
    if out.len() != expanded {
        return Err(fail(format!(
            "expanded to {} bytes where the header promised {expanded}",
            out.len()
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn something_that_is_not_a_dcx_is_said_so() {
        assert!(!wraps(b"BND4"));
        let refused = expand(b"BND4 and then some", "test");
        assert!(format!("{refused:?}").contains("rather than DCX"), "{refused:?}");
    }

    #[test]
    fn a_header_pointing_nowhere_is_refused_rather_than_followed() {
        // The failure that matters: a truncated or hostile header sending the
        // reader off the end of the buffer.
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(b"DCX\0");
        bytes[0x08..0x0c].copy_from_slice(&0xffff_u32.to_be_bytes());
        assert!(expand(&bytes, "test").is_err());
    }
}
