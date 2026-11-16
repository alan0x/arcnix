//! Bounded exit receipts tied to a task and one systemd runtime cycle.
use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
pub const LIMIT: usize = 192;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    Legacy,
    Invalid,
    TaskChanged,
    InvocationChanged,
    InvocationUnavailable,
    ObservationChanged,
}
impl Issue {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Legacy => {
                "Legacy exit result has no runtime binding; inspect the original server task"
            }
            Self::Invalid => "Saved server task exit result is invalid; inspect the original task.",
            Self::TaskChanged => "Saved exit receipt belongs to a different task",
            Self::InvocationChanged => {
                "Saved exit receipt belongs to a different service invocation"
            }
            Self::InvocationUnavailable => {
                "Cannot verify the service invocation for this saved exit receipt"
            }
            Self::ObservationChanged => {
                "Server task state changed during the query; query the original task again."
            }
        }
    }
}
#[derive(Debug)]
pub struct Evidence {
    pub code: Option<i32>,
    pub issue: Option<Issue>,
}
pub fn valid_invocation(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn code(raw: &str) -> Option<i32> {
    if raw.is_empty()
        || raw.len() > 1 && raw.starts_with('0')
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        None
    } else {
        raw.parse::<u8>().ok().map(i32::from)
    }
}
/// `None` means the manager explicitly reported the unit missing, not unavailable.
pub fn parse(encoded: &str, task: &str, invocation: Option<&str>) -> Result<Evidence> {
    let issue = |issue| Evidence {
        code: None,
        issue: Some(issue),
    };
    if encoded == "-" {
        return Ok(Evidence {
            code: None,
            issue: None,
        });
    }
    ensure!(
        encoded.len() <= LIMIT.div_ceil(3) * 4,
        "Invalid server task exit encoding"
    );
    let bytes = STANDARD.decode(encoded)?;
    ensure!(bytes.len() <= LIMIT, "Invalid server task exit encoding");
    let Ok(raw) = std::str::from_utf8(&bytes) else {
        return Ok(issue(Issue::Invalid));
    };
    if code(raw.strip_suffix('\n').unwrap_or(raw)).is_some() {
        return Ok(issue(Issue::Legacy));
    }
    let Some(line) = raw.strip_suffix('\n') else {
        return Ok(issue(Issue::Invalid));
    };
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 4
        || fields[0] != "ARCNIX_TASK_RESULT_V1"
        || crate::remote_tasks::validate_id(fields[1]).is_err()
        || !valid_invocation(fields[2])
    {
        return Ok(issue(Issue::Invalid));
    }
    let Some(code) = code(fields[3]) else {
        return Ok(issue(Issue::Invalid));
    };
    if fields[1] != task {
        return Ok(issue(Issue::TaskChanged));
    }
    if let Some(expected) = invocation {
        if !valid_invocation(expected) {
            return Ok(issue(Issue::InvocationUnavailable));
        }
        if expected != fields[2] {
            return Ok(issue(Issue::InvocationChanged));
        }
    }
    Ok(Evidence {
        code: Some(code),
        issue: None,
    })
}
pub(crate) fn shell_suffix(id: &str, directory: &str) -> String {
    use crate::remote_tasks::quote;
    format!(
        "\ncode=$?\nprintf '%s\\t%s\\t%s\\t%s\\n' 'ARCNIX_TASK_RESULT_V1' {} \"$arcnix_invocation\" \"$code\" > {}\nmv -f -- {} {}\nexit \"$code\"",
        quote(id),
        quote(&format!("{directory}/result.tmp")),
        quote(&format!("{directory}/result.tmp")),
        quote(&format!("{directory}/result"))
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_require_exact_task_runtime_and_canonical_complete_bytes() -> Result<()> {
        let invocation = "a".repeat(32);
        let receipt = format!("ARCNIX_TASK_RESULT_V1\tjob-1\t{invocation}\t7\n");
        let encoded = STANDARD.encode(&receipt);
        assert_eq!(parse(&encoded, "job-1", Some(&invocation))?.code, Some(7));
        assert_eq!(parse(&encoded, "job-1", None)?.code, Some(7));
        assert_eq!(
            parse(&encoded, "other", None)?.issue,
            Some(Issue::TaskChanged)
        );
        assert_eq!(
            parse(&encoded, "job-1", Some(&"b".repeat(32)))?.issue,
            Some(Issue::InvocationChanged)
        );
        assert_eq!(
            parse(&encoded, "job-1", Some("-"))?.issue,
            Some(Issue::InvocationUnavailable)
        );
        for bytes in [b"0\n".as_slice(), b"7", b"255\n"] {
            assert_eq!(
                parse(&STANDARD.encode(bytes), "job-1", None)?.issue,
                Some(Issue::Legacy)
            );
        }
        for raw in [
            receipt.trim_end().to_owned(),
            receipt.clone() + "\n",
            receipt.replace("7\n", "07\n"),
            receipt.replace("7\n", "256\n"),
            receipt.replace("7\n", "0\0\n"),
            receipt.replace(&invocation, &"A".repeat(32)),
            receipt.replace("job-1", "x/y"),
        ] {
            assert_eq!(
                parse(&STANDARD.encode(raw), "job-1", None)?.issue,
                Some(Issue::Invalid)
            );
        }
        assert!(parse(&STANDARD.encode("x".repeat(LIMIT + 1)), "job-1", None).is_err());
        assert!(parse("???", "job-1", None).is_err());
        assert_eq!(parse("-", "job-1", None)?.code, None);
        Ok(())
    }
}
