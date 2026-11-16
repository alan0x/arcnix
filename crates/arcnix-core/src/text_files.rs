//! Bounded UTF-8 editing with optimistic content checks and preserved formatting.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
pub const LIMIT: usize = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
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
        ensure!(
            !content.contains(['\0', '\r']),
            "Text contains unsupported control characters"
        );
        let text = if self.crlf {
            content.replace('\n', "\r\n")
        } else {
            content.to_owned()
        };
        let mut bytes = if self.bom {
            vec![0xef, 0xbb, 0xbf]
        } else {
            vec![]
        };
        bytes.extend_from_slice(text.as_bytes());
        ensure!(bytes.len() <= LIMIT, "Edited file exceeds 1 MiB");
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
