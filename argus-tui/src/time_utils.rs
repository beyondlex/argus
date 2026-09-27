use chrono::{Datelike, Local, TimeZone};

pub fn now_in_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num_str, mult) = if s.ends_with('w') || s.ends_with('W') {
        (&s[..s.len() - 1], 604_800_000u64)
    } else if s.ends_with('d') || s.ends_with('D') {
        (&s[..s.len() - 1], 86_400_000u64)
    } else if s.ends_with('h') || s.ends_with('H') {
        (&s[..s.len() - 1], 3_600_000u64)
    } else if s.ends_with('m') || s.ends_with('M') {
        (&s[..s.len() - 1], 60_000u64)
    } else {
        (s, 3_600_000u64)
    };
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {num_str}"))?;
    // Absurd inputs must error, not wrap around silently.
    n.checked_mul(mult)
        .ok_or_else(|| format!("duration too large: {s}"))
}

pub(crate) fn parse_date_time(s: &str) -> Result<(u32, u32, u32, u32), String> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.len() {
        1 => {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            if date_parts.len() == 2 {
                let month: u32 = date_parts[0]
                    .parse()
                    .map_err(|_| format!("invalid month: {}", date_parts[0]))?;
                let day: u32 = date_parts[1]
                    .parse()
                    .map_err(|_| format!("invalid day: {}", date_parts[1]))?;
                Ok((month, day, 0, 0))
            } else {
                Err(format!("invalid date: {}", s))
            }
        }
        2 => {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            if date_parts.len() != 2 {
                return Err(format!("invalid date: {}", parts[0]));
            }
            let month: u32 = date_parts[0]
                .parse()
                .map_err(|_| format!("invalid month: {}", date_parts[0]))?;
            let day: u32 = date_parts[1]
                .parse()
                .map_err(|_| format!("invalid day: {}", date_parts[1]))?;
            let time_parts: Vec<&str> = parts[1].split(':').collect();
            if time_parts.len() != 2 {
                return Err(format!("invalid time: {}", parts[1]));
            }
            let hour: u32 = time_parts[0]
                .parse()
                .map_err(|_| format!("invalid hour: {}", time_parts[0]))?;
            let minute: u32 = time_parts[1]
                .parse()
                .map_err(|_| format!("invalid minute: {}", time_parts[1]))?;
            Ok((month, day, hour, minute))
        }
        _ => Err(format!("invalid date-time: {s}")),
    }
}

pub(crate) fn is_time_only(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 2
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
}

/// Local-time timestamp for a month/day[/hour:minute] in the current or
/// previous year. Errors on impossible dates (Feb 30, hour 24, minute 99):
/// returning 0 used to silently mean "from epoch", disabling the time
/// filter while the label still showed the typo.
pub(crate) fn datetime_to_millis(
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> Result<u64, String> {
    let now = Local::now();
    let year = now.year();
    for y in [year, year - 1] {
        if let Some(dt) = Local
            .with_ymd_and_hms(y, month, day, hour, minute, 0)
            .single()
        {
            let ms = dt.timestamp_millis() as u64;
            if ms <= now.timestamp_millis() as u64 || y < year {
                return Ok(ms);
            }
        }
    }
    Err(format!(
        "invalid date/time: {month:02}-{day:02} {hour:02}:{minute:02}"
    ))
}

pub(crate) fn format_time_label(left: &str, right: &str) -> String {
    format!("{} ~ {}", left, right)
}

pub(crate) fn format_duration_label(ms: u64) -> String {
    if ms.is_multiple_of(604_800_000) {
        format!("{}w", ms / 604_800_000)
    } else if ms.is_multiple_of(86_400_000) {
        format!("{}d", ms / 86_400_000)
    } else if ms.is_multiple_of(3_600_000) {
        format!("{}h", ms / 3_600_000)
    } else {
        format!("{}m", ms / 60_000)
    }
}

pub(crate) fn format_absolute_label(month: u32, day: u32, hour: u32, minute: u32) -> String {
    if hour == 0 && minute == 0 {
        format!("{month:02}-{day:02}")
    } else {
        format!("{month:02}-{day:02} {hour:02}:{minute:02}")
    }
}

pub struct ParsedTimeArg {
    pub ms: u64,
    pub label: String,
    pub date: Option<(u32, u32)>,
}

pub fn parse_single_time_arg(arg: &str) -> Result<ParsedTimeArg, String> {
    if is_time_only(arg) {
        let parts: Vec<&str> = arg.split(':').collect();
        let h: u32 = parts[0]
            .parse()
            .map_err(|_| format!("invalid hour: {arg}"))?;
        let min: u32 = parts[1]
            .parse()
            .map_err(|_| format!("invalid minute: {arg}"))?;
        let (m, d) = today_md();
        Ok(ParsedTimeArg {
            ms: datetime_to_millis(m, d, h, min)?,
            label: format!("{:02}:{:02}", h, min),
            date: Some((m, d)),
        })
    } else if let Ok(ms) = parse_duration(arg) {
        let now = now_in_millis();
        Ok(ParsedTimeArg {
            ms: now.saturating_sub(ms),
            label: format_duration_label(ms),
            date: None,
        })
    } else {
        let (m, d, h, min) = parse_date_time(arg).map_err(|e| format!("invalid date-time: {e}"))?;
        Ok(ParsedTimeArg {
            ms: datetime_to_millis(m, d, h, min)?,
            label: format_absolute_label(m, d, h, min),
            date: Some((m, d)),
        })
    }
}

pub(crate) fn today_md() -> (u32, u32) {
    let now = Local::now();
    (now.month(), now.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_units() {
        assert_eq!(parse_duration("90m").unwrap(), 90 * 60_000);
        assert_eq!(parse_duration("2h").unwrap(), 2 * 3_600_000);
        assert_eq!(parse_duration("3d").unwrap(), 3 * 86_400_000);
        assert_eq!(parse_duration("1w").unwrap(), 604_800_000);
        // Unit-less defaults to hours.
        assert_eq!(parse_duration("5").unwrap(), 5 * 3_600_000);
    }

    #[test]
    fn test_parse_duration_rejects_garbage_and_overflow() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("d").is_err());
        assert!(parse_duration("99999999999999999999d").is_err());
    }

    /// Impossible calendar dates must error, not fall back to epoch (which
    /// silently disabled the time filter while the label showed the typo).
    #[test]
    fn test_datetime_to_millis_rejects_impossible_dates() {
        assert!(datetime_to_millis(2, 30, 0, 0).is_err());
        assert!(datetime_to_millis(13, 1, 0, 0).is_err());
        assert!(datetime_to_millis(1, 1, 24, 0).is_err());
        assert!(datetime_to_millis(1, 1, 12, 99).is_err());
    }

    #[test]
    fn test_datetime_to_millis_accepts_real_dates() {
        // Jan 1 00:00 is always in the past (or clamps to last year).
        assert!(datetime_to_millis(1, 1, 0, 0).is_ok());
        assert!(datetime_to_millis(12, 31, 23, 59).is_ok());
    }

    #[test]
    fn test_parse_single_time_arg_rejects_impossible_time() {
        assert!(parse_single_time_arg("12:99").is_err());
    }

    #[test]
    fn test_parse_single_time_arg_duration_ok() {
        let parsed = parse_single_time_arg("3d").unwrap();
        assert_eq!(parsed.label, "3d");
        assert!(parsed.date.is_none());
        assert!(parsed.ms <= now_in_millis());
    }
}
