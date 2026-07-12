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
    } else {
        (s, 3_600_000u64)
    };
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {num_str}"))?;
    Ok(n * mult)
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

pub(crate) fn datetime_to_millis(month: u32, day: u32, hour: u32, minute: u32) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    for year_offset in [0i64, -1] {
        let year = 1970 + (now as f64 / 31557600.0) as i64 + year_offset;
        if let Some(ms) = date_to_millis(year as i32, month, day, hour, minute) {
            if ms <= now as u64 * 1000 || year_offset < 0 {
                return ms;
            }
        }
    }
    0
}

fn date_to_millis(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }

    let days = days_since_epoch(year, month, day)?;
    Some((days as u64 * 86400 + hour as u64 * 3600 + minute as u64 * 60) * 1000)
}

fn days_since_epoch(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let y = if month <= 2 {
        year as i64 - 1
    } else {
        year as i64
    };
    let m = if month <= 2 {
        month as i64 + 12
    } else {
        month as i64
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days)
}

pub(crate) fn format_time_label(left: &str, right: &str) -> String {
    format!("{} ~ {}", left, right)
}

pub(crate) fn format_duration_label(ms: u64) -> String {
    if ms % 604_800_000 == 0 {
        format!("{}w", ms / 604_800_000)
    } else if ms % 86_400_000 == 0 {
        format!("{}d", ms / 86_400_000)
    } else {
        format!("{}h", ms / 3_600_000)
    }
}

pub(crate) fn format_absolute_label(month: u32, day: u32, hour: u32, minute: u32) -> String {
    if hour == 0 && minute == 0 {
        format!("{month:02}-{day:02}")
    } else {
        format!("{month:02}-{day:02} {hour:02}:{minute:02}")
    }
}

pub(crate) fn today_md() -> (u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = now / 86400 + 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    (month, day)
}
