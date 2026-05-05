//! Time newtypes – [`SecondOfDay`] (a timestamp), [`Duration`] (a length of
//! time), and [`Transfers`] (a transfer cap). Distinct types so the algorithm
//! signatures can express which kind they expect; arithmetic across the two
//! time types is saturating.

use std::fmt;

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

impl fmt::Display for SecondOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
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
