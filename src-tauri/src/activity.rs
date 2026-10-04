//! The app's own log of what it did (`<app data>/activity.log`, owner-only): node starts and stops, trades placed,
//! phones paired. No words, keys or full addresses go in it. It is kept under 512 KiB (the older half is dropped).

use std::io::Write;
use std::path::Path;

const MAX: u64 = 512 * 1024;

pub fn note(dir: &Path, msg: &str) {
    let p = dir.join("activity.log");
    if std::fs::metadata(&p).map(|m| m.len() > MAX).unwrap_or(false) {
        if let Ok(b) = std::fs::read(&p) {
            let _ = crate::files::write_private(&p, newer_half(&b));
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    if let Ok(mut f) = o.open(&p) {
        let line = msg.replace('\n', " ");
        let _ = writeln!(f, "{} {}", now_utc(), line);
    }
}

/// The newer half of a log: from the first line that starts after the middle. Works on bytes, so a character split by
/// the middle can't panic (security review M2).
fn newer_half(b: &[u8]) -> &[u8] {
    let mid = b.len() / 2;
    match b[mid..].iter().position(|c| *c == b'\n') {
        Some(i) => &b[mid + i + 1..],
        None => &[],
    }
}

/// The time as "2026-10-04T21:30:00Z".
pub fn now_utc() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (s / 86400, s % 86400);
    let (y, m, d) = civil(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to a date (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// An address shown in the log: its first 6 characters.
pub fn mask(addr: &str) -> String {
    format!("{}…", addr.chars().take(6).collect::<String>())
}

#[cfg(test)]
mod tests {
    #[test]
    fn trimming_never_splits_a_character() {
        for n in 0..64 {
            let line = format!("{} withdrawal to eCash bc1qxy…\n", "é".repeat(n % 7));
            let log = line.repeat(50 + n);
            let half = super::newer_half(log.as_bytes());
            assert!(std::str::from_utf8(half).is_ok());
            assert!(half.is_empty() || half.starts_with(line.as_bytes()));
        }
    }

    #[test]
    fn dates() {
        assert_eq!(super::civil(0), (1970, 1, 1));
        assert_eq!(super::civil(20730), (2026, 10, 4));
    }
}
