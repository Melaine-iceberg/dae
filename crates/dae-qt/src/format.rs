//! Delegate text, computed on the model side.
//!
//! QML gets finished strings rather than raw numbers, so a visible row costs
//! one `data()` call per role instead of a JavaScript format in every delegate.

use std::sync::atomic::{AtomicBool, Ordering};

/// How byte counts are labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SizeUnits {
    /// Powers of 1024, labelled `KiB`/`MiB`. This is what the divisor does, and
    /// an explorer that divides by 1024 while printing "KB" is lying by 2.4%.
    #[default]
    Binary,
    /// Powers of 1000, labelled `KB`/`MB` — the units drive vendors, phone
    /// storage screens and most upload forms use.
    Decimal,
}

/// Process-wide because `size()` is called from `data()` on every visible row;
/// threading a units argument through the models would add a branch to the
/// hottest text path for a value that changes once a session.
static DECIMAL_UNITS: AtomicBool = AtomicBool::new(false);

pub fn set_size_units(units: SizeUnits) {
    DECIMAL_UNITS.store(units == SizeUnits::Decimal, Ordering::Relaxed);
}

pub fn size_units() -> SizeUnits {
    if DECIMAL_UNITS.load(Ordering::Relaxed) {
        SizeUnits::Decimal
    } else {
        SizeUnits::Binary
    }
}

/// Human-readable byte count in the process-wide units.
pub fn size(bytes: u64) -> String {
    size_with(bytes, size_units())
}

/// Pure formatter, for tests and for callers that must not follow the setting.
pub fn size_with(bytes: u64, units: SizeUnits) -> String {
    let (base, labels): (f64, [&str; 5]) = match units {
        SizeUnits::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB"]),
        SizeUnits::Decimal => (1000.0, ["B", "KB", "MB", "GB", "TB"]),
    };
    if bytes < base as u64 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= base && unit + 1 < labels.len() {
        value /= base;
        unit += 1;
    }
    format!("{value:.1} {}", labels[unit])
}

/// `YYYY-MM-DD HH:MM` in UTC. Calendar math is inlined (Howard Hinnant's
/// `civil_from_days`) so the shell does not pull a date crate in just to label
/// a row.
pub fn timestamp(epoch_millis: u64) -> String {
    if epoch_millis == 0 {
        return "-".to_owned();
    }
    let seconds = (epoch_millis / 1000) as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// A compact label for an overview row: "今天", "昨天", "N 天前", or the
/// calendar date once it falls outside the last week. Stamps are milliseconds
/// since the Unix epoch, matching `recents.json`.
pub fn day_label(epoch_millis: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    day_label_at(now, epoch_millis)
}

fn day_label_at(now_millis: u64, epoch_millis: u64) -> String {
    if epoch_millis == 0 {
        return "-".to_owned();
    }
    let now_days = (now_millis / 86_400_000) as i64;
    let then_days = (epoch_millis / 86_400_000) as i64;
    match now_days - then_days {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        days @ 2..=6 => format!("{days} days ago"),
        _ => timestamp(epoch_millis)
            .split(' ')
            .next()
            .unwrap_or("-")
            .to_owned(),
    }
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_known_epoch() {
        // 2026-10-25T00:00:00Z
        assert_eq!(timestamp(1_792_886_400_000), "2026-10-25 00:00");
    }

    #[test]
    fn formats_binary_sizes_with_binary_labels() {
        assert_eq!(size_with(0, SizeUnits::Binary), "0 B");
        assert_eq!(size_with(1023, SizeUnits::Binary), "1023 B");
        assert_eq!(size_with(1024, SizeUnits::Binary), "1.0 KiB");
        assert_eq!(size_with(1024 * 1024 * 3 / 2, SizeUnits::Binary), "1.5 MiB");
    }

    #[test]
    fn formats_decimal_sizes_with_si_labels() {
        assert_eq!(size_with(999, SizeUnits::Decimal), "999 B");
        assert_eq!(size_with(1000, SizeUnits::Decimal), "1.0 KB");
        // 1 GiB is 1.07 GB: the point of the switch is that the number moves.
        assert_eq!(size_with(1024 * 1024 * 1024, SizeUnits::Decimal), "1.1 GB");
    }

    #[test]
    fn the_process_wide_units_follow_the_setter() {
        set_size_units(SizeUnits::Decimal);
        assert_eq!(size_units(), SizeUnits::Decimal);
        assert_eq!(size(1000), "1.0 KB");
        set_size_units(SizeUnits::Binary);
        assert_eq!(size_units(), SizeUnits::Binary);
        assert_eq!(size(1024), "1.0 KiB");
    }

    #[test]
    fn labels_recent_days_and_falls_back_to_a_date() {
        let now = 1_792_929_600_000; // 2026-10-25T12:00:00Z
        let day = 86_400_000;

        assert_eq!(day_label_at(now, now - 10 * 3_600_000), "Today");
        assert_eq!(day_label_at(now, now - day), "Yesterday");
        assert_eq!(day_label_at(now, now - 3 * day), "3 days ago");
        assert_eq!(day_label_at(now, now - 30 * day), "2026-09-25");
        assert_eq!(day_label_at(now, 0), "-");
    }
}
