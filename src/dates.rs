use std::time::{SystemTime, UNIX_EPOCH};

pub fn is_rfc3339_utc(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() {
            return false;
        }
    }
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..10].parse::<u32>().unwrap_or(0);
    let hour = value[11..13].parse::<u32>().unwrap_or(24);
    let minute = value[14..16].parse::<u32>().unwrap_or(60);
    let second = value[17..19].parse::<u32>().unwrap_or(60);
    (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24 && minute < 60 && second < 60
}

pub fn now_rfc3339() -> String {
    unix_to_rfc3339(now_secs())
}

pub fn months_ago_rfc3339(months: i64) -> String {
    months_before_rfc3339(now_secs(), months)
}

/// Subtracts calendar months, clamping the day to the target month's length.
pub fn months_before_rfc3339(secs: i64, months: i64) -> String {
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let total = year * 12 + month - 1 - months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let month_start = days_from_civil(year, month, 1);
    let month_len = days_from_civil(year + month / 12, month % 12 + 1, 1) - month_start;
    let days = month_start + day.min(month_len) - 1;
    unix_to_rfc3339(days * 86_400 + secs.rem_euclid(86_400))
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn unix_to_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub fn date_days(value: &str) -> Option<i64> {
    let date = value.get(0..10)?;
    let bytes = date.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = date.get(0..4)?.parse::<i64>().ok()?;
    let month = date.get(5..7)?.parse::<i64>().ok()?;
    let day = date.get(8..10)?.parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

pub fn date_from_days(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted_month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + (m <= 2) as i64, m, d)
}

pub fn short_date(iso: &str) -> String {
    iso.get(0..10).unwrap_or(iso).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtracts_calendar_months() {
        let secs = |y, m, d| days_from_civil(y, m, d) * 86_400 + 3_661;
        assert_eq!(
            months_before_rfc3339(secs(2026, 5, 31), 3),
            "2026-02-28T01:01:01Z"
        );
        assert_eq!(
            months_before_rfc3339(secs(2024, 3, 31), 1),
            "2024-02-29T01:01:01Z"
        );
        assert_eq!(
            months_before_rfc3339(secs(2026, 1, 15), 13),
            "2024-12-15T01:01:01Z"
        );
        assert_eq!(
            months_before_rfc3339(secs(2026, 1, 15), 0),
            "2026-01-15T01:01:01Z"
        );
    }

    #[test]
    fn validates_utc_timestamp_shape() {
        assert!(is_rfc3339_utc("2026-05-27T11:10:47Z"));
        assert!(!is_rfc3339_utc("2026-99-27T11:10:47Z"));
        assert!(!is_rfc3339_utc("2026-05-27 11:10:47Z"));
    }
}
