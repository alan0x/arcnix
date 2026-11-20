//! Ephemeral configuration snapshots and bounded local comparisons.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const LIMIT: usize = crate::configurations::LIMIT;
pub const RESPONSE_LIMIT: usize = 256 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u8,
    pub path: String,
}
impl Plan {
    pub fn new(path: String) -> Result<Self> {
        path_valid(&path)?;
        Ok(Self { version: 1, path })
    }
    pub fn execution(
        &self,
        administrator: bool,
        root: bool,
        sudo: bool,
        password: bool,
    ) -> Result<crate::packages::Execution> {
        ensure!(
            self.version == 1,
            "Invalid configuration comparison request"
        );
        path_valid(&self.path)?;
        let script = format!(
            "# arcnix:configuration-compare:read\ncd /\nexec timeout --kill-after=2 15 python3 -I -c {} {}",
            crate::remote_tasks::quote(
                &include_str!("configuration_compare.py").replace("\r\n", "\n")
            ),
            crate::remote_tasks::quote(&STANDARD.encode(serde_json::to_vec(self)?))
        );
        crate::operations::shell_execution(
            &crate::operations::Plan {
                label: "Read configuration snapshot".into(),
                script,
                administrator,
                is_root: root,
                sudo_available: sudo,
            },
            password,
        )
    }
}
fn path_valid(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path != "/"
            && path.len() <= 1024
            && !path.chars().any(char::is_control)
            && !path.split('/').any(|s| matches!(s, "." | ".."))
            && !["/proc", "/sys", "/dev"]
                .iter()
                .any(|p| path == *p || path.starts_with(&format!("{p}/"))),
        "Choose an absolute configuration file path outside /proc, /sys and /dev"
    );
    Ok(())
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    Missing,
    Denied,
    NotRegular,
    TooLarge,
    Changed,
    Binary,
    UnsupportedPath,
    Unavailable,
}
impl Issue {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "File not found",
            Self::Denied => "Permission denied",
            Self::NotRegular => "Not a regular configuration file",
            Self::TooLarge => "Configuration file exceeds 128 KiB",
            Self::Changed => "File changed during reading; read snapshots again",
            Self::Binary => "Configuration file is not UTF-8 text",
            Self::UnsupportedPath => "Unsupported configuration file path",
            Self::Unavailable => "Configuration snapshot unavailable",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    version: u8,
    path: String,
    sampled: u64,
    snapshot: Option<File>,
    issue: Option<Issue>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    canonical: String,
    bytes: usize,
    sha256: String,
    content: String,
    uid: u32,
    gid: u32,
    mode: u32,
    symlink: bool,
}
#[derive(Clone)]
pub struct Snapshot {
    pub sampled: u64,
    pub issue: Option<Issue>,
    pub file: Option<View>,
}
#[derive(Clone)]
pub struct View {
    pub canonical: String,
    pub bytes: usize,
    pub sha256: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub symlink: bool,
    pub text: String,
    pub redacted: bool,
    pub shortened: bool,
    pub bom: bool,
    pub endings: &'static str,
}
fn append_bounded(output: &mut String, value: &str, limit: usize) -> bool {
    let mut end = value.len().min(limit.saturating_sub(output.len()));
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&value[..end]);
    end == value.len()
}
/// Match original text only; inserted markers must never become new matches.
pub fn redact_text(value: &str, secrets: &[&str], limit: usize) -> (String, bool, bool) {
    let mut keys = secrets
        .iter()
        .copied()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    keys.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    keys.dedup();
    let mut positions = keys.iter().map(|key| value.find(*key)).collect::<Vec<_>>();
    let redacted = positions.iter().any(Option::is_some);
    let mut offset = 0;
    let mut output = String::new();
    while let Some((index, position)) = positions
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .min_by_key(|(i, p)| (*p, std::cmp::Reverse(keys[*i].len())))
    {
        if !append_bounded(&mut output, &value[offset..position], limit)
            || output.len() + 10 > limit
        {
            return (output, redacted, true);
        }
        output.push_str("[redacted]");
        offset = position + keys[index].len();
        for (i, key) in keys.iter().enumerate() {
            if positions[i].is_some_and(|p| p < offset) {
                positions[i] = value[offset..].find(*key).map(|p| p + offset);
            }
        }
    }
    let complete = append_bounded(&mut output, &value[offset..], limit);
    (output, redacted, !complete)
}
pub fn parse(text: &str, plan: &Plan, secrets: &[&str]) -> Result<Snapshot> {
    ensure!(
        plan.version == 1,
        "Invalid configuration comparison request"
    );
    path_valid(&plan.path)?;
    ensure!(
        text.len() <= RESPONSE_LIMIT,
        "Configuration snapshot response is too large"
    );
    let packet: Packet =
        serde_json::from_str(text).context("Invalid configuration snapshot response")?;
    ensure!(
        packet.version == 1
            && packet.path == plan.path
            && packet.sampled > 0
            && packet.sampled <= 253_402_300_799
            && packet.snapshot.is_some() != packet.issue.is_some(),
        "Incomplete or mismatched configuration snapshot"
    );
    let file = packet
        .snapshot
        .map(|file| -> Result<_> {
            path_valid(&file.canonical)?;
            ensure!(
                file.bytes <= LIMIT
                    && file.content.len() <= LIMIT.div_ceil(3) * 4
                    && file.mode <= 0o7777,
                "Invalid configuration snapshot bounds"
            );
            let bytes = STANDARD.decode(&file.content)?;
            ensure!(
                bytes.len() == file.bytes && format!("{:x}", Sha256::digest(&bytes)) == file.sha256,
                "Configuration snapshot checksum mismatch"
            );
            let text = String::from_utf8(bytes)?;
            ensure!(!text.contains('\0'), "Configuration file is not UTF-8 text");
            let bom = text.starts_with('\u{feff}');
            let crlf = text.matches("\r\n").count();
            let lf = text.matches('\n').count();
            let cr = text.matches('\r').count();
            let endings = if lf == 0 && cr == 0 {
                "No line breaks"
            } else if crlf > 0 && lf == crlf && cr == crlf {
                "CRLF"
            } else if cr == 0 {
                "LF"
            } else {
                "Mixed line endings"
            };
            let (text, redacted, shortened) = redact_text(&text, secrets, LIMIT * 2);
            Ok(View {
                canonical: file.canonical,
                bytes: file.bytes,
                sha256: file.sha256,
                uid: file.uid,
                gid: file.gid,
                mode: file.mode,
                symlink: file.symlink,
                text,
                redacted,
                shortened,
                bom,
                endings,
            })
        })
        .transpose()?;
    Ok(Snapshot {
        sampled: packet.sampled,
        issue: packet.issue,
        file,
    })
}
#[derive(Clone)]
pub struct Comparison {
    pub identical: bool,
    pub metadata_different: bool,
    pub shortened: bool,
    pub text: String,
    pub redacted: bool,
}
pub fn compare(base: &View, target: &View) -> Comparison {
    let identical = base.sha256 == target.sha256;
    let metadata_different =
        (base.uid, base.gid, base.mode) != (target.uid, target.gid, target.mode);
    let mut text = if identical {
        String::new()
    } else {
        similar::TextDiff::configure()
            .timeout(std::time::Duration::from_millis(120))
            .diff_lines(&base.text, &target.text)
            .unified_diff()
            .header("baseline", "compared server")
            .to_string()
    };
    let shortened = base.shortened || target.shortened || text.len() > 32 * 1024;
    if text.len() > 32 * 1024 {
        let mut end = 32 * 1024;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Comparison {
        identical,
        metadata_different,
        shortened,
        text,
        redacted: base.redacted || target.redacted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(content: &[u8]) -> serde_json::Value {
        serde_json::json!({"version":1,"path":"/etc/example.conf","sampled":1700000000,"issue":null,"snapshot":{"canonical":"/etc/example.conf","bytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content)),"content":STANDARD.encode(content),"uid":0,"gid":0,"mode":420,"symlink":false}})
    }
    #[test]
    fn snapshots_are_bound_complete_and_checked_before_redaction() -> Result<()> {
        let plan = Plan::new("/etc/example.conf".into())?;
        let body = "\u{feff}token=first-secret\r\nvalue=中文\r\n";
        let first = parse(
            &packet(body.as_bytes()).to_string(),
            &plan,
            &["first-secret", "second-secret"],
        )?;
        let file = first.file.unwrap();
        assert!(
            file.bom
                && file.redacted
                && file.endings == "CRLF"
                && file.text.contains("[redacted]")
                && !file.text.contains("first-secret")
        );
        assert_eq!(file.bytes, body.len());
        let other = parse(
            &packet(body.replace("first-secret", "second-secret").as_bytes()).to_string(),
            &plan,
            &["first-secret", "second-secret"],
        )?
        .file
        .unwrap();
        assert_eq!(file.text, other.text);
        assert!(!compare(&file, &other).identical);
        let mut failures = vec![];
        let mut wrong = packet(b"x");
        wrong["path"] = "/different".into();
        failures.push(wrong);
        let mut wrong = packet(b"x");
        wrong["snapshot"]["sha256"] = "0".repeat(64).into();
        failures.push(wrong);
        let mut wrong = packet(b"x");
        wrong["snapshot"]["bytes"] = 2.into();
        failures.push(wrong);
        let mut wrong = packet(b"x");
        wrong["snapshot"]["canonical"] = "/proc/version".into();
        failures.push(wrong);
        let mut wrong = packet(b"x");
        wrong["snapshot"]["mode"] = 65535.into();
        failures.push(wrong);
        let mut wrong = packet(b"x");
        wrong["issue"] = "missing".into();
        failures.push(wrong);
        failures.push(packet(b"x\0"));
        failures.push(packet(&[255]));
        for wrong in failures {
            assert!(parse(&wrong.to_string(), &plan, &[]).is_err());
        }
        let empty = parse(&packet(b"").to_string(), &plan, &[])?.file.unwrap();
        assert_eq!(empty.bytes, 0);
        assert!(compare(&empty, &empty).identical);
        let mut missing = packet(b"");
        missing["snapshot"] = serde_json::Value::Null;
        missing["issue"] = "missing".into();
        assert!(matches!(
            parse(&missing.to_string(), &plan, &[])?.issue,
            Some(Issue::Missing)
        ));
        Ok(())
    }
    #[test]
    fn local_diffs_keep_metadata_and_full_checksums_when_preview_is_shortened() -> Result<()> {
        let plan = Plan::new("/etc/example.conf".into())?;
        let base = parse(&packet(b"one\ntwo\n").to_string(), &plan, &[])?
            .file
            .unwrap();
        let mut packet = packet(b"one\ntwo\n");
        packet["snapshot"]["mode"] = 384.into();
        let target = parse(&packet.to_string(), &plan, &[])?.file.unwrap();
        let same = compare(&base, &target);
        assert!(same.identical && same.metadata_different && same.text.is_empty());
        let large = (0..12000)
            .map(|n| format!("A-{n:05}\n"))
            .collect::<String>();
        let a = parse(&self::packet(large.as_bytes()).to_string(), &plan, &[])?
            .file
            .unwrap();
        let b = parse(
            &self::packet(large.replace('A', "B").as_bytes()).to_string(),
            &plan,
            &[],
        )?
        .file
        .unwrap();
        let diff = compare(&a, &b);
        assert!(!diff.identical && diff.shortened && diff.text.len() <= 32 * 1024);
        assert!(
            parse(
                &self::packet(&vec![b'x'; LIMIT + 1]).to_string(),
                &plan,
                &[]
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn read_commands_quote_paths_and_elevation_is_explicit() -> Result<()> {
        for path in [
            "/",
            "relative",
            "/etc/../secret",
            "/proc/version",
            "/dev/null",
            "/sys/kernel",
        ] {
            assert!(Plan::new(path.into()).is_err());
        }
        let plan = Plan::new("/srv/配置 ' $(id) $x.conf".into())?;
        let ordinary = plan.execution(false, false, false, false)?;
        assert!(
            !ordinary.command.contains("sudo")
                && !ordinary.use_stdin_password
                && ordinary.command.contains("timeout --kill-after=2 15")
        );
        assert!(plan.execution(true, false, false, false).is_err());
        assert!(plan.execution(true, false, true, true)?.use_stdin_password);
        Ok(())
    }
    #[test]
    fn duplicate_short_credentials_never_reprocess_markers_or_expand_without_a_bound() {
        let keys = vec!["e", "e", "a", "r", "d", "[", "]", "secret", "secret-value"];
        assert_eq!(
            redact_text("eeee", &keys, 100),
            ("[redacted]".repeat(4), true, false)
        );
        assert_eq!(
            redact_text("secret-value", &keys, 100),
            ("[redacted]".into(), true, false)
        );
        let (text, redacted, shortened) = redact_text(&"e".repeat(LIMIT), &keys, 4096);
        assert!(redacted && shortened && text.len() <= 4096);
        assert_eq!(
            redact_text("中文 secret-value", &keys, 4),
            ("中".into(), true, true)
        );
    }
}
