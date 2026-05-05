//! Dense `u32` indices for stops, routes, and trips. Adapters intern from
//! external IDs (e.g. GTFS string IDs) to these at construction time, so the
//! algorithm can use array indexing rather than hash-map lookups in its hot
//! loop.

use std::fmt;

/// Dense index of a stop within a [`Timetable`](crate::Timetable). Indices are in `0..tt.n_stops()`.
///
/// Constructed by adapters at timetable-construction time. Display formats as
/// the bare `u32`; round-trip via [`StopIdx::get`] / [`From<u32>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StopIdx(u32);

impl StopIdx {
    /// Construct from a raw `u32`. The caller is responsible for the value
    /// being a valid index for the timetable in question.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }
    /// The underlying `u32`.
    pub const fn get(self) -> u32 {
        self.0
    }
    #[inline]
    pub(crate) fn idx(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for StopIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u32> for StopIdx {
    fn from(n: u32) -> Self {
        Self(n)
    }
}
impl From<StopIdx> for u32 {
    fn from(s: StopIdx) -> Self {
        s.0
    }
}

/// Dense index of a route within a [`Timetable`](crate::Timetable). Indices are in `0..tt.n_routes()`.
///
/// In the GTFS adapter, a single GTFS `route_id` may map to multiple
/// `RouteIdx`s – one per equivalence class of trips with identical stop
/// sequences and pairwise non-overtaking schedules. See
/// [`gtfs::GtfsTimetable`](crate::gtfs::GtfsTimetable) for the splitting rules and lookup APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteIdx(u32);

impl RouteIdx {
    /// Construct from a raw `u32`. The caller is responsible for the value
    /// being a valid index for the timetable in question.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }
    /// The underlying `u32`.
    pub const fn get(self) -> u32 {
        self.0
    }
    #[inline]
    pub(crate) fn idx(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for RouteIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u32> for RouteIdx {
    fn from(n: u32) -> Self {
        Self(n)
    }
}
impl From<RouteIdx> for u32 {
    fn from(r: RouteIdx) -> Self {
        r.0
    }
}

/// Dense index of a trip within a [`Timetable`](crate::Timetable). Indices are in `0..n_trips`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TripIdx(u32);

impl TripIdx {
    /// Construct from a raw `u32`. The caller is responsible for the value
    /// being a valid index for the timetable in question.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }
    /// The underlying `u32`.
    pub const fn get(self) -> u32 {
        self.0
    }
    #[inline]
    pub(crate) fn idx(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TripIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u32> for TripIdx {
    fn from(n: u32) -> Self {
        Self(n)
    }
}
impl From<TripIdx> for u32 {
    fn from(t: TripIdx) -> Self {
        t.0
    }
}
