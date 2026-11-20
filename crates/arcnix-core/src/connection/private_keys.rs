//! Offline inspection of SSH private key files before authentication.
//!
//! This only answers whether a passphrase prompt is needed. It looks at the
//! envelope of the file (PEM labels, PEM headers and the OpenSSH cipher
//! name) and never decodes, decrypts or interprets key material. The file
//! contents are wiped from memory after inspection. Anything it does not
//! recognise is reported as [`KeyProtection::Unknown`], so callers keep
//! asking for a passphrase.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use zeroize::Zeroizing;

/// Whether a private key file needs a passphrase before it can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyProtection {
    /// The file is a recognised private key format stored without encryption.
    Unencrypted,
    /// The file is a recognised private key format protected by a passphrase.
    Encrypted,
    /// The file could not be read, is too large or is not a recognised format.
    Unknown,
}

/// Private key files are a few KiB; anything larger is not inspected.
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;
const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";
/// Base64 characters decoded from an OpenSSH body: 66 bytes cover the magic,
/// the length prefix and every cipher name OpenSSH uses, and stop well before
/// the public and private key sections.
const OPENSSH_HEADER_CHARS: usize = 88;

/// Reports whether the private key at `path` is protected by a passphrase.
///
/// Read errors, non-regular files, oversized files and unrecognised formats
/// return [`KeyProtection::Unknown`] instead of an error, because the
/// caller's safe response is the same in every case: ask for a passphrase as
/// before. Only regular files are opened, so a FIFO or device path cannot
/// block the caller.
pub fn key_protection(path: &Path) -> KeyProtection {
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return KeyProtection::Unknown;
    }
    let Ok(file) = File::open(path) else {
        return KeyProtection::Unknown;
    };
    let mut bytes = Zeroizing::new(Vec::new());
    if file
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_KEY_FILE_BYTES
    {
        return KeyProtection::Unknown;
    }
    match std::str::from_utf8(&bytes) {
        Ok(text) => classify(text),
        Err(_) => KeyProtection::Unknown,
    }
}

fn classify(text: &str) -> KeyProtection {
    let mut lines = text.lines().map(str::trim);
    let Some(label) = lines.find_map(|line| {
        line.strip_prefix("-----BEGIN ")
            .and_then(|rest| rest.strip_suffix("-----"))
    }) else {
        return KeyProtection::Unknown;
    };
    let end = format!("-----END {label}-----");
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines {
        if line == end {
            closed = true;
            break;
        }
        body.push(line);
    }
    if !closed {
        return KeyProtection::Unknown;
    }
    match label {
        "OPENSSH PRIVATE KEY" => openssh(&body),
        "RSA PRIVATE KEY" | "EC PRIVATE KEY" | "DSA PRIVATE KEY" => legacy_pem(&body),
        "PRIVATE KEY" => KeyProtection::Unencrypted,
        "ENCRYPTED PRIVATE KEY" => KeyProtection::Encrypted,
        _ => KeyProtection::Unknown,
    }
}

/// Traditional OpenSSL PEM marks encryption with RFC 1421 headers.
fn legacy_pem(body: &[&str]) -> KeyProtection {
    let encrypted = body.iter().take_while(|line| !line.is_empty()).any(|line| {
        line.strip_prefix("Proc-Type:")
            .is_some_and(|value| value.contains("ENCRYPTED"))
            || line.starts_with("DEK-Info:")
    });
    if encrypted {
        KeyProtection::Encrypted
    } else {
        KeyProtection::Unencrypted
    }
}

/// The OpenSSH format stores the cipher name in clear text after the magic.
/// Only that header is decoded, never the key sections that follow it.
fn openssh(body: &[&str]) -> KeyProtection {
    let header: String = body
        .iter()
        .flat_map(|line| line.chars())
        .take(OPENSSH_HEADER_CHARS)
        .collect();
    let Ok(bytes) = STANDARD.decode(header) else {
        return KeyProtection::Unknown;
    };
    let Some(rest) = bytes.strip_prefix(OPENSSH_MAGIC) else {
        return KeyProtection::Unknown;
    };
    let Some((length, rest)) = rest.split_first_chunk::<4>() else {
        return KeyProtection::Unknown;
    };
    let length = u32::from_be_bytes(*length) as usize;
    match rest.get(..length) {
        Some(b"none") => KeyProtection::Unencrypted,
        Some(cipher) if !cipher.is_empty() => KeyProtection::Encrypted,
        _ => KeyProtection::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an OpenSSH envelope whose fields after the cipher are dummy
    /// bytes; the classifier must not depend on them.
    fn openssh_file(cipher: &str, line_ending: &str) -> String {
        let mut bytes = OPENSSH_MAGIC.to_vec();
        bytes.extend_from_slice(&(cipher.len() as u32).to_be_bytes());
        bytes.extend_from_slice(cipher.as_bytes());
        bytes.extend_from_slice(&[0; 48]);
        let encoded = STANDARD.encode(bytes);
        let mut text = format!("-----BEGIN OPENSSH PRIVATE KEY-----{line_ending}");
        for chunk in encoded.as_bytes().chunks(70) {
            text.push_str(std::str::from_utf8(chunk).unwrap());
            text.push_str(line_ending);
        }
        text.push_str("-----END OPENSSH PRIVATE KEY-----");
        text.push_str(line_ending);
        text
    }

    fn pem(label: &str, headers: &str) -> String {
        format!("-----BEGIN {label}-----\n{headers}QUJDRA==\n-----END {label}-----\n")
    }

    #[test]
    fn openssh_cipher_name_decides_protection() {
        assert_eq!(
            classify(&openssh_file("none", "\n")),
            KeyProtection::Unencrypted
        );
        for cipher in [
            "aes256-ctr",
            "aes256-gcm@openssh.com",
            "chacha20-poly1305@openssh.com",
        ] {
            assert_eq!(
                classify(&openssh_file(cipher, "\n")),
                KeyProtection::Encrypted,
                "{cipher}"
            );
        }
    }

    #[test]
    fn only_the_openssh_header_is_decoded() {
        // A body that is not valid base64 after the header proves the key
        // sections are never decoded.
        let header = &openssh_file("none", "")["-----BEGIN OPENSSH PRIVATE KEY-----".len()..]
            [..OPENSSH_HEADER_CHARS];
        let text = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{header}\n!!!not-base64!!!\n-----END OPENSSH PRIVATE KEY-----\n"
        );
        assert_eq!(classify(&text), KeyProtection::Unencrypted);
        // The longest cipher name OpenSSH uses still fits in the header.
        assert_eq!(
            classify(&openssh_file("chacha20-poly1305@openssh.com", "\n")),
            KeyProtection::Encrypted
        );
    }

    #[test]
    fn windows_line_endings_and_surrounding_text_are_accepted() {
        assert_eq!(
            classify(&openssh_file("none", "\r\n")),
            KeyProtection::Unencrypted
        );
        assert_eq!(
            classify(&openssh_file("aes256-ctr", "\r\n")),
            KeyProtection::Encrypted
        );
        let with_preamble = format!(
            "Bag Attributes\n    localKeyID: 01\n{}",
            pem("PRIVATE KEY", "")
        );
        assert_eq!(classify(&with_preamble), KeyProtection::Unencrypted);
    }

    #[test]
    fn legacy_pem_headers_decide_protection() {
        for label in ["RSA PRIVATE KEY", "EC PRIVATE KEY", "DSA PRIVATE KEY"] {
            assert_eq!(classify(&pem(label, "")), KeyProtection::Unencrypted);
            assert_eq!(
                classify(&pem(
                    label,
                    "Proc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00112233445566778899AABBCCDDEEFF\n\n"
                )),
                KeyProtection::Encrypted,
                "{label}"
            );
        }
    }

    #[test]
    fn pkcs8_label_decides_protection() {
        assert_eq!(
            classify(&pem("PRIVATE KEY", "")),
            KeyProtection::Unencrypted
        );
        assert_eq!(
            classify(&pem("ENCRYPTED PRIVATE KEY", "")),
            KeyProtection::Encrypted
        );
    }

    #[test]
    fn unrecognised_or_damaged_files_stay_unknown() {
        let truncated = openssh_file("none", "\n");
        let truncated = truncated
            .trim_end()
            .trim_end_matches("-----END OPENSSH PRIVATE KEY-----");
        let not_base64 =
            "-----BEGIN OPENSSH PRIVATE KEY-----\n!!!\n-----END OPENSSH PRIVATE KEY-----\n";
        let wrong_magic = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            STANDARD.encode(b"not-an-openssh-key\0\0\0\0\x04none")
        );
        let short_cipher = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            STANDARD.encode([OPENSSH_MAGIC, &[0, 0, 0, 9], b"none"].concat())
        );
        let empty_cipher = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            STANDARD.encode([OPENSSH_MAGIC, &[0, 0, 0, 0]].concat())
        );
        let mismatched = "-----BEGIN PRIVATE KEY-----\nQUJDRA==\n-----END RSA PRIVATE KEY-----\n";
        for text in [
            "",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA user@host",
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n",
            &pem("CERTIFICATE", ""),
            &pem("SSH2 ENCRYPTED PRIVATE KEY", ""),
            truncated,
            not_base64,
            &wrong_magic,
            &short_cipher,
            &empty_cipher,
            mismatched,
        ] {
            assert_eq!(classify(text), KeyProtection::Unknown, "{text:?}");
        }
    }

    #[test]
    fn unreadable_binary_and_oversized_files_stay_unknown() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("arcnix-key-check-{}", crate::new_id()));
        std::fs::create_dir(&directory)?;
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());

        assert_eq!(
            key_protection(&directory.join("missing")),
            KeyProtection::Unknown
        );
        assert_eq!(key_protection(&directory), KeyProtection::Unknown);

        let binary = directory.join("binary");
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x80])?;
        assert_eq!(key_protection(&binary), KeyProtection::Unknown);

        let unencrypted = openssh_file("none", "\n");
        let readable = directory.join("id_ed25519");
        std::fs::write(&readable, &unencrypted)?;
        assert_eq!(key_protection(&readable), KeyProtection::Unencrypted);

        #[cfg(unix)]
        {
            // Opening a FIFO blocks until a writer appears; the check must
            // reject it without opening it.
            let fifo = directory.join("fifo");
            let created = std::process::Command::new("mkfifo").arg(&fifo).status()?;
            assert!(created.success());
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || sender.send(key_protection(&fifo)));
            assert_eq!(
                receiver.recv_timeout(std::time::Duration::from_secs(5)),
                Ok(KeyProtection::Unknown),
                "inspecting a FIFO must not block"
            );
        }

        let oversized = directory.join("oversized");
        let padding = " ".repeat(MAX_KEY_FILE_BYTES as usize);
        std::fs::write(&oversized, format!("{unencrypted}{padding}"))?;
        assert_eq!(key_protection(&oversized), KeyProtection::Unknown);
        Ok(())
    }
}
