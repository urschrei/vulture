//! [`Timetable`] – the trait adapters implement to plug a transit
//! network into the RAPTOR algorithm. The trait carries only the data
//! accessors, the closure-of-footpaths declaration, and the
//! [`Timetable::query`] / [`Timetable::query_with_label`] entry points
//! into the typestate builder. The algorithm itself lives in
//! [`crate::algorithm`].

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
/// Implement this trait to describe your transit network's topology and
/// schedule. The algorithm itself is invoked via the
/// [`Timetable::query`] builder.
///
/// Identifiers are dense `u32` indices ([`StopIdx`], [`RouteIdx`],
/// [`TripIdx`]). Adapters intern from external IDs (e.g. GTFS string IDs)
/// at construction time.
///
/// # Footpaths
///
/// The footpath relation returned by [`get_footpaths_from`] is the
/// *direct* walking edges only. A relation is *transitively closed*
/// when every walk reachable through a chain of direct edges is
/// already present as a single direct edge: i.e. if `A → B` and
/// `B → C` are both in the relation, then `A → C` is too, with the
/// combined walk time ([Wikipedia][tc]). The algorithm does **not**
/// require closure – it chains direct walks within a single round
/// using multi-source Dijkstra, so a non-closed relation produces
/// correct answers; closure is purely an optimisation that lets the
/// algorithm switch to a cheaper single-pass `O(E)` relaxation. See
/// [`footpaths_are_transitively_closed`] for the opt-in.
///
/// [tc]: https://en.wikipedia.org/wiki/Transitive_closure
/// [`footpaths_are_transitively_closed`]: Timetable::footpaths_are_transitively_closed
///
/// # No overtaking within a route
///
/// All trips returned by [`get_earliest_trip`] for a given route must
/// share a stop sequence and pairwise must not overtake. The algorithm
/// uses a binary search by departure time at intermediate stops, which
/// is only sound when the trip ordering is monotone at every stop.
/// Adapters that ingest data with multiple stop patterns or overtaking
/// should split such groups into separate routes at construction.
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
    fn get_earliest_trip(&self, route: RouteIdx, at: SecondOfDay, pos: u32) -> Option<TripIdx>;

    /// Returns the arrival time of a trip at the given position within its
    /// route's sequence.
    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay;

    /// Returns the departure time of a trip at the given position within its
    /// route's sequence.
    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay;

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
            mode: NeedsDeparture,
            _label: PhantomData,
        }
    }

    /// Like [`Timetable::query`] but with a custom [`Label`] type for
    /// multi-criterion routing. `Vec<Journey<L>>` and `Vec<RangeJourney<L>>`
    /// come back from the corresponding `.run()`, with each entry on the
    /// returned Pareto front a different trade-off across `L`'s criteria.
    ///
    /// You only need this if [`ArrivalTime`] (the default) is the wrong
    /// shape for your problem – e.g. you want to surface a slower route
    /// with less walking. The bundled [`labels::ArrivalAndWalk`](crate::labels::ArrivalAndWalk)
    /// does exactly that. See the [`Label`] trait for what's involved in
    /// writing your own.
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
            mode: NeedsDeparture,
            _label: PhantomData,
        }
    }
}
