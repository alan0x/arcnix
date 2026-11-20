//! One display policy for files, properties, logs and transfer statistics.
use std::cell::Cell;

use arcnix_core::preferences::{DateStyle, DisplaySettings, DisplayTimezone, SizeUnits};
thread_local! { static SETTINGS: Cell<DisplaySettings> = Cell::new(DisplaySettings::default()); }
pub(crate) fn configure(settings: DisplaySettings) {
    SETTINGS.set(settings);
}
fn grouped(value: &str) -> String {
    let mut result = String::new();
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
    for (index, c) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    if !fraction.is_empty() {
        result.push('.');
        result.push_str(fraction);
    }
    result
}
pub(crate) fn bytes(bytes: u64) -> String {
    bytes_with(bytes, SETTINGS.get())
}
pub(crate) fn bytes_with(bytes: u64, settings: DisplaySettings) -> String {
    if settings.units == SizeUnits::Bytes {
        let n = bytes.to_string();
        return format!(
            "{} B",
            if settings.group_digits {
                grouped(&n)
            } else {
                n
            }
        );
    }
    let (base, units) = if settings.units == SizeUnits::Iec {
        (1024f64, ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"])
    } else {
        (1000f64, ["B", "KB", "MB", "GB", "TB", "PB", "EB"])
    };
    let mut index = 0;
    let mut value = bytes as f64;
    while value >= base && index < units.len() - 1 {
        value /= base;
        index += 1;
    }
    let n = if index == 0 {
        bytes.to_string()
    } else {
        format!("{:.*}", settings.decimals.min(3) as usize, value)
    };
    format!(
        "{} {}",
        if settings.group_digits {
            grouped(&n)
        } else {
            n
        },
        units[index]
    )
}
pub(crate) fn timestamp(seconds: Option<u64>) -> String {
    let Some(date) = seconds
        .and_then(|s| i64::try_from(s).ok())
        .and_then(|s| time::OffsetDateTime::from_unix_timestamp(s).ok())
    else {
        return "—".into();
    };
    let settings = SETTINGS.get();
    let date = if settings.timezone == DisplayTimezone::Utc {
        date
    } else {
        date.to_offset(time::UtcOffset::local_offset_at(date).unwrap_or(time::UtcOffset::UTC))
    };
    if settings.dates == DateStyle::System
        && let Some(text) = system_date(date)
    {
        return text;
    }
    date.format(&time::format_description::well_known::Rfc3339)
        .map(|s| s.replace('T', " "))
        .unwrap_or_else(|_| "—".into())
}
pub(crate) fn modified(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "—".into();
    };
    if SETTINGS.get().dates != DateStyle::Relative {
        return timestamp(Some(seconds));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if seconds > now {
        return timestamp(Some(seconds));
    }
    let elapsed = now - seconds;
    crate::i18n::translate(&if elapsed < 60 {
        "刚刚".into()
    } else if elapsed < 3600 {
        format!("{} 分钟前", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{} 小时前", elapsed / 3600)
    } else {
        format!("{} 天前", elapsed / 86400)
    })
}
#[cfg(windows)]
fn system_date(date: time::OffsetDateTime) -> Option<String> {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::Globalization::{
        DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_FORMAT_FLAGS,
    };
    let native = SYSTEMTIME {
        wYear: date.year().try_into().ok()?,
        wMonth: date.month() as u16,
        wDay: date.day() as u16,
        wHour: date.hour() as u16,
        wMinute: date.minute() as u16,
        wSecond: date.second() as u16,
        ..Default::default()
    };
    let mut day = [0u16; 128];
    let mut clock = [0u16; 128];
    // Null locale and format request the operating system's user preferences.
    let d = unsafe {
        GetDateFormatEx(
            None,
            DATE_SHORTDATE,
            Some(&native),
            None,
            Some(&mut day),
            None,
        )
    };
    let t = unsafe {
        GetTimeFormatEx(
            None,
            TIME_FORMAT_FLAGS(0),
            Some(&native),
            None,
            Some(&mut clock),
        )
    };
    if d <= 0 || t <= 0 {
        return None;
    }
    Some(format!(
        "{} {} {}",
        String::from_utf16_lossy(&day[..d as usize - 1]),
        String::from_utf16_lossy(&clock[..t as usize - 1]),
        date.offset()
    ))
}
#[cfg(not(windows))]
fn system_date(_date: time::OffsetDateTime) -> Option<String> {
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn units_grouping_and_large_counts() {
        let mut s = DisplaySettings::default();
        assert_eq!(bytes_with(1024, s), "1.0 KiB");
        s.units = SizeUnits::Decimal;
        assert_eq!(bytes_with(1000, s), "1.0 KB");
        s.units = SizeUnits::Bytes;
        s.group_digits = true;
        assert_eq!(bytes_with(1234567, s), "1,234,567 B");
        assert_eq!(bytes_with(u64::MAX, s), "18,446,744,073,709,551,615 B");
    }
}
