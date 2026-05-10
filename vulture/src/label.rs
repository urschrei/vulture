//! The [`Label`] trait – what the algorithm carries at each `(round, stop)`
//! cell – plus the default single-criterion implementation [`ArrivalTime`].
//!
//! For multi-criterion impls (trade-off queries) see [`crate::labels`].

use crate::ids::{RouteIdx, StopIdx, TripIdx};
use crate::time::Duration;
use crate::time::SecondOfDay;

/// Per-trip context passed to [`Label::extend_by_trip`]. Single-criterion
/// labels read only `arrival`; multi-criterion impls (fare-aware,
/// route-preference, transfer-penalty) draw on the remaining fields.
#[derive(Debug, Clone, Copy)]
pub struct TripContext {
    /// The trip the rider is alighting from.
    pub trip: TripIdx,
    /// The route the trip belongs to. Use this as the key for fare
    /// tables, per-route preference scores, and similar route-keyed
    /// lookup data threaded via [`Label::Ctx`].
    pub route: RouteIdx,
    /// The stop where the rider boarded `trip`.
    pub board_stop: StopIdx,
    /// The position of `board_stop` within the route's stop
    /// sequence (the index into `Timetable::get_stops_after(route, 0)`).
    pub board_pos: u32,
    /// The stop where the rider is alighting.
    pub alight_stop: StopIdx,
    /// The position of `alight_stop` within the route's stop
    /// sequence.
    pub alight_pos: u32,
    /// Trip arrival time at `alight_pos`. Single-criterion labels copy
    /// this into their arrival component.
    pub arrival: SecondOfDay,
}

/// A label attached to a `(round, stop)` cell during the RAPTOR scan.
///
/// Most callers can ignore this trait. [`Timetable::query`](crate::Timetable::query)
/// uses [`ArrivalTime`] (single-criterion: minimise arrival time, fewest
/// transfers), matching the original RAPTOR paper.
///
/// The trait exists for *multi-criterion* routing: minimising arrival time
/// alongside another criterion and returning a Pareto front of trade-offs.
/// [`ArrivalAndWalk`](crate::labels::ArrivalAndWalk) (arrival vs walking
/// time) and [`ArrivalAndFare`](crate::labels::ArrivalAndFare) (arrival vs
/// accumulated fare via [`Label::Ctx`]) are worked examples. The builder
/// entry points are
/// [`Timetable::query_with_label`](crate::Timetable::query_with_label) and
/// [`Query::with_context`](crate::Query::with_context).
///
/// The algorithm maintains a Pareto front (a *bag* of mutually
/// non-dominated labels) per `(round, stop)`. Multi-criterion impls produce
/// Pareto fronts at the targets; single-criterion `ArrivalTime` bags stay
/// size 1.
///
/// # Defining a custom label
///
/// The example below sketches a route-preference label. Every route has an
/// integer "badness" score (lower is preferred); the algorithm returns a
/// Pareto front of `(arrival_time, worst_score_on_journey)`.
///
/// 1. Define the label `struct` and a `Ctx` carrying the lookup table.
///    `Ctx` is borrowed immutably by every callback; put heavy tables here.
/// 2. Implement [`Label`]. [`Label::extend_by_trip`] receives `route` and
///    `trip` for score lookups; [`Label::extend_by_footpath`] is the place
///    for walk-side criteria.
/// 3. Build the `Ctx` once and supply it via
///    [`Query::with_context`](crate::Query::with_context) before
///    `.depart_at(...)`.
///
/// ```no_run
/// use std::collections::HashMap;
/// use vulture::{
///     Duration, Journey, Label, RouteIdx, SecondOfDay, StopIdx, Timetable, TripContext,
/// };
///
/// // 1. Ctx: a route -> badness-score lookup. Default = empty map
/// //    (every route scores zero).
/// #[derive(Default, Debug, Clone)]
/// pub struct RouteScores(pub HashMap<RouteIdx, u32>);
///
/// // 2. Label: arrival time + worst score encountered along the
/// //    journey so far. Pareto-dominance is component-wise.
/// #[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// pub struct ArrivalAndWorstScore {
///     pub arrival: SecondOfDay,
///     pub worst: u32,
/// }
///
/// impl Label for ArrivalAndWorstScore {
///     type Ctx = RouteScores;
///     const UNREACHED: Self = Self {
///         arrival: SecondOfDay::MAX,
///         worst: 0,
///     };
///
///     fn from_departure(_ctx: &Self::Ctx, at: SecondOfDay) -> Self {
///         Self { arrival: at, worst: 0 }
///     }
///
///     fn extend_by_trip(self, ctx: &Self::Ctx, leg: TripContext) -> Self {
///         let score = ctx.0.get(&leg.route).copied().unwrap_or(0);
///         Self {
///             arrival: leg.arrival,
///             worst: self.worst.max(score),
///         }
///     }
///
///     fn extend_by_footpath(
///         self,
///         _ctx: &Self::Ctx,
///         _from_stop: StopIdx,
///         _to_stop: StopIdx,
///         walk: Duration,
///     ) -> Self {
///         Self { arrival: self.arrival + walk, worst: self.worst }
///     }
///
///     fn dominates(&self, other: &Self) -> bool {
///         self.arrival <= other.arrival && self.worst <= other.worst
///     }
///
///     fn arrival(&self) -> SecondOfDay { self.arrival }
/// }
///
/// // 3. Wire it in at query time.
/// # fn run<T: Timetable>(
/// #     tt: &T,
/// #     start: StopIdx,
/// #     target: StopIdx,
/// #     scores: HashMap<RouteIdx, u32>,
/// # ) {
/// let journeys: Vec<Journey<ArrivalAndWorstScore>> = tt
///     .query_with_label::<ArrivalAndWorstScore>()
///     .with_context(RouteScores(scores))
///     .from(start)
///     .to(target)
///     .max_transfers(5)
///     .depart_at(SecondOfDay::hms(9, 0, 0))
///     .run();
///
/// for j in &journeys {
///     println!("arrives {}, worst route score {}", j.label.arrival, j.label.worst);
/// }
/// # }
/// ```
pub trait Label: Copy + std::fmt::Debug {
    /// User-supplied sidecar data threaded into every `extend_*` /
    /// `from_departure` call. Carries lookup tables for per-trip /
    /// per-footpath criteria (fare table keyed by [`RouteIdx`], stop →
    /// zone map, agency → preference rank). The label itself stays `Copy`;
    /// heavy data lives in `Ctx` and is borrowed immutably.
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
    /// stop; `leg` bundles the boarding-and-alighting context so
    /// criteria like fare-per-trip can be evaluated. See
    /// [`TripContext`] for the field list. Single-criterion labels
    /// typically read only `leg.arrival`; multi-criterion impls pull
    /// from `ctx` keyed on `leg.route` / `leg.trip` etc.
    fn extend_by_trip(self, ctx: &Self::Ctx, leg: TripContext) -> Self;

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
    fn extend_by_trip(self, _ctx: &Self::Ctx, leg: TripContext) -> Self {
        ArrivalTime(leg.arrival)
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
