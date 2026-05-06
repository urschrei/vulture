//! Canned [`Label`] implementations.
//!
//! Single-criterion routing uses [`crate::ArrivalTime`] (re-exported at
//! the crate root). For multi-criterion routing, this module ships a
//! small set of vetted impls users can drive the algorithm with via
//! [`Timetable::query_with_label`](crate::Timetable::query_with_label).
//!
//! Custom impls live in user code – see the [`Label`]
//! trait docs for the requirements.

use std::collections::HashMap;

use crate::label::TripContext;
use crate::{Duration, Label, RouteIdx, SecondOfDay, StopIdx};

/// Two-criterion label tracking arrival time *and* accumulated walking
/// time. Trip rides preserve the boarding label's walking time;
/// footpath relaxations add the walk's duration to both arrival and
/// the running walking-time component.
///
/// Pareto dominance is the obvious component-wise relation: `self`
/// dominates `other` iff its arrival is `≤` and its walking time is
/// `≤`. The algorithm maintains a Pareto front per stop, so a query
/// with this label can return multiple journeys at the same target —
/// for example, a faster one with more walking and a slower one with
/// less.
///
/// ```no_run
/// use vulture::{Journey, StopIdx, SecondOfDay, Timetable};
/// use vulture::labels::ArrivalAndWalk;
///
/// # fn run<T: Timetable>(tt: &T, start: StopIdx, target: StopIdx) {
/// let journeys: Vec<Journey<ArrivalAndWalk>> = tt
///     .query_with_label::<ArrivalAndWalk>()
///     .from(start)
///     .to(target)
///     .max_transfers(10)
///     .depart_at(SecondOfDay::hms(9, 0, 0))
///     .run();
/// for j in &journeys {
///     println!(
///         "arrives {}s, walked {}s",
///         j.label.arrival, j.label.walk_time,
///     );
/// }
/// # }
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArrivalAndWalk {
    /// Effective arrival time at the labelled stop, in seconds since
    /// midnight.
    pub arrival: SecondOfDay,
    /// Total walking time accumulated along the path that produced
    /// this label, in seconds.
    pub walk_time: Duration,
}

impl Label for ArrivalAndWalk {
    type Ctx = ();
    const UNREACHED: Self = ArrivalAndWalk {
        arrival: SecondOfDay::MAX,
        walk_time: Duration::ZERO,
    };

    #[inline]
    fn from_departure(_ctx: &Self::Ctx, at: SecondOfDay) -> Self {
        ArrivalAndWalk {
            arrival: at,
            walk_time: Duration::ZERO,
        }
    }

    #[inline]
    fn extend_by_trip(self, _ctx: &Self::Ctx, leg: TripContext) -> Self {
        ArrivalAndWalk {
            arrival: leg.arrival,
            walk_time: self.walk_time,
        }
    }

    #[inline]
    fn extend_by_footpath(
        self,
        _ctx: &Self::Ctx,
        _from_stop: StopIdx,
        _to_stop: StopIdx,
        walk: Duration,
    ) -> Self {
        ArrivalAndWalk {
            arrival: self.arrival + walk,
            walk_time: self.walk_time + walk,
        }
    }

    #[inline]
    fn dominates(&self, other: &Self) -> bool {
        self.arrival <= other.arrival && self.walk_time <= other.walk_time
    }

    #[inline]
    fn arrival(&self) -> SecondOfDay {
        self.arrival
    }
}

/// Per-route fare table used by [`ArrivalAndFare`]. Routes outside
/// the table contribute a fare of zero. Stored as a flat
/// [`HashMap<RouteIdx, u32>`] of fare cents (or any integer fare unit
/// the user prefers — vulture treats it opaquely).
///
/// Build the table once at query-construction time from your feed's
/// fare data; pass into the query via
/// [`Query::with_context`](crate::Query::with_context).
#[derive(Debug, Default, Clone)]
pub struct FareTable {
    /// `RouteIdx` -> fare in user-defined units (typically cents).
    pub per_route: HashMap<RouteIdx, u32>,
}

impl FareTable {
    /// Lookup a route's fare; missing entries return zero.
    #[inline]
    pub fn fare_for(&self, route: RouteIdx) -> u32 {
        self.per_route.get(&route).copied().unwrap_or(0)
    }
}

impl FromIterator<(RouteIdx, u32)> for FareTable {
    /// Build a table from `(RouteIdx, fare)` pairs.
    fn from_iter<I: IntoIterator<Item = (RouteIdx, u32)>>(iter: I) -> Self {
        Self {
            per_route: iter.into_iter().collect(),
        }
    }
}

/// Two-criterion label tracking arrival time *and* accumulated fare.
/// Each trip ride adds the route's fare from the
/// [`FareTable`] context; footpaths advance arrival but not fare.
///
/// Pareto dominance is component-wise: `self` dominates `other` iff
/// its arrival is `≤` and its fare is `≤`. The Pareto front at a
/// target stop returns multiple journeys — typically a fastest-but-
/// expensive option and a slower-but-cheaper option, with whatever
/// trade-offs lie in between.
///
/// ```no_run
/// use std::collections::HashMap;
/// use vulture::{Journey, RouteIdx, SecondOfDay, StopIdx, Timetable};
/// use vulture::labels::{ArrivalAndFare, FareTable};
///
/// # fn run<T: Timetable>(tt: &T, start: StopIdx, target: StopIdx, premium_route: RouteIdx) {
/// // Premium route costs 500; everything else is fare-zero.
/// let mut per_route = HashMap::new();
/// per_route.insert(premium_route, 500);
/// let fares = FareTable { per_route };
///
/// let journeys: Vec<Journey<ArrivalAndFare>> = tt
///     .query_with_label::<ArrivalAndFare>()
///     .with_context(fares)
///     .from(start)
///     .to(target)
///     .max_transfers(10)
///     .depart_at(SecondOfDay::hms(9, 0, 0))
///     .run();
/// for j in &journeys {
///     println!("arrives {}s, fare {}", j.label.arrival, j.label.fare);
/// }
/// # }
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArrivalAndFare {
    /// Effective arrival time at the labelled stop.
    pub arrival: SecondOfDay,
    /// Accumulated fare across all transit legs of the journey
    /// producing this label, in the units of the supplied
    /// [`FareTable`].
    pub fare: u32,
}

impl Label for ArrivalAndFare {
    type Ctx = FareTable;
    const UNREACHED: Self = ArrivalAndFare {
        arrival: SecondOfDay::MAX,
        fare: 0,
    };

    #[inline]
    fn from_departure(_ctx: &Self::Ctx, at: SecondOfDay) -> Self {
        ArrivalAndFare {
            arrival: at,
            fare: 0,
        }
    }

    #[inline]
    fn extend_by_trip(self, ctx: &Self::Ctx, leg: TripContext) -> Self {
        ArrivalAndFare {
            arrival: leg.arrival,
            fare: self.fare.saturating_add(ctx.fare_for(leg.route)),
        }
    }

    #[inline]
    fn extend_by_footpath(
        self,
        _ctx: &Self::Ctx,
        _from_stop: StopIdx,
        _to_stop: StopIdx,
        walk: Duration,
    ) -> Self {
        ArrivalAndFare {
            arrival: self.arrival + walk,
            fare: self.fare,
        }
    }

    #[inline]
    fn dominates(&self, other: &Self) -> bool {
        self.arrival <= other.arrival && self.fare <= other.fare
    }

    #[inline]
    fn arrival(&self) -> SecondOfDay {
        self.arrival
    }
}
