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
///
/// # Defining a custom label
///
/// The example below sketches a *route-preference* label: every route
/// has an integer "badness" score (lower = nicer route — perhaps more
/// scenic, better A/C, fewer crowds) and the algorithm should return
/// a Pareto front of `(arrival_time, worst_score_on_journey)`
/// trade-offs. The fast journey may pick the worst route; a slightly
/// slower journey may avoid it entirely.
///
/// 1. Define your label `struct` and a `Ctx` carrying the lookup
///    table. `Ctx` is borrowed immutably by every callback, so put
///    your big tables here rather than cloning them into the label.
/// 2. Implement [`Label`]. The per-trip context passed to
///    [`Label::extend_by_trip`] gives you `route` and `trip` to look
///    up scores; [`Label::extend_by_footpath`] is the place to
///    accumulate walk-side criteria.
/// 3. At query time, build the `Ctx` once and supply it via
///    [`Query::with_context`](crate::Query::with_context) before
///    `.depart_at(...)`.
///
/// ```no_run
/// use std::collections::HashMap;
/// use vulture::{
///     Duration, Journey, Label, RouteIdx, SecondOfDay, StopIdx, Timetable, TripIdx,
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
///     fn extend_by_trip(
///         self,
///         ctx: &Self::Ctx,
///         _trip: TripIdx,
///         route: RouteIdx,
///         _board_stop: StopIdx,
///         _board_pos: u32,
///         _alight_stop: StopIdx,
///         _alight_pos: u32,
///         arrival: SecondOfDay,
///     ) -> Self {
///         let score = ctx.0.get(&route).copied().unwrap_or(0);
///         Self {
///             arrival,
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
