//! Time newtypes – [`SecondOfDay`] (a timestamp), [`Duration`] (a length of
//! time), and [`Transfers`] (a transfer cap). Distinct types so the algorithm
//! signatures can express which kind they expect; arithmetic across the two
//! time types is saturating.
//!
//! Conversions to and from [`jiff::civil::Time`] and [`jiff::SignedDuration`]
//! are provided so callers already in jiff land don't have to disassemble
//! values manually.

use std::fmt;
use std::str::FromStr;

use jiff::SignedDuration;
use jiff::civil::Time;

/// A point in time, in seconds since midnight on the timetable's
/// service date. Wraps a `u32` – the day is 86,400 seconds; `u32`
/// covers feed quirks like trips encoded past 24h with room to spare.
///
/// SecondOfDay is a *timestamp*. A *length* of time – walk-time offset,
/// transfer time, dwell time – is a [`Duration`], a distinct type.
/// The trait surface uses both consistently so they can't be
/// silently confused.
///
/// Construct via [`SecondOfDay::ZERO`], [`SecondOfDay::from_secs`], [`SecondOfDay::hms`], or
/// the public-field constructor `SecondOfDay(n)`. Extract via
/// [`SecondOfDay::as_secs`] / [`SecondOfDay::as_hms`]. Arithmetic with [`Duration`]
/// is saturating: `SecondOfDay::MAX + Duration::MAX` stays at `SecondOfDay::MAX`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SecondOfDay(pub u32);

impl SecondOfDay {
    /// Midnight (0 seconds since the start of the service day).
    pub const ZERO: SecondOfDay = SecondOfDay(0);
    /// Sentinel for "unreached". The algorithm uses this internally
    /// for empty `(round, stop)` cells.
    pub const MAX: SecondOfDay = SecondOfDay(u32::MAX);

    /// Construct from a raw seconds-since-midnight count.
    pub const fn from_secs(s: u32) -> Self {
        SecondOfDay(s)
    }

    /// Construct from `(hours, minutes, seconds)`.
    pub const fn hms(h: u32, m: u32, s: u32) -> Self {
        SecondOfDay(h * 3600 + m * 60 + s)
    }

    /// The underlying `u32` – seconds since midnight.
    pub const fn as_secs(self) -> u32 {
        self.0
    }

    /// `(hours, minutes, seconds)` decomposition.
    pub const fn as_hms(self) -> (u32, u32, u32) {
        (self.0 / 3600, (self.0 / 60) % 60, self.0 % 60)
    }

    /// Iterator of departure times from `start` (inclusive) to `end`
    /// (exclusive) at `step`-second intervals. Convenience for the
    /// common range-query input pattern:
    ///
    /// ```
    /// # use vulture::SecondOfDay;
    /// let deps: Vec<_> = SecondOfDay::every(
    ///     SecondOfDay::hms(17, 0, 0),
    ///     SecondOfDay::hms(17, 5, 0),
    ///     60,
    /// ).collect();
    /// assert_eq!(deps.len(), 5);
    /// assert_eq!(deps[0], SecondOfDay::hms(17, 0, 0));
    /// assert_eq!(deps[4], SecondOfDay::hms(17, 4, 0));
    /// ```
    ///
    /// Pass directly to
    /// [`Query::depart_in_window`](crate::Query::depart_in_window).
    pub fn every(
        start: SecondOfDay,
        end: SecondOfDay,
        step: u32,
    ) -> impl Iterator<Item = SecondOfDay> + Clone {
        (start.0..end.0)
            .step_by(step.max(1) as usize)
            .map(SecondOfDay)
    }
}

impl From<u32> for SecondOfDay {
    fn from(s: u32) -> Self {
        SecondOfDay(s)
    }
}

impl From<Time> for SecondOfDay {
    /// `(t.hour() * 3600 + t.minute() * 60 + t.second())`. Subseconds
    /// are truncated; vulture works in whole seconds.
    fn from(t: Time) -> Self {
        SecondOfDay((t.hour() as u32) * 3600 + (t.minute() as u32) * 60 + (t.second() as u32))
    }
}

impl TryFrom<SecondOfDay> for Time {
    type Error = jiff::Error;
    /// Fails with a [`jiff::Error`] when `s.0 >= 86_400`. GTFS feeds
    /// regularly use after-midnight values like 25:30:00 to encode trips
    /// that started on the previous service day; those don't fit in a
    /// `civil::Time`.
    fn try_from(s: SecondOfDay) -> Result<Self, Self::Error> {
        let (h, m, sec) = s.as_hms();
        // jiff::civil::Time accepts 0..=23 hours, so this fails for
        // anything from 24:00:00 onwards. The hour-out-of-range error
        // bubbles through unchanged.
        Time::new(h as i8, m as i8, sec as i8, 0)
    }
}

impl fmt::Display for SecondOfDay {
    /// `HH:MM:SS`. Hours can exceed 23 for after-midnight times often
    /// seen in GTFS feeds (e.g. `25:30:00`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m, s) = self.as_hms();
        write!(f, "{h:02}:{m:02}:{s:02}")
    }
}

impl FromStr for SecondOfDay {
    type Err = ParseSecondOfDayError;
    /// Parse `HH:MM:SS` or `HH:MM` (seconds default to 0). Hours have no
    /// upper bound (GTFS allows values like 25:30:00); minutes and seconds
    /// must be 0–59.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let h_str = parts
            .next()
            .ok_or_else(|| ParseSecondOfDayError::BadFormat(s.to_string()))?;
        let m_str = parts
            .next()
            .ok_or_else(|| ParseSecondOfDayError::BadFormat(s.to_string()))?;
        let sec_str = parts.next();
        if parts.next().is_some() {
            return Err(ParseSecondOfDayError::BadFormat(s.to_string()));
        }
        let h: u32 = h_str
            .parse()
            .map_err(|_| ParseSecondOfDayError::BadField(h_str.to_string()))?;
        let m: u32 = m_str
            .parse()
            .map_err(|_| ParseSecondOfDayError::BadField(m_str.to_string()))?;
        if m > 59 {
            return Err(ParseSecondOfDayError::OutOfRange(m));
        }
        let sec: u32 = match sec_str {
            None => 0,
            Some(s) => {
                let v: u32 = s
                    .parse()
                    .map_err(|_| ParseSecondOfDayError::BadField(s.to_string()))?;
                if v > 59 {
                    return Err(ParseSecondOfDayError::OutOfRange(v));
                }
                v
            }
        };
        Ok(SecondOfDay::hms(h, m, sec))
    }
}

/// Errors produced when parsing a string into a [`SecondOfDay`] via [`FromStr`].
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum ParseSecondOfDayError {
    /// The input did not match `HH:MM` or `HH:MM:SS`.
    #[error("expected HH:MM[:SS], got `{0}`")]
    BadFormat(String),
    /// One of the colon-separated components was not a base-10 integer.
    #[error("invalid time field: `{0}`")]
    BadField(String),
    /// Minutes or seconds outside `0..=59`.
    #[error("time field out of range: {0} (must be 0..=59)")]
    OutOfRange(u32),
}

impl std::ops::Add<Duration> for SecondOfDay {
    /// `SecondOfDay + Duration` advances the timestamp; saturating on overflow.
    type Output = SecondOfDay;
    fn add(self, d: Duration) -> SecondOfDay {
        SecondOfDay(self.0.saturating_add(d.0))
    }
}

impl std::ops::Sub<SecondOfDay> for SecondOfDay {
    /// `SecondOfDay - SecondOfDay` is the [`Duration`] between them, saturating at
    /// [`Duration::ZERO`] if `self < other`.
    type Output = Duration;
    fn sub(self, other: SecondOfDay) -> Duration {
        Duration(self.0.saturating_sub(other.0))
    }
}

impl std::ops::Sub<Duration> for SecondOfDay {
    /// `SecondOfDay - Duration` rewinds the timestamp; saturating at
    /// [`SecondOfDay::ZERO`] on underflow.
    type Output = SecondOfDay;
    fn sub(self, d: Duration) -> SecondOfDay {
        SecondOfDay(self.0.saturating_sub(d.0))
    }
}

/// A length of time, in seconds. Distinct from [`SecondOfDay`] (a point in
/// time) so the algorithm signatures can express which kind they
/// expect: a walk-time offset, a transfer time, and a dwell time
/// are all `Duration`; an arrival time and a departure time are
/// both `SecondOfDay`.
///
/// Constructed via [`Duration::ZERO`], [`Duration::from_secs`], or
/// the public-field constructor `Duration(n)`. Arithmetic is
/// saturating: `Duration::MAX + Duration` stays at `Duration::MAX`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Duration(pub u32);

impl Duration {
    /// Zero seconds.
    pub const ZERO: Duration = Duration(0);
    /// Maximum representable duration. Used internally as the
    /// "no walk recorded" sentinel for some algorithm paths.
    pub const MAX: Duration = Duration(u32::MAX);

    /// Construct from a raw seconds count.
    pub const fn from_secs(s: u32) -> Self {
        Duration(s)
    }

    /// The underlying `u32` – seconds.
    pub const fn as_secs(self) -> u32 {
        self.0
    }
}

impl From<u32> for Duration {
    fn from(s: u32) -> Self {
        Duration(s)
    }
}

impl From<Duration> for SignedDuration {
    /// Total, lossless: vulture's [`Duration`] is `u32` seconds, jiff's
    /// [`SignedDuration`] is `i64` seconds + `i32` subsec nanos.
    fn from(d: Duration) -> Self {
        SignedDuration::from_secs(d.0 as i64)
    }
}

impl TryFrom<SignedDuration> for Duration {
    type Error = TryFromSignedDurationError;
    /// Fails when the value is negative or its whole-second part exceeds
    /// `u32::MAX`. Subseconds are truncated; vulture works in whole seconds.
    fn try_from(d: SignedDuration) -> Result<Self, Self::Error> {
        let secs = d.as_secs();
        if secs < 0 {
            return Err(TryFromSignedDurationError::Negative);
        }
        u32::try_from(secs)
            .map(Duration)
            .map_err(|_| TryFromSignedDurationError::Overflow)
    }
}

/// Errors produced when narrowing a [`jiff::SignedDuration`] into a
/// vulture [`Duration`].
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum TryFromSignedDurationError {
    /// The source duration was negative; [`Duration`] is unsigned.
    #[error("negative duration cannot be represented as vulture::Duration")]
    Negative,
    /// The source duration's whole-second component overflowed `u32`.
    #[error("duration overflows u32 seconds")]
    Overflow,
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Add<Duration> for Duration {
    type Output = Duration;
    fn add(self, o: Duration) -> Duration {
        Duration(self.0.saturating_add(o.0))
    }
}

/// User-facing transfer cap. The algorithm explores rounds 0
/// through `transfers` inclusive, so `Transfers(10)` lets a journey
/// involve up to 10 boardings. `u8` is plenty – practical journey
/// queries cap at single digits.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Transfers(pub u8);

impl Transfers {
    /// No transfers allowed (only direct journeys).
    pub const ZERO: Transfers = Transfers(0);
    /// Maximum representable transfer cap (255).
    pub const MAX: Transfers = Transfers(u8::MAX);

    /// Construct from a raw `u8`.
    pub const fn new(n: u8) -> Self {
        Transfers(n)
    }

    /// The underlying `u8`.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl From<u8> for Transfers {
    fn from(n: u8) -> Self {
        Transfers(n)
    }
}

impl fmt::Display for Transfers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_of_day_display_is_hms() {
        assert_eq!(SecondOfDay::hms(9, 30, 0).to_string(), "09:30:00");
        assert_eq!(SecondOfDay::hms(0, 0, 0).to_string(), "00:00:00");
        assert_eq!(SecondOfDay::hms(25, 30, 5).to_string(), "25:30:05");
    }

    #[test]
    fn second_of_day_from_str_round_trips() {
        for s in ["00:00:00", "09:30:00", "23:59:59", "25:30:05"] {
            let parsed: SecondOfDay = s.parse().expect("parses");
            assert_eq!(parsed.to_string(), s, "round-trip {s}");
        }
    }

    #[test]
    fn second_of_day_from_str_accepts_hh_mm() {
        let s: SecondOfDay = "09:30".parse().expect("parses");
        assert_eq!(s, SecondOfDay::hms(9, 30, 0));
    }

    #[test]
    fn second_of_day_from_str_rejects_garbage() {
        assert!(matches!(
            "9:30:".parse::<SecondOfDay>(),
            Err(ParseSecondOfDayError::BadField(_))
        ));
        assert!(matches!(
            "abc".parse::<SecondOfDay>(),
            Err(ParseSecondOfDayError::BadFormat(_))
        ));
        assert!(matches!(
            "09:30:00:00".parse::<SecondOfDay>(),
            Err(ParseSecondOfDayError::BadFormat(_))
        ));
        assert!(matches!(
            "09:60:00".parse::<SecondOfDay>(),
            Err(ParseSecondOfDayError::OutOfRange(60))
        ));
        assert!(matches!(
            "09:30:99".parse::<SecondOfDay>(),
            Err(ParseSecondOfDayError::OutOfRange(99))
        ));
    }

    #[test]
    fn second_of_day_jiff_time_round_trip() {
        let t = Time::new(9, 30, 5, 0).unwrap();
        let s: SecondOfDay = t.into();
        assert_eq!(s, SecondOfDay::hms(9, 30, 5));
        let back: Time = s.try_into().unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn jiff_time_subseconds_truncated() {
        let t = Time::new(9, 30, 5, 999_999_999).unwrap();
        let s: SecondOfDay = t.into();
        assert_eq!(s, SecondOfDay::hms(9, 30, 5));
    }

    #[test]
    fn second_of_day_after_midnight_is_unrepresentable_as_jiff_time() {
        let s = SecondOfDay::hms(25, 30, 0);
        assert!(Time::try_from(s).is_err());
    }

    #[test]
    fn duration_to_signed_duration_total() {
        let d = Duration::from_secs(60);
        let sd: SignedDuration = d.into();
        assert_eq!(sd.as_secs(), 60);
    }

    #[test]
    fn signed_duration_to_duration_truncates_subseconds() {
        let sd = SignedDuration::new(60, 999_999_999);
        let d: Duration = sd.try_into().unwrap();
        assert_eq!(d, Duration::from_secs(60));
    }

    #[test]
    fn signed_duration_negative_rejected() {
        let sd = SignedDuration::from_secs(-1);
        assert_eq!(
            Duration::try_from(sd),
            Err(TryFromSignedDurationError::Negative)
        );
    }

    #[test]
    fn signed_duration_overflow_rejected() {
        let sd = SignedDuration::from_secs(i64::from(u32::MAX) + 1);
        assert_eq!(
            Duration::try_from(sd),
            Err(TryFromSignedDurationError::Overflow)
        );
    }
}
