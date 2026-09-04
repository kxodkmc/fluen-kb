//! UTC 时间戳（RFC3339）。零依赖，civil-from-days 算法（Howard Hinnant）。

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    rfc3339(secs)
}

pub fn now_year_month() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let (y, m, _) = civil_from_days(secs.div_euclid(86400));
    format!("{y:04}{m:02}")
}

pub fn rfc3339(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86400);
    let sod = unix_secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(rfc3339(1_000_000_000), "2001-09-09T01:46:40+00:00");
        assert_eq!(rfc3339(1_756_900_800), "2025-09-03T12:00:00+00:00");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00+00:00");
    }
}
