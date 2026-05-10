//! [`Timetable`] – the adapter trait for plugging a transit network into
//! RAPTOR. Carries the data accessors, the closure-of-footpaths
//! declaration, and the [`Timetable::query`] / [`Timetable::query_with_label`]
//! entry points. The algorithm lives in [`crate::algorithm`].

use std::marker::PhantomData;

use crate::NeedsDeparture;
use crate::Query;
use crate::endpoints::Endpoints;
use crate::ids::RouteIdx;
use crate::ids::StopIdx;
use crate::ids::TripIdx;
use crate::label::ArrivalTime;
use crate::label::Label;
use crate::time::Duration;
use crate::time::SecondOfDay;
use crate::time::Transfers;

/// Models a route-based transit network for the RAPTOR algorithm.
///
/// The algorithm is invoked via the [`Timetable::query`] builder.
///
/// Identifiers are dense `u32` indices ([`StopIdx`], [`RouteIdx`],
/// [`TripIdx`]). Adapters intern external IDs (e.g. GTFS string IDs) at
/// construction.
///
/// # Footpaths
///
/// [`get_footpaths_from`] returns *direct* walking edges only. A relation is
/// *transitively closed* when every walk reachable through a chain of direct
/// edges is also present as a single direct edge: if `A → B` and `B → C` are
/// in the relation, so is `A → C` with the combined walk time
/// ([Wikipedia][tc]). The algorithm does **not** require closure; it chains
/// direct walks within a round via multi-source Dijkstra. Closure is an
/// optimisation that switches the relaxation to single-pass `O(E)`; see
/// [`footpaths_are_transitively_closed`].
///
/// [tc]: https://en.wikipedia.org/wiki/Transitive_closure
/// [`footpaths_are_transitively_closed`]: Timetable::footpaths_are_transitively_closed
///
/// # No overtaking within a route
///
/// Trips returned by [`get_earliest_trip`] for a given route must share a
/// stop sequence and pairwise must not overtake. The algorithm uses binary
/// search by departure time at intermediate stops, which is only sound when
/// the trip ordering is monotone at every stop. Adapters ingesting data with
/// multiple stop patterns or overtaking should split such groups into
/// separate routes at construction.
///
/// [`get_footpaths_from`]: Timetable::get_footpaths_from
/// [`get_earliest_trip`]: Timetable::get_earliest_trip
pub trait Timetable {
    /// Number of stops in this timetable. Stop indices are in `0..n_stops()`.
    fn n_stops(&self) -> usize;
    /// Number of routes (post-pattern-splitting). Route indices are in
    /// `0..n_routes()`.
    fn n_routes(&self) -> usize;

    /// Returns each route serving the given stop, paired with the *earliest*
    /// position of `stop` within that route's sequence.
    ///
    /// For loop routes where `stop` appears more than once on a route, only
    /// the smallest position is reported. Each route appears at most once
    /// in the returned slice.
    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)];

    /// Returns the route's stop sequence from `pos` onwards, inclusive.
    ///
    /// Iterating the returned slice with positional offsets gives the
    /// algorithm `(pos + offset, stop_at_position)` pairs without ambiguity,
    /// even when a route revisits stops.
    ///
    /// Panics if `pos` is out of range for the route.
    fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx];

    /// Returns the stop at the given position within a route's sequence.
    ///
    /// Panics if `pos` is out of range for the route.
    fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx;

    /// Finds the earliest trip on a route departing at or after `at` from
    /// the stop at the given position within the route's sequence.
    ///
    /// `pos` disambiguates which visit of the stop to consider when the route
    /// revisits it. Returns `None` if no trip departs at or after `at`.
    ///
    /// **Pickup contract.** The returned trip must allow boarding at `pos`
    /// (i.e. [`pickup_allowed(trip, pos)`](Self::pickup_allowed) is `true`).
    /// Trips that visit `pos` only to drop off passengers are not valid
    /// boarding candidates and must be skipped by the adapter; the
    /// algorithm relies on this filter happening here rather than gating
    /// every returned trip itself.
    fn get_earliest_trip(&self, route: RouteIdx, at: SecondOfDay, pos: u32) -> Option<TripIdx>;

    /// Returns the arrival time of a trip at the given position within its
    /// route's sequence.
    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay;

    /// Returns the departure time of a trip at the given position within its
    /// route's sequence.
    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay;

    /// Reports whether a passenger can *board* `trip` at the stop at
    /// `pos` along its route's sequence.
    ///
    /// Maps to GTFS `stop_times.pickup_type`: `0` (regularly scheduled),
    /// `2` (must phone agency), and `3` (must coordinate with driver) all
    /// count as boardable; `1` (no pickup available) is the only false
    /// case. Adapters whose data has no equivalent flag should leave the
    /// default impl in place.
    ///
    /// The algorithm consults this only as a contract on
    /// [`get_earliest_trip`](Self::get_earliest_trip) — adapters must
    /// filter unboardable trips out of that lookup directly. The method
    /// is exposed publicly as a way for callers (and adapter
    /// implementations) to query the same flag.
    fn pickup_allowed(&self, trip: TripIdx, pos: u32) -> bool {
        let (_, _) = (trip, pos);
        true
    }

    /// Reports whether a passenger can *alight* from `trip` at the stop
    /// at `pos` along its route's sequence.
    ///
    /// Maps to GTFS `stop_times.drop_off_type`: only `1` (no drop-off
    /// available) returns `false`; the other values all permit
    /// alighting. Adapters whose data has no equivalent flag should
    /// leave the default impl in place.
    ///
    /// The algorithm consults this during the alighting phase of each
    /// round: an arrival time at `pos` is recorded only when this method
    /// returns `true`. Trips that pass through `pos` but don't allow
    /// drop-off cannot be the alighting leg of a journey ending or
    /// transferring there.
    ///
    /// **Caveat (issue L in `docs/soundness.md`)**: with the default
    /// single-criterion `Label = ArrivalTime`, the route-scan bag
    /// collapses to one trip per route per round. If two trips on the
    /// same route have identical arrival times at every stop but
    /// different per-position drop-off flags, the algorithm cannot
    /// switch between them mid-route — the rider boards the
    /// tie-break-first trip and may miss a target the sibling trip
    /// would have allowed. This is rare in real feeds (different
    /// drop-off semantics usually correlate with different timing) but
    /// legal in GTFS.
    fn drop_off_allowed(&self, trip: TripIdx, pos: u32) -> bool {
        let (_, _) = (trip, pos);
        true
    }

    /// Reports whether `trip` is wheelchair-accessible. Maps to GTFS
    /// `trips.wheelchair_accessible`: only `2` (no accommodations)
    /// returns `false`; `0` (no info), `1` (some accommodations), and
    /// any unknown value all permit boarding for wheelchair queries.
    /// Adapters whose data has no equivalent flag should leave the
    /// default impl in place.
    ///
    /// Consulted only when a query has set
    /// [`Query::require_wheelchair_accessible`](crate::Query::require_wheelchair_accessible);
    /// otherwise the algorithm ignores accessibility and treats every
    /// trip as eligible.
    fn trip_wheelchair_accessible(&self, trip: TripIdx) -> bool {
        let _ = trip;
        true
    }

    /// Reports whether `stop` is wheelchair-accessible. Maps to GTFS
    /// `stops.wheelchair_boarding`: only `2` (not accessible) returns
    /// `false`; `0` (no info), `1` (some accessibility), and any
    /// unknown value all permit alighting for wheelchair queries.
    /// Adapters whose data has no equivalent flag should leave the
    /// default impl in place.
    ///
    /// Consulted only when a query has set
    /// [`Query::require_wheelchair_accessible`](crate::Query::require_wheelchair_accessible).
    fn stop_wheelchair_accessible(&self, stop: StopIdx) -> bool {
        let _ = stop;
        true
    }

    /// Like [`get_earliest_trip`](Self::get_earliest_trip), but if
    /// `require_wheelchair_accessible` is `true`, returns the earliest
    /// trip at `pos` whose [`trip_wheelchair_accessible`](Self::trip_wheelchair_accessible)
    /// is also true — walking *by trip index*, not by time, so trips
    /// that share a departure time but differ in accessibility don't
    /// get leapfrogged.
    ///
    /// The default implementation falls back to walking forward by
    /// `departure_time + 1s` after each rejected trip; that's correct
    /// for feeds where no two trips on the same route share a stop's
    /// departure time, but loses simultaneously-departing trips that
    /// differ in their wheelchair flag. Adapters with direct access to
    /// the per-route trip list (such as [`crate::gtfs::GtfsTimetable`])
    /// should override with a trip-index walk.
    fn earliest_accessible_trip(
        &self,
        route: RouteIdx,
        at: SecondOfDay,
        pos: u32,
        require_wheelchair_accessible: bool,
    ) -> Option<TripIdx> {
        if !require_wheelchair_accessible {
            return self.get_earliest_trip(route, at, pos);
        }
        let mut probe = at;
        loop {
            let trip = self.get_earliest_trip(route, probe, pos)?;
            if self.trip_wheelchair_accessible(trip) {
                return Some(trip);
            }
            let dep = self.get_departure_time(trip, pos);
            probe = dep + Duration::from_secs(1);
        }
    }

    /// Returns all stops directly reachable from the given stop via
    /// walking (footpaths).
    ///
    /// The relation does not need to be transitively closed – the
    /// algorithm chains walks within a round. See the trait-level docs.
    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx];

    /// Returns the walking transfer time between two stops.
    /// The default implementation returns 1 second.
    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Duration {
        let (_, _) = (from, to);
        Duration(1)
    }

    /// Reports whether the footpath relation is transitively closed —
    /// that is, whether `A → C` is already a direct edge whenever
    /// `A → B` and `B → C` are. The default is `false`.
    ///
    /// When `true`, the algorithm uses a single-pass `O(E)` footpath
    /// relaxation per round instead of the multi-source Dijkstra
    /// fallback (`O(E log V)`). This is a meaningful speedup on
    /// dense closed graphs (e.g. publisher-curated `transfers.txt`
    /// files in Berlin / Paris feeds).
    ///
    /// **Soundness**: returning `true` when the relation is *not*
    /// closed will cause the algorithm to miss journeys whose optimal
    /// path requires chaining direct walks within a round. Only return
    /// `true` if you know the relation is closed.
    fn footpaths_are_transitively_closed(&self) -> bool {
        false
    }

    /// Start a typestate-builder query. Returns a [`Query`] in the
    /// [`NeedsDeparture`] state. Call `.from(...).to(...).max_transfers(...)`
    /// (any order, all optional with defaults), then either
    /// `.depart_at(...)` for a single-departure query or
    /// `.depart_in_window(...)` for a range query, then `.run()`.
    ///
    /// ```no_run
    /// # use vulture::{Timetable, SecondOfDay, Duration, StopIdx};
    /// # fn ex<T: Timetable>(tt: &T, start: StopIdx, end: StopIdx) {
    /// let journeys = tt
    ///     .query()
    ///     .from(start)
    ///     .to(end)
    ///     .max_transfers(10)
    ///     .depart_at(SecondOfDay::hms(9, 0, 0))
    ///     .run();
    /// # }
    /// ```
    fn query(&self) -> Query<'_, Self, ArrivalTime, NeedsDeparture>
    where
        Self: Sized,
    {
        Query {
            tt: self,
            origins: Endpoints::new(),
            targets: Endpoints::new(),
            max_transfers: Transfers(10),
            require_wheelchair_accessible: false,
            ctx: Default::default(),
            mode: NeedsDeparture,
            _label: PhantomData,
        }
    }

    /// Like [`Timetable::query`] but parameterised on a custom [`Label`].
    /// `.run()` returns `Vec<Journey<L>>` or `Vec<RangeJourney<L>>`; each
    /// entry on the Pareto front is a different trade-off across `L`'s
    /// criteria.
    ///
    /// Use when [`ArrivalTime`] is the wrong shape for the problem (e.g.
    /// surfacing a slower route with less walking). [`labels::ArrivalAndWalk`](crate::labels::ArrivalAndWalk)
    /// is the bundled two-criterion impl; see the [`Label`] trait for
    /// custom impls.
    ///
    /// ```no_run
    /// # use vulture::{Timetable, SecondOfDay, StopIdx};
    /// # use vulture::labels::ArrivalAndWalk;
    /// # fn ex<T: Timetable>(tt: &T, start: StopIdx, end: StopIdx) {
    /// let pareto_front = tt
    ///     .query_with_label::<ArrivalAndWalk>()
    ///     .from(start)
    ///     .to(end)
    ///     .max_transfers(10)
    ///     .depart_at(SecondOfDay::hms(9, 0, 0))
    ///     .run();
    /// # }
    /// ```
    fn query_with_label<L: Label>(&self) -> Query<'_, Self, L, NeedsDeparture>
    where
        Self: Sized,
    {
        Query {
            tt: self,
            origins: Endpoints::new(),
            targets: Endpoints::new(),
            max_transfers: Transfers(10),
            require_wheelchair_accessible: false,
            ctx: Default::default(),
            mode: NeedsDeparture,
            _label: PhantomData,
        }
    }
}
