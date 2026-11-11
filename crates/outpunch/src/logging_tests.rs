use std::time::Duration;

use super::*;

#[test]
fn time_of_day_is_fixed_width_utc_with_microseconds() {
    let at = UNIX_EPOCH
        + Duration::from_secs(19_000 * 86_400 + 3 * 3600 + 4 * 60 + 5)
        + Duration::from_micros(42);
    assert_eq!(time_of_day(at), "03:04:05.000042Z");
}

#[test]
fn time_of_day_wraps_at_midnight() {
    let at = UNIX_EPOCH + Duration::from_secs(86_400 - 1) + Duration::from_micros(999_999);
    assert_eq!(time_of_day(at), "23:59:59.999999Z");
    assert_eq!(
        time_of_day(at + Duration::from_micros(1)),
        "00:00:00.000000Z"
    );
}
