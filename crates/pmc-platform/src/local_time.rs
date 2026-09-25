//! Calendar dates as a person reads them, in a named IANA time zone.

use std::str::FromStr;

use jiff::tz::TimeZone;
use jiff::Timestamp;

/// The `YYYY-MM-DD` date of an RFC 3339 instant in `timezone`, or `None` if
/// either cannot be read. Used for the restore's typed confirmation (DG3
/// restore amendment §3.5), rendered in the person's current time zone.
#[must_use]
pub fn local_calendar_date(rfc3339: &str, timezone: &str) -> Option<String> {
    let instant = Timestamp::from_str(rfc3339).ok()?;
    let zone = TimeZone::get(timezone).ok()?;
    Some(instant.to_zoned(zone).date().to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_utc_instant_falls_on_the_local_calendar_date() {
        // 2026-09-21 20:00 UTC is already 2026-09-22 in Taipei.
        assert_eq!(
            local_calendar_date("2026-09-21T20:00:00.000Z", "Asia/Taipei").unwrap(),
            "2026-09-22"
        );
        assert_eq!(
            local_calendar_date("2026-09-21T20:00:00.000Z", "America/New_York").unwrap(),
            "2026-09-21"
        );
        assert!(local_calendar_date("not a time", "Asia/Taipei").is_none());
        assert!(local_calendar_date("2026-09-21T20:00:00.000Z", "Nowhere/City").is_none());
    }
}
