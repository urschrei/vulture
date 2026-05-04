#![deny(missing_docs)]

//! Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.
//!
//! RAPTOR computes pareto-optimal journeys in a public transit network, trading off
//! between arrival time and number of transfers. Implement the [`Timetable`] trait
//! for your transit data, then call [`Timetable::raptor`] to query for journeys.
//!
//! A ready-made implementation for GTFS feeds is available in the [`gtfs`] module.
//!
//! # Example
//!
//! ```no_run
//! use raptor::Timetable;
//!
//! // implement Timetable for your transit data, then:
//! // let journeys = timetable.raptor(max_transfers, departure_time, source, target);
//! ```
//!
//! Based on the paper:
//! *Round-Based Public Transit Routing* by Daniel Delling, Thomas Pajor, and Renato F. Werneck.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod gtfs;
/// In-memory timetable for testing and simple use cases.
pub mod simple;

#[cfg(test)]
mod test;

/// The number of transfers (round number in the RAPTOR algorithm).
pub type K = usize;

/// Time value in seconds since midnight.
pub type Tau = usize;

/// Dense index of a stop within a [`Timetable`]. Indices are in `0..tt.n_stops()`.
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

/// Dense index of a route within a [`Timetable`]. Indices are in `0..tt.n_routes()`.
///
/// In the GTFS adapter, a single GTFS `route_id` may map to multiple
/// `RouteIdx`s — one per equivalence class of trips with identical stop
/// sequences and pairwise non-overtaking schedules. See
/// [`gtfs::GtfsTimetable`] for the splitting rules and lookup APIs.
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

/// Dense index of a trip within a [`Timetable`]. Indices are in `0..n_trips`.
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

/// A journey found by the RAPTOR algorithm.
///
/// Each journey consists of a sequence of (route, alight stop) steps and a
/// final arrival time. Multiple journeys may be returned for a single query,
/// representing pareto-optimal trade-offs between fewer transfers and earlier
/// arrival.
#[derive(Debug, Clone)]
pub struct Journey {
    /// Sequence of steps, each a (route, alight stop) pair.
    ///
    /// The source stop is implicit — it is not part of the plan. Each entry
    /// means "take this route until this stop". The first step boards at the
    /// source stop passed to [`Timetable::raptor`], and each subsequent step
    /// boards at the stop where the previous step got off.
    pub plan: Vec<(RouteIdx, StopIdx)>,
    /// Arrival time at the target stop, in seconds since midnight.
    pub arrival: Tau,
}

/// One reconstructable step in a journey: either a transit boarding event
/// (route-scan) or a walk along a footpath. Walks do not consume a round —
/// they happen *within* round `k` at the stop they alight on — so the
/// reconstruction logic chains through walk entries without decrementing
/// the round index.
#[derive(Debug, Clone, Copy)]
enum Step {
    Boarded { from: StopIdx, route: RouteIdx },
    Walked { from: StopIdx },
}

type BoardingTree = BTreeMap<(K, StopIdx), Step>;

/// Relax footpaths from every stop in `sources` at round `k`.
///
/// Improves `labels[k][p_dash]` and the τ\* table (`best_arrival`) when
/// walking yields a strictly better arrival, and records a `Walked` step
/// in the boarding tree so that `reconstruct_journey` can later trace
/// back through the walk leg. Stops that should be added to the marked
/// set after target-pruning against τ\*(pt) are pushed onto `out`; the
/// caller is responsible for clearing/draining it.
#[allow(clippy::too_many_arguments)]
fn relax_footpaths_round<T: Timetable + ?Sized>(
    timetable: &T,
    k: K,
    labels: &mut [BTreeMap<StopIdx, Tau>],
    best_arrival: &mut BTreeMap<StopIdx, Tau>,
    board_detail: &mut BoardingTree,
    sources: &BTreeSet<StopIdx>,
    pt: StopIdx,
    out: &mut Vec<StopIdx>,
) {
    for &stop in sources {
        let stop_arrival = labels[k].get(&stop).copied().unwrap_or(Tau::MAX);
        if stop_arrival == Tau::MAX {
            continue;
        }
        for &p_dash in timetable.get_footpaths_from(stop) {
            let via_walk = stop_arrival.saturating_add(timetable.get_transfer_time(stop, p_dash));
            let cur = labels[k].get(&p_dash).copied().unwrap_or(Tau::MAX);
            if via_walk < cur {
                labels[k].insert(p_dash, via_walk);
                board_detail.insert((k, p_dash), Step::Walked { from: stop });
                best_arrival
                    .entry(p_dash)
                    .and_modify(|v| *v = (*v).min(via_walk))
                    .or_insert(via_walk);
                if via_walk < best_arrival.get(&pt).copied().unwrap_or(Tau::MAX) {
                    out.push(p_dash);
                }
            }
        }
    }
}

fn reconstruct_journey(
    tree: &BoardingTree,
    ps: StopIdx,
    pt: StopIdx,
    transfers: K,
) -> Vec<Vec<(RouteIdx, StopIdx)>> {
    if tree.is_empty() {
        // Either no trips were taken, or we never reached target. The latter is
        // possible if ps and pt are nodes of a disjoint graph
        return Default::default();
    }

    let mut plans = Vec::new();

    for k in 1..=transfers {
        let mut plan = Vec::with_capacity(k);
        let mut parent = pt;
        let mut inner_k = k;
        // Bound the trace length to avoid pathological loops on a malformed
        // tree. In a well-formed tree walks are at most one hop per round
        // (footpaths are transitively closed and the helper only walks from
        // the round's marked sources), so 2 * k + 1 steps suffice.
        let mut budget = 2 * k + 1;

        log::debug!("outer_k = {k} | parent = {parent:?} | plans = {plans:?}");

        while parent != ps && budget > 0 {
            budget -= 1;
            log::debug!("inner_k = {inner_k} | parent = {parent:?} | plan = {plan:?}");

            let Some(step) = tree.get(&(inner_k, parent)).copied() else {
                log::debug!("stopping because tree has no entry for current (inner_k, parent)");
                break;
            };

            match step {
                Step::Boarded { from, route } => {
                    plan.push((route, parent));
                    parent = from;
                    if inner_k == 0 {
                        break;
                    }
                    inner_k -= 1;
                }
                Step::Walked { from } => {
                    parent = from;
                    // walks do not consume a round
                }
            }
        }

        if !plan.is_empty() && parent == ps {
            plan.reverse();
            plans.push(plan)
        }
    }

    plans
}

/// Models a route-based transit network for the RAPTOR algorithm.
///
/// Implement this trait to describe your transit network's topology and
/// schedule. The algorithm itself is provided as a default method
/// ([`Timetable::raptor`]).
///
/// Identifiers are dense `u32` indices ([`StopIdx`], [`RouteIdx`],
/// [`TripIdx`]). Adapters intern from external IDs (e.g. GTFS string IDs)
/// at construction time.
///
/// # Footpath transitivity
///
/// The footpath relation returned by [`get_footpaths_from`] must be
/// **transitively closed**: if you can walk `A → B` and `B → C`, then
/// `A → C` must also be reported as a footpath from `A` (with a transfer
/// time at most the sum of the two legs). The algorithm relaxes footpaths
/// once per round; it does not iterate to a fixed point. A non-closed
/// relation will cause RAPTOR to miss journeys whose optimal path involves
/// chained walks within a single round.
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

    /// Returns all routes that serve the given stop.
    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx];

    /// Given two stops on a route, returns whichever appears earlier in the
    /// route's sequence.
    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx;

    /// Returns all stops on a route from the given stop onwards (inclusive),
    /// in sequence order.
    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx];

    /// Finds the earliest trip on a route departing at or after `at` from
    /// `stop`. Returns `None` if no such trip exists.
    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx>;

    /// Returns the arrival time of a trip at a stop.
    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau;

    /// Returns the departure time of a trip at a stop.
    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau;

    /// Returns all stops reachable from the given stop via walking
    /// (footpaths).
    ///
    /// **The footpath relation must be transitively closed.** See the
    /// trait-level docs for details.
    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx];

    /// Returns the walking transfer time between two stops, in seconds.
    /// The default implementation returns `1`.
    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
        let (_, _) = (from, to);
        1
    }

    /// Runs the RAPTOR algorithm and returns all pareto-optimal journeys.
    ///
    /// Allocates fresh scratch buffers on every call. For server use cases
    /// running thousands of queries against the same timetable, prefer
    /// [`Timetable::raptor_with_cache`] and reuse a [`RaptorCache`].
    fn raptor(&self, transfers: usize, tau: Tau, ps: StopIdx, pt: StopIdx) -> Vec<Journey>
    where
        Self: Sized,
    {
        let mut cache = RaptorCache::for_timetable(self);
        self.raptor_with_cache(&mut cache, transfers, tau, ps, pt)
    }

    /// Same as [`Timetable::raptor`], but reuses scratch buffers from
    /// `cache`. The cache is reset at the start of the call. Panics if the
    /// cache was sized for a different timetable.
    fn raptor_with_cache(
        &self,
        cache: &mut RaptorCache,
        transfers: usize,
        tau: Tau,
        ps: StopIdx,
        pt: StopIdx,
    ) -> Vec<Journey>
    where
        Self: Sized,
    {
        cache.reset_for_query(transfers, self.n_stops() as u32, self.n_routes() as u32);
        let RaptorCache {
            labels,
            best_arrival,
            board_detail,
            marked_stops,
            q,
            walked_buf,
            ..
        } = cache;

        labels[0].insert(ps, tau);
        best_arrival.insert(ps, tau);
        marked_stops.insert(ps);

        // Round 0 footpath relaxation: a journey starting with a walk should
        // be discoverable in round 1, which requires `ps`'s walk-neighbours to
        // already appear in labels[0] before the first round.
        relax_footpaths_round(
            self,
            0,
            labels,
            best_arrival,
            board_detail,
            marked_stops,
            pt,
            walked_buf,
        );
        marked_stops.extend(walked_buf.drain(..));

        for k in 1..=transfers {
            // Carry forward round k-1's labels into round k as the baseline,
            // so that any stop reached in a previous round remains usable as a
            // boarding point or footpath origin even if route scanning in this
            // round does not re-improve it.
            labels[k] = labels[k - 1].clone();

            q.clear();
            // find all routes that serve the marked stops, for evaluation in this round
            for &marked_stop in marked_stops.iter() {
                for &route in self.get_routes_serving_stop(marked_stop) {
                    let p_dash = q.entry(route).or_insert(marked_stop);
                    *p_dash = self.get_earlier_stop(route, marked_stop, *p_dash);
                }
            }

            marked_stops.clear();

            // scanning each route
            for (&route, &p) in q.iter() {
                let mut current_trip: Option<TripIdx> = None;
                let mut boarding_stop = p;

                for &pi in self.get_stops_after(route, p) {
                    if let Some(arr) = current_trip.map(|trip| self.get_arrival_time(trip, pi)) {
                        let best_arrival_to_target = best_arrival.get(&pt).unwrap_or(&Tau::MAX);
                        let best_arrival_to_pi = best_arrival.get(&pi).unwrap_or(&Tau::MAX);
                        let time_to_beat = *best_arrival_to_pi.min(best_arrival_to_target);

                        if arr < time_to_beat {
                            board_detail.insert(
                                (k, pi),
                                Step::Boarded {
                                    from: boarding_stop,
                                    route,
                                },
                            );
                            labels[k].insert(pi, arr);
                            best_arrival.insert(pi, arr);
                            marked_stops.insert(pi);
                        }
                    }

                    let t_prev_pi = labels[k - 1].get(&pi).copied().unwrap_or(Tau::MAX);
                    if t_prev_pi
                        <= current_trip
                            .map(|trip| self.get_departure_time(trip, pi))
                            .unwrap_or(Tau::MAX)
                    {
                        current_trip = self.get_earliest_trip(route, t_prev_pi, pi);
                        boarding_stop = pi;
                    }
                }
            }

            relax_footpaths_round(
                self,
                k,
                labels,
                best_arrival,
                board_detail,
                marked_stops,
                pt,
                walked_buf,
            );
            marked_stops.extend(walked_buf.drain(..));

            if marked_stops.is_empty() {
                break;
            }
        }

        let plans = reconstruct_journey(board_detail, ps, pt, transfers);

        let mut journeys: Vec<Journey> = plans
            .into_iter()
            .map(|plan| {
                let arrival = *labels[plan.len()].get(&pt).unwrap();
                Journey { plan, arrival }
            })
            .collect();

        // Output-side Pareto filter. Sort by trip count ascending, then keep
        // only journeys whose arrival is strictly less than the best seen so
        // far. After this, no returned journey is dominated by another in
        // the (trip count, arrival) ordering — i.e. for any two journeys
        // (k_a, t_a) and (k_b, t_b) with k_a < k_b, we have t_a > t_b.
        //
        // Local and target pruning during the rounds *should* already
        // prevent dominated journeys from being recorded, but this filter
        // makes the output contract independent of pruning correctness.
        journeys.sort_by_key(|j| j.plan.len());
        let mut best = Tau::MAX;
        journeys.retain(|j| {
            if j.arrival < best {
                best = j.arrival;
                true
            } else {
                false
            }
        });
        journeys
    }
}

/// Reusable scratch buffers for [`Timetable::raptor_with_cache`].
///
/// A `RaptorCache` is sized for a specific timetable's stop and route counts.
/// Construct with [`RaptorCache::for_timetable`]; passing it to a query
/// against a differently-sized timetable will panic.
///
/// A `RaptorCache` is *not* thread-safe and must not be shared across
/// queries running concurrently. For parallel query workloads, give each
/// worker thread its own cache.
pub struct RaptorCache {
    n_stops: u32,
    n_routes: u32,

    labels: Vec<BTreeMap<StopIdx, Tau>>,
    best_arrival: BTreeMap<StopIdx, Tau>,
    board_detail: BoardingTree,
    marked_stops: BTreeSet<StopIdx>,
    q: BTreeMap<RouteIdx, StopIdx>,
    walked_buf: Vec<StopIdx>,
}

impl RaptorCache {
    /// Constructs a cache sized for the given timetable.
    pub fn for_timetable<T: Timetable + ?Sized>(tt: &T) -> Self {
        Self::with_capacity(tt.n_stops() as u32, tt.n_routes() as u32)
    }

    /// Constructs a cache for a timetable with the given counts. Use
    /// [`for_timetable`](Self::for_timetable) when you have the timetable
    /// in scope.
    pub fn with_capacity(n_stops: u32, n_routes: u32) -> Self {
        Self {
            n_stops,
            n_routes,
            labels: Vec::new(),
            best_arrival: BTreeMap::new(),
            board_detail: BTreeMap::new(),
            marked_stops: BTreeSet::new(),
            q: BTreeMap::new(),
            walked_buf: Vec::new(),
        }
    }

    fn reset_for_query(&mut self, transfers: K, tt_n_stops: u32, tt_n_routes: u32) {
        assert_eq!(
            self.n_stops, tt_n_stops,
            "RaptorCache sized for {} stops but timetable has {}",
            self.n_stops, tt_n_stops
        );
        assert_eq!(
            self.n_routes, tt_n_routes,
            "RaptorCache sized for {} routes but timetable has {}",
            self.n_routes, tt_n_routes
        );

        for m in self.labels.iter_mut() {
            m.clear();
        }
        let needed = transfers + 1;
        if self.labels.len() < needed {
            self.labels.resize_with(needed, BTreeMap::new);
        } else {
            self.labels.truncate(needed);
        }
        self.best_arrival.clear();
        self.board_detail.clear();
        self.marked_stops.clear();
        self.q.clear();
        self.walked_buf.clear();
    }
}
