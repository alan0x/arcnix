use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

use super::host_capabilities::checked_command;
use super::system_logs::{
    JournalPeriod, LogCapabilities, LogEntry, LogQuery, LogSnapshot, LogSource, validate_query,
};
use super::windows_host::json;
use crate::remote::RemoteSession;

pub(super) fn discover_darwin(remote: &RemoteSession) -> Result<LogCapabilities> {
    checked_command(remote, "test -x /usr/bin/log", "Unified log")?;
    Ok(LogCapabilities {
        sources: vec![LogSource::Unified],
        warnings: Vec::new(),
    })
}

pub(super) fn darwin_command(query: &LogQuery) -> Result<String> {
    validate_query(query)?;
    ensure!(
        query.source == LogSource::Unified && query.unit.trim().is_empty(),
        "This filter is unavailable for the macOS unified log"
    );
    let interval = match query.period {
        JournalPeriod::RecentHour => "--last 1h".into(),
        JournalPeriod::Today => "--today".into(),
        JournalPeriod::Boot => "--last boot".into(),
        JournalPeriod::Window { start, end } => format!("--start '@{start}' --end '@{end}'"),
    };
    let priority = match query.priority {
        Some(0 | 1) => return Ok("printf '%s\\n' ARCNIX_UNIFIED_LOG_V1".into()),
        Some(2) => "--predicate 'logType == \"fault\"'",
        Some(3 | 4) => "--predicate 'logType == \"fault\" OR logType == \"error\"'",
        Some(5) => "",
        Some(6) => "--info",
        _ => "--info --debug",
    };
    // Preserve log's exit status, keep only the latest rows, and exclude oversized JSON records.
    // The command is built from fixed options and validated integers.
    Ok(format!(
        r#"
printf 'ARCNIX_UNIFIED_LOG_V1\n'
/bin/bash -o pipefail -s <<'ARCNIX_UNIFIED_LOG'
export LC_ALL=C
/usr/bin/log show --style ndjson --timezone UTC --no-pager --color none --no-loss {interval} {priority} |
  /usr/bin/awk 'length($0) <= 4096' |
  /usr/bin/tail -n {}
ARCNIX_UNIFIED_LOG
"#,
        query.limit + 1
    ))
}

pub(super) fn parse_darwin(output: &str, query: &LogQuery) -> Result<LogSnapshot> {
    let mut lines = output.lines();
    ensure!(
        lines.next() == Some("ARCNIX_UNIFIED_LOG_V1"),
        "Incomplete macOS unified log response"
    );
    let mut entries = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).context("Invalid macOS unified log record")?;
        let Some(message) = row["eventMessage"].as_str() else {
            continue;
        };
        let field = |key| {
            row[key]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let priority = match row["messageType"].as_str() {
            Some("Fault") => Some(2),
            Some("Error") => Some(3),
            Some("Default" | "Release") => Some(5),
            Some("Info") => Some(6),
            Some("Debug") => Some(7),
            _ => None,
        };
        if query
            .priority
            .is_some_and(|limit| priority.is_none_or(|actual| actual > limit))
        {
            continue;
        }
        entries.push(LogEntry {
            timestamp: field("timestamp"),
            unit: field("subsystem").or_else(|| {
                field("processImagePath")
                    .and_then(|path| path.rsplit('/').next().map(str::to_owned))
            }),
            priority,
            message: message.chars().take(4096).collect(),
        });
    }
    let entries = entries.split_off(entries.len().saturating_sub(query.limit));
    Ok(LogSnapshot {
        source: LogSource::Unified,
        entries,
        warnings: Vec::new(),
    })
}

pub(super) fn read_darwin(remote: &RemoteSession, query: &LogQuery) -> Result<LogSnapshot> {
    let output = checked_command(remote, &darwin_command(query)?, "Unified log")?;
    let mut snapshot = parse_darwin(&output.stdout, query)?;
    if !output.stderr.trim().is_empty() {
        snapshot.warnings.push(output.stderr.trim().to_owned());
    }
    Ok(snapshot)
}

pub(super) const WINDOWS_DISCOVER: &str = r#"
$sources = @()
$warnings = @()
foreach ($channel in @('System','Application')) {
    try {
        if ((Get-WinEvent -ListLog $channel).IsEnabled) { $sources += $channel }
    } catch { $warnings += "$channel event log is unavailable: $($_.Exception.Message)" }
}
$result = @{ sources = $sources; warnings = $warnings }
"#;

pub(super) fn discover_windows(remote: &RemoteSession) -> Result<LogCapabilities> {
    #[derive(Deserialize)]
    struct Reply {
        sources: Vec<String>,
        warnings: Vec<String>,
    }
    let reply = json::<Reply>(remote, WINDOWS_DISCOVER, "Windows Event Log")?;
    let sources = reply
        .sources
        .into_iter()
        .filter_map(|channel| match channel.as_str() {
            "System" => Some(LogSource::WindowsSystem),
            "Application" => Some(LogSource::WindowsApplication),
            _ => None,
        })
        .collect();
    Ok(LogCapabilities {
        sources,
        warnings: reply.warnings,
    })
}

pub(super) fn windows_script(query: &LogQuery) -> Result<String> {
    validate_query(query)?;
    ensure!(
        query.unit.trim().is_empty(),
        "Service-unit filters are unavailable for Windows Event Log"
    );
    let channel = match query.source {
        LogSource::WindowsSystem => "System",
        LogSource::WindowsApplication => "Application",
        _ => anyhow::bail!("This log source is unavailable on Windows"),
    };
    let (start, end) = match query.period {
        JournalPeriod::RecentHour => ("(Get-Date).AddHours(-1)".into(), None),
        JournalPeriod::Today => ("(Get-Date).Date".into(), None),
        JournalPeriod::Boot => (
            "(Get-CimInstance Win32_OperatingSystem).LastBootUpTime".into(),
            None,
        ),
        JournalPeriod::Window { start, end } => (
            format!("[DateTimeOffset]::FromUnixTimeSeconds({start}).LocalDateTime"),
            Some(format!(
                "$filter.EndTime=[DateTimeOffset]::FromUnixTimeSeconds({end}).LocalDateTime"
            )),
        ),
    };
    let level = match query.priority {
        Some(0 | 1) => return Ok("$result=@()".into()),
        Some(2) => "$filter.Level=@(1)",
        Some(3) => "$filter.Level=@(1,2)",
        Some(4 | 5) => "$filter.Level=@(1,2,3)",
        Some(6) => "$filter.Level=@(1,2,3,4)",
        _ => "",
    };
    Ok(format!(
        r#"
$filter=@{{LogName='{channel}';StartTime={start}}}
{end}
{level}
try {{
    $result=@(Get-WinEvent -FilterHashtable $filter -MaxEvents {limit} | ForEach-Object {{
        try {{ $message = [string]$_.Message }} catch {{ $message = '' }}
        if (!$message) {{ $message = "Event $($_.Id)" }}
        if ($message.Length -gt 768) {{
            $length = if ([char]::IsHighSurrogate($message[767])) {{ 767 }} else {{ 768 }}
            $message = $message.Substring(0, $length) + '...'
        }}
        $time = if ($_.TimeCreated) {{ $_.TimeCreated.ToUniversalTime().ToString('o') }} else {{ $null }}
        @{{ timestamp = $time; provider = $_.ProviderName; level = $_.Level; message = $message }}
    }})
}} catch {{
    if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound,*') {{ $result=@() }} else {{ throw }}
}}
"#,
        end = end.unwrap_or_default(),
        limit = query.limit
    ))
}

pub(super) fn read_windows(remote: &RemoteSession, query: &LogQuery) -> Result<LogSnapshot> {
    #[derive(Deserialize)]
    struct Event {
        timestamp: Option<String>,
        provider: Option<String>,
        level: Option<u8>,
        message: String,
    }
    let events = json::<Vec<Event>>(remote, &windows_script(query)?, "Windows Event Log")?;
    let entries = events
        .into_iter()
        .take(query.limit)
        .rev()
        .map(|row| LogEntry {
            timestamp: row.timestamp,
            unit: row.provider,
            priority: match row.level {
                Some(1) => Some(2),
                Some(2) => Some(3),
                Some(3) => Some(4),
                Some(4) => Some(6),
                Some(5) => Some(7),
                _ => None,
            },
            message: row.message,
        })
        .collect();
    Ok(LogSnapshot {
        source: query.source,
        entries,
        warnings: Vec::new(),
    })
}
