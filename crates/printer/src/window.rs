//! The daily time window autoprint is allowed to run in, in the Pi's local
//! time. Times are minutes since local midnight; the window is `[start, end)`
//! and wraps past midnight when `start > end` (e.g. 22:00-02:00).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: u32,
    pub end: u32,
}

pub const MINUTES_PER_DAY: u32 = 24 * 60;
pub const DEFAULT_START: u32 = 10 * 60;
pub const DEFAULT_END: u32 = 16 * 60;

impl Window {
    /// Both ends within a day, and not equal (an empty or whole-day window is
    /// expressed by disabling the window, not by `start == end`).
    pub fn is_valid(&self) -> bool {
        self.start < MINUTES_PER_DAY && self.end < MINUTES_PER_DAY && self.start != self.end
    }

    pub fn contains(&self, minute_of_day: u32) -> bool {
        if self.start < self.end {
            minute_of_day >= self.start && minute_of_day < self.end
        } else {
            minute_of_day >= self.start || minute_of_day < self.end
        }
    }
}

/// Seconds east of UTC that apply at `unix` in the system's local time zone.
pub fn local_utc_offset(unix: f64) -> i64 {
    let time = unix as libc::time_t;
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call; localtime_r is thread-safe.
    let ok = unsafe { !libc::localtime_r(&time, &mut local).is_null() };
    if ok {
        local.tm_gmtoff as i64
    } else {
        0
    }
}

fn minute_of_day(unix: f64, offset: i64) -> u32 {
    let local = unix as i64 + offset;
    (local.rem_euclid(86_400) / 60) as u32
}

/// The earliest time at or after `candidate` that lies inside `window`:
/// `candidate` itself when it already does, otherwise the next window start.
/// `utc_offset` maps a Unix time to its local UTC offset (injected so tests
/// need no time zone and DST changes can be exercised).
pub fn next_allowed(candidate: f64, window: Window, utc_offset: impl Fn(f64) -> i64) -> f64 {
    let offset = utc_offset(candidate);
    let minute = minute_of_day(candidate, offset);
    if window.contains(minute) {
        return candidate;
    }
    let local_seconds = (candidate as i64 + offset).rem_euclid(86_400);
    let wait_minutes = (window.start + MINUTES_PER_DAY - minute) % MINUTES_PER_DAY;
    let mut target = candidate - (local_seconds % 60) as f64 + (wait_minutes * 60) as f64;
    // Crossing a DST change: the wall-clock start is what matters.
    target -= (utc_offset(target) - offset) as f64;
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Window = Window { start: DEFAULT_START, end: DEFAULT_END };
    // 2026-10-02 00:00:00 UTC.
    const MIDNIGHT: f64 = 1_790_899_200.0;
    fn utc(_: f64) -> i64 {
        0
    }
    fn at(hour: u32, minute: u32) -> f64 {
        MIDNIGHT + (hour * 3600 + minute * 60) as f64
    }

    #[test]
    fn a_time_inside_the_window_is_kept() {
        assert_eq!(next_allowed(at(12, 30), DAY, utc), at(12, 30));
        assert_eq!(next_allowed(at(10, 0), DAY, utc), at(10, 0));
    }

    #[test]
    fn the_end_of_the_window_is_exclusive() {
        assert_eq!(next_allowed(at(16, 0), DAY, utc), at(10, 0) + 86_400.0);
    }

    #[test]
    fn before_the_window_waits_for_todays_start() {
        assert_eq!(next_allowed(at(7, 15) + 20.0, DAY, utc), at(10, 0));
    }

    #[test]
    fn after_the_window_waits_for_tomorrows_start() {
        assert_eq!(next_allowed(at(22, 5), DAY, utc), at(10, 0) + 86_400.0);
    }

    #[test]
    fn a_window_may_wrap_past_midnight() {
        let night = Window { start: 22 * 60, end: 2 * 60 };
        assert!(night.contains(23 * 60) && night.contains(60) && !night.contains(12 * 60));
        assert_eq!(next_allowed(at(23, 0), night, utc), at(23, 0));
        assert_eq!(next_allowed(at(12, 0), night, utc), at(22, 0));
    }

    #[test]
    fn local_offset_shifts_the_window() {
        // UTC-7: 10:00-16:00 local is 17:00-23:00 UTC.
        let pdt = |_: f64| -7 * 3600;
        assert_eq!(next_allowed(at(12, 0), DAY, pdt), at(17, 0));
        assert_eq!(next_allowed(at(18, 0), DAY, pdt), at(18, 0));
    }

    #[test]
    fn a_dst_change_before_the_next_start_keeps_the_wall_clock_time() {
        // Offset is -7h until t0 + 3h, then -8h: the next 10:00 local lands
        // an hour later in UTC than a fixed offset would give.
        let switch = at(3, 0);
        let offset = move |t: f64| if t < switch { -7 * 3600 } else { -8 * 3600 };
        // 02:00 UTC is 19:00 local (previous evening, outside the window).
        assert_eq!(next_allowed(at(2, 0), DAY, offset), at(18, 0));
    }

    #[test]
    fn validity() {
        assert!(DAY.is_valid());
        assert!(!Window { start: 600, end: 600 }.is_valid());
        assert!(!Window { start: 600, end: 1440 }.is_valid());
    }
}
