//! The [`Label`] trait – what the algorithm carries at each `(round, stop)`
//! cell – plus the default single-criterion implementation [`ArrivalTime`].
//!
//! For multi-criterion impls (trade-off queries) see [`crate::labels`].

use crate::time::Duration;
use crate::time::SecondOfDay;

/// A label attached to a `(round, stop)` cell during the RAPTOR scan.
///
/// **Most users can ignore this trait.** [`Timetable::query`](crate::Timetable::query) uses
/// [`ArrivalTime`] (single-criterion: minimise arrival time, fewest
/// transfers), which is what the original RAPTOR paper describes and
/// what almost every routing application wants.
///
/// The trait exists so the algorithm can be reused for *multi-criterion*
/// routing – minimising arrival time *and* something else at the same
/// time, returning a Pareto front of trade-offs. Reach for it when a
/// single "best" answer is the wrong shape: e.g. an accessibility-aware
/// query that should also report the route with less walking, even if
/// it arrives slightly later. The bundled [`labels::ArrivalAndWalk`](crate::labels::ArrivalAndWalk)
/// is one such impl; see also [`Timetable::query_with_label`](crate::Timetable::query_with_label) for the
/// builder entry point.
///
/// The algorithm maintains a Pareto front (a *bag* of mutually
/// non-dominated labels) per `(round, stop)`, so multi-criterion impls
/// produce real Pareto fronts at the targets rather than a single
/// tiebroken label. Single-criterion `ArrivalTime` bags stay size 1,
/// with no behaviour change versus a non-bag implementation.
pub trait Label: Copy + std::fmt::Debug {
    /// The "unreached" sentinel. The algorithm initialises every
    /// `(round, stop)` cell to this value before seeding origins.
    const UNREACHED: Self;

    /// Initial label at an origin stop, given the user's departure time.
    fn from_departure(at: SecondOfDay) -> Self;

    /// New label produced by alighting from a trip at this stop with
    /// the given arrival time. `self` is the label at the boarding
    /// stop. For multi-criterion impls, components like accumulated
    /// walking time inherit from `self`.
    fn extend_by_trip(self, arrival: SecondOfDay) -> Self;

    /// New label after walking a footpath of duration `walk_time`.
    fn extend_by_footpath(self, walk_time: Duration) -> Self;

    /// `self` weakly dominates `other` (every criterion of `self` is
    /// at most the corresponding criterion of `other`). The default
    /// implementation uses [`Label::arrival`], which is correct for
    /// single-criterion impls.
    fn dominates(&self, other: &Self) -> bool {
        self.arrival() <= other.arrival()
    }

    /// Effective arrival time at the labelled stop. Used by the
    /// algorithm for target-threshold comparisons and by [`Journey`](crate::Journey)
    /// output. Always returns [`SecondOfDay::MAX`] for [`Label::UNREACHED`].
    fn arrival(&self) -> SecondOfDay;
}

/// Single-criterion label = arrival time at a stop. Default `L`
/// throughout the algorithm. Constructing from a `SecondOfDay` is direct;
/// extracting back is `arrival()`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrivalTime(pub SecondOfDay);

impl Label for ArrivalTime {
    const UNREACHED: Self = ArrivalTime(SecondOfDay::MAX);

    #[inline]
    fn from_departure(at: SecondOfDay) -> Self {
        ArrivalTime(at)
    }

    #[inline]
    fn extend_by_trip(self, arrival: SecondOfDay) -> Self {
        ArrivalTime(arrival)
    }

    #[inline]
    fn extend_by_footpath(self, walk_time: Duration) -> Self {
        ArrivalTime(self.0 + walk_time)
    }

    #[inline]
    fn arrival(&self) -> SecondOfDay {
        self.0
    }
}
