//! The [`Label`] trait – what the algorithm carries at each `(round, stop)`
//! cell – plus the default single-criterion implementation [`ArrivalTime`].
//!
//! For multi-criterion impls (trade-off queries) see [`crate::labels`].

use crate::ids::{RouteIdx, StopIdx, TripIdx};
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
/// single "best" answer is the wrong shape: e.g. a fare-aware query
/// that should also report the cheapest journey alongside the fastest.
/// The bundled
/// [`ArrivalAndWalk`](crate::labels::ArrivalAndWalk) (arrival vs.
/// walking time) and
/// [`ArrivalAndFare`](crate::labels::ArrivalAndFare) (arrival vs.
/// accumulated fare from a route → fare table threaded via [`Label::Ctx`])
/// are worked examples; see [`Timetable::query_with_label`](crate::Timetable::query_with_label)
/// for the builder entry point and [`Query::with_context`](crate::Query::with_context)
/// for supplying lookup tables.
///
/// The algorithm maintains a Pareto front (a *bag* of mutually
/// non-dominated labels) per `(round, stop)`, so multi-criterion impls
/// produce real Pareto fronts at the targets rather than a single
/// tiebroken label. Single-criterion `ArrivalTime` bags stay size 1,
/// with no behaviour change versus a non-bag implementation.
pub trait Label: Copy + std::fmt::Debug {
    /// User-supplied sidecar data the algorithm threads into every
    /// `extend_*` / `from_departure` call. Use this to carry tables
    /// the label needs to evaluate per-trip / per-footpath criteria
    /// (a fare table keyed on [`RouteIdx`], a stop → zone map, an
    /// agency → preference rank). The label itself stays `Copy` and
    /// cheap to duplicate; the (typically heavy) lookup data lives
    /// in `Ctx` and is borrowed immutably.
    ///
    /// `Ctx` must implement [`Default`] so [`crate::Timetable::query`]
    /// /  [`crate::Timetable::query_with_label`] can construct a
    /// query without forcing every caller to provide context for
    /// labels that don't need any (e.g. [`ArrivalTime`] uses
    /// `Ctx = ()`). Override the default with
    /// [`crate::Query::with_context`].
    type Ctx: Default;

    /// The "unreached" sentinel. The algorithm initialises every
    /// `(round, stop)` cell to this value before seeding origins.
    const UNREACHED: Self;

    /// Initial label at an origin stop, given the user's departure time.
    fn from_departure(ctx: &Self::Ctx, at: SecondOfDay) -> Self;

    /// New label produced by alighting from a trip at this stop with
    /// the given arrival time. `self` is the label at the boarding
    /// stop. The algorithm passes the surrounding trip context so
    /// criteria like fare-per-trip can be evaluated:
    ///
    /// - `trip` / `route` — identifiers for fare-table or
    ///   per-route-rank lookups via `ctx`.
    /// - `board_stop` / `board_pos` — where the rider boarded this
    ///   trip on the route (position is the index into the route's
    ///   stop sequence).
    /// - `alight_stop` / `alight_pos` — where the rider is alighting.
    /// - `arrival` — the trip's arrival time at `alight_pos`.
    ///
    /// For multi-criterion impls, components like accumulated
    /// walking time inherit from `self`; per-trip criteria pull from
    /// `ctx` keyed on the identifiers above.
    #[allow(clippy::too_many_arguments)]
    fn extend_by_trip(
        self,
        ctx: &Self::Ctx,
        trip: TripIdx,
        route: RouteIdx,
        board_stop: StopIdx,
        board_pos: u32,
        alight_stop: StopIdx,
        alight_pos: u32,
        arrival: SecondOfDay,
    ) -> Self;

    /// New label after walking a footpath of duration `walk_time`
    /// from `from_stop` to `to_stop`. `ctx` carries any walk-criterion
    /// tables (a stop → zone map for cross-zone surcharges, etc.).
    fn extend_by_footpath(
        self,
        ctx: &Self::Ctx,
        from_stop: StopIdx,
        to_stop: StopIdx,
        walk_time: Duration,
    ) -> Self;

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
    type Ctx = ();
    const UNREACHED: Self = ArrivalTime(SecondOfDay::MAX);

    #[inline]
    fn from_departure(_ctx: &Self::Ctx, at: SecondOfDay) -> Self {
        ArrivalTime(at)
    }

    #[inline]
    fn extend_by_trip(
        self,
        _ctx: &Self::Ctx,
        _trip: TripIdx,
        _route: RouteIdx,
        _board_stop: StopIdx,
        _board_pos: u32,
        _alight_stop: StopIdx,
        _alight_pos: u32,
        arrival: SecondOfDay,
    ) -> Self {
        ArrivalTime(arrival)
    }

    #[inline]
    fn extend_by_footpath(
        self,
        _ctx: &Self::Ctx,
        _from_stop: StopIdx,
        _to_stop: StopIdx,
        walk_time: Duration,
    ) -> Self {
        ArrivalTime(self.0 + walk_time)
    }

    #[inline]
    fn arrival(&self) -> SecondOfDay {
        self.0
    }
}
