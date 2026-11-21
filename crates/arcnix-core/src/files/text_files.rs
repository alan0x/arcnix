//! Bounded UTF-8 editing with optimistic content checks and preserved formatting.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
pub const LIMIT: usize = 1024 * 1024;
pub const RANGE_BYTES: usize = 128 * 1024;
// Reserve room for UTF-8 and CRLF alignment on both sides of a nominal page.
pub const PAGE_BYTES: u64 = RANGE_BYTES as u64 - 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TextSnapshot {
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(serde::Serialize)]
pub struct TextRange {
    pub text: String,
    pub next: u64,
    pub breaks: usize,
    pub digest: [u8; 32],
}

// Earlier bytes in a sample may start mid-scalar. Only its final scalar can
// be incomplete because a writer split a UTF-8 sequence across two writes.
pub(crate) fn incomplete_tail(bytes: &[u8]) -> usize {
    let Some(start) = bytes.iter().rposition(|b| b & 0xc0 != 0x80) else {
        return 0;
    };
    let last = &bytes[start..];
    if std::str::from_utf8(last).is_err_and(|error| error.error_len().is_none()) {
        last.len()
    } else {
        0
    }
}

// Align the same nominal boundary identically when reading either neighbour.
fn boundary(bytes: &[u8], mut at: usize) -> usize {
    if at == bytes.len() {
        return at;
    }
    if bytes[at] == b'\n' && at > 0 && bytes[at - 1] == b'\r' {
        return at - 1;
    }
    let earliest = at.saturating_sub(3);
    while at > earliest && bytes[at] & 0xc0 == 0x80 {
        at -= 1;
    }
    at
}

pub(crate) fn decode_page(
    bytes: &[u8],
    buffer_start: u64,
    offset: u64,
    next: u64,
) -> Result<TextRange> {
    let start = boundary(bytes, (offset - buffer_start) as usize);
    let end = boundary(bytes, (next - buffer_start) as usize);
    let payload = &bytes[start..end];
    let text = std::str::from_utf8(payload)?;
    ensure!(!text.contains('\0'), "Not UTF-8 text: binary data");
    let text = if offset == 0 {
        text.strip_prefix('\u{feff}').unwrap_or(text)
    } else {
        text
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(TextRange {
        breaks: text.bytes().filter(|&byte| byte == b'\n').count(),
        text,
        next,
        digest: Sha256::digest(payload).into(),
    })
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Version {
    pub hash: [u8; 32],
    pub size: u64,
    pub modified: Option<u64>,
    pub mode: u32,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct Document {
    pub path: String,
    pub content: String,
    pub version: Version,
    pub bom: bool,
    pub crlf: bool,
}
impl Document {
    pub(crate) fn decode(
        path: String,
        bytes: Vec<u8>,
        modified: Option<u64>,
        mode: u32,
    ) -> Result<Self> {
        ensure!(
            bytes.len() <= LIMIT && !bytes.contains(&0),
            "Only UTF-8 text files up to 1 MiB can be edited"
        );
        let hash = Sha256::digest(&bytes).into();
        let size = bytes.len() as u64;
        let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let text = String::from_utf8(if bom { bytes[3..].to_vec() } else { bytes })?;
        let crlf = text.contains("\r\n");
        let normalized = text.replace("\r\n", "\n");
        ensure!(
            !normalized.contains('\r')
                && (!crlf || text.matches('\n').count() == text.matches("\r\n").count()),
            "Mixed or legacy line endings are not supported by the built-in editor"
        );
        Ok(Self {
            path,
            content: normalized,
            version: Version {
                hash,
                size,
                modified,
                mode,
                uid: None,
                gid: None,
            },
            bom,
            crlf,
        })
    }
    pub fn encode(&self, content: &str) -> Result<Vec<u8>> {
        encode(content, self.bom, self.crlf)
    }
}

pub fn encode(content: &str, bom: bool, crlf: bool) -> Result<Vec<u8>> {
    ensure!(
        !content.contains(['\0', '\r']),
        "Text contains unsupported control characters"
    );
    let text = if crlf {
        content.replace('\n', "\r\n")
    } else {
        content.to_owned()
    };
    let mut bytes = if bom { vec![0xef, 0xbb, 0xbf] } else { vec![] };
    bytes.extend_from_slice(text.as_bytes());
    ensure!(bytes.len() <= LIMIT, "Edited file exceeds 1 MiB");
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unfinished_utf8_at_eof_is_deferred() {
        let record = "日志🚦".as_bytes();
        for cut in 0..=record.len() {
            let expected = cut
                - std::str::from_utf8(&record[..cut]).map_or_else(|e| e.valid_up_to(), str::len);
            assert_eq!(incomplete_tail(&record[..cut]), expected);
        }
        assert_eq!(incomplete_tail(&[0x80, b'a', 0xf0, 0x9f]), 2);
        assert_eq!(incomplete_tail(&[0xff]), 0);
    }
    #[test]
    fn ranges_preserve_raw_offsets_and_normalize_split_crlf_and_utf8() {
        let bytes = "\u{feff}a\r\n读🚦b".as_bytes();
        let size = bytes.len() as u64;
        for split in 3..bytes.len() {
            let left = decode_page(&bytes[..split + 1], 0, 0, split as u64).unwrap();
            let base = split.saturating_sub(3);
            let right = decode_page(&bytes[base..], base as u64, split as u64, size).unwrap();
            assert_eq!(left.breaks + right.breaks, 1);
            assert_eq!(left.text + &right.text, "a\n读🚦b");
        }
        assert!(decode_page(&[0xf0, 0x9f], 0, 0, 2).is_err());
        assert!(decode_page(b"abc\0", 0, 0, 4).is_err());
    }
    #[test]
    fn preserves_bom_and_crlf_and_rejects_binary_and_mixed_endings() {
        let original = b"\xef\xbb\xbfhello\r\nworld\r\n".to_vec();
        let doc = Document::decode("/file".into(), original.clone(), None, 0o644).unwrap();
        assert_eq!(doc.content, "hello\nworld\n");
        assert_eq!(doc.encode(&doc.content).unwrap(), original);
        assert!(Document::decode("/file".into(), b"a\r\nb\n".to_vec(), None, 0o644).is_err());
        assert!(Document::decode("/file".into(), vec![0], None, 0o644).is_err());
    }
}
