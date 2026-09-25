#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtcTimestamp(i64);

impl UtcTimestamp {
    #[must_use]
    pub const fn from_unix_millis(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn unix_millis(self) -> i64 {
        self.0
    }

    /// Renders this timestamp as an RFC 3339 UTC string with millisecond
    /// precision (`"1970-01-01T00:00:00.000Z"`). Uses pure integer
    /// civil-calendar arithmetic rather than a date/time crate -- this
    /// workspace has none, and adding one is its own decision, not a detail
    /// of one caller (the managed projection schema needs this format).
    #[must_use]
    pub fn to_rfc3339_utc(self) -> String {
        let millis_of_day = self.0.rem_euclid(86_400_000);
        let days = (self.0 - millis_of_day) / 86_400_000;
        let (year, month, day) = civil_from_days(days);
        let hour = millis_of_day / 3_600_000;
        let minute = (millis_of_day / 60_000) % 60;
        let second = (millis_of_day / 1_000) % 60;
        let millis = millis_of_day % 1_000;
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }
}

/// Howard Hinnant's `civil_from_days`: converts a day count relative to the
/// Unix epoch (1970-01-01 = day 0) into a proleptic-Gregorian
/// `(year, month, day)`, correct for the algorithm's full domain
/// (`days` in `[i64::MIN/399, i64::MAX/399]` or so -- far beyond any real
/// timestamp this type stores).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = u32::try_from(day_of_year - (153 * month_prime + 2) / 5 + 1).unwrap_or_default(); // [1, 31]
    let month = u32::try_from(if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    })
    .unwrap_or_default(); // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

pub trait Clock {
    fn now(&self) -> UtcTimestamp;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unix_epoch_renders_as_the_canonical_zero_instant() {
        assert_eq!(
            UtcTimestamp::from_unix_millis(0).to_rfc3339_utc(),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn a_well_known_instant_renders_correctly() {
        // 2020-09-13T12:26:40.000Z, a widely-cited round Unix-seconds value.
        assert_eq!(
            UtcTimestamp::from_unix_millis(1_600_000_000_000).to_rfc3339_utc(),
            "2020-09-13T12:26:40.000Z"
        );
    }

    #[test]
    fn milliseconds_and_a_leap_day_render_correctly() {
        // 2024-02-29 is a leap day; this also exercises non-zero milliseconds.
        assert_eq!(
            UtcTimestamp::from_unix_millis(1_709_164_800_123).to_rfc3339_utc(),
            "2024-02-29T00:00:00.123Z"
        );
    }

    #[test]
    fn an_instant_before_the_epoch_renders_correctly() {
        // One millisecond before the epoch must roll back to the prior day,
        // not produce a negative or malformed time-of-day.
        assert_eq!(
            UtcTimestamp::from_unix_millis(-1).to_rfc3339_utc(),
            "1969-12-31T23:59:59.999Z"
        );
    }
}
