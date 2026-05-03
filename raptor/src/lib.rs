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

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

pub mod gtfs;
/// In-memory timetable for testing and simple use cases.
pub mod simple;

#[cfg(test)]
mod test;

#[cfg(test)]
mod proptest_support;

/// The number of transfers (round number in the RAPTOR algorithm).
pub type K = usize;

/// Time value in seconds since midnight.
pub type Tau = usize;

/// A journey found by the RAPTOR algorithm.
///
/// Each journey consists of a sequence of steps (route, arrival stop) and a final arrival time.
/// Multiple journeys may be returned for a single query, representing pareto-optimal trade-offs
/// between fewer transfers and earlier arrival.
#[derive(Debug, Clone)]
pub struct Journey<Route, Stop> {
    /// Sequence of steps, each a (route, stop to get off at) pair.
    ///
    /// The source stop is implicit — it is not part of the plan. Each entry means
    /// "take this route until this stop". The first step boards at the source stop
    /// passed to [`Timetable::raptor`], and each subsequent step boards at the stop
    /// where the previous step got off.
    ///
    /// For example, going from stop `"A"` to stop `"D"` with two transfers, the plan
    /// would look like:
    ///
    /// ```json
    /// [("R1", "B"), ("R2", "C"), ("R3", "D")]
    /// ```
    ///
    /// Read as: board `R1` at `A`, get off at `B`, board `R2` at `B`, get off at `C`,
    /// board `R3` at `C`, get off at `D`.
    ///
    /// See the [`gtfs-timetable`](https://github.com/keogami/raptor-rs/blob/main/examples/gtfs-timetable.rs)
    /// example for how to interpret and display a plan.
    pub plan: Vec<(Route, Stop)>,
    /// Arrival time at the target stop, in seconds since midnight.
    pub arrival: Tau,
}

/// One reconstructable step in a journey: either a transit boarding event
/// (route-scan) or a walk along a footpath. Walks do not consume a round —
/// they happen *within* round `k` at the stop they alight on — so the
/// reconstruction logic chains through walk entries without decrementing
/// the round index.
#[derive(Debug, Clone, Copy)]
enum Step<Route, Stop> {
    Boarded { from: Stop, route: Route },
    Walked { from: Stop },
}

type BoardingTree<Route, Stop> = BTreeMap<(K, Stop), Step<Route, Stop>>;

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
    labels: &mut [BTreeMap<T::Stop, Tau>],
    best_arrival: &mut BTreeMap<T::Stop, Tau>,
    board_detail: &mut BoardingTree<T::Route, T::Stop>,
    sources: &BTreeSet<T::Stop>,
    pt: T::Stop,
    out: &mut Vec<T::Stop>,
) {
    for &stop in sources {
        let stop_arrival = labels[k].get(&stop).copied().unwrap_or(Tau::MAX);
        if stop_arrival == Tau::MAX {
            continue;
        }
        for &p_dash in timetable.get_footpaths_from(stop).iter() {
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

fn reconstruct_journey<R, S>(
    tree: &BoardingTree<R, S>,
    ps: S,
    pt: S,
    transfers: K,
) -> Vec<Vec<(R, S)>>
where
    S: Ord + Copy + Debug,
    R: Copy + Debug,
{
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
/// Implement this trait to describe your transit network's topology and schedule.
/// The algorithm itself is provided as a default method ([`Timetable::raptor`]).
///
/// # Footpath transitivity
///
/// The footpath relation returned by [`get_footpaths_from`] must be
/// **transitively closed**: if you can walk `A → B` and `B → C`, then
/// `A → C` must also be reported as a footpath from `A` (with a
/// transfer time at most the sum of the two legs). The algorithm relaxes
/// footpaths once per round; it does not iterate to a fixed point. A
/// non-closed relation will cause RAPTOR to miss journeys whose optimal
/// path involves chained walks within a single round.
///
/// Most well-formed GTFS feeds satisfy this because `transfers.txt`
/// entries are typically explicit pairs. If your data source gives you
/// transitive walking edges (e.g. coordinate-derived footpaths within a
/// max radius), close the relation yourself before returning it from
/// `get_footpaths_from`.
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
    /// Identifier for a transit stop.
    type Stop: Ord + Copy + Debug;
    /// Identifier for a transit route.
    type Route: Ord + Copy + Debug;
    /// Identifier for a specific trip (a single run of a route).
    type Trip: Copy + Debug;

    /// Returns all routes that serve the given stop.
    fn get_routes_serving_stop(&self, stop: Self::Stop) -> Cow<'_, [Self::Route]>;

    /// Given two stops on a route, returns whichever appears earlier in the route's sequence.
    fn get_earlier_stop(
        &self,
        route: Self::Route,
        left: Self::Stop,
        right: Self::Stop,
    ) -> Self::Stop;

    /// Returns all stops on a route from the given stop onwards (inclusive), in sequence order.
    fn get_stops_after(&self, route: Self::Route, stop: Self::Stop) -> Cow<'_, [Self::Stop]>;

    /// Finds the earliest trip on a route departing at or after `at` from `stop`.
    ///
    /// Returns `None` if no such trip exists.
    fn get_earliest_trip(
        &self,
        route: Self::Route,
        at: Tau,
        stop: Self::Stop,
    ) -> Option<Self::Trip>;

    /// Returns the arrival time of a trip at a stop.
    fn get_arrival_time(&self, trip: Self::Trip, stop: Self::Stop) -> Tau;

    /// Returns the departure time of a trip at a stop.
    fn get_departure_time(&self, trip: Self::Trip, stop: Self::Stop) -> Tau;

    /// Returns all stops reachable from the given stop via walking (footpaths).
    ///
    /// **The footpath relation must be transitively closed.** See the
    /// trait-level docs for details. Without closure, the algorithm will
    /// miss journeys whose optimal path chains multiple walk legs in a
    /// single round.
    fn get_footpaths_from(&self, stop: Self::Stop) -> Cow<'_, [Self::Stop]>;

    /// Returns the walking transfer time between two stops, in seconds.
    ///
    /// The default implementation returns `1`. Override this for realistic transfer times.
    fn get_transfer_time(&self, from: Self::Stop, to: Self::Stop) -> Tau {
        let (_, _) = (from, to);
        1
    }

    /// Runs the RAPTOR algorithm and returns all pareto-optimal journeys.
    ///
    /// Finds journeys from `ps` (source) to `pt` (target) departing at or after `tau`,
    /// using at most `transfers` steps. Returns a set of pareto-optimal journeys trading
    /// off between fewer transfers and earlier arrival.
    ///
    /// "Pareto-optimal" here means: for any two returned journeys A and B,
    /// neither weakly dominates the other in the (trip count, arrival)
    /// ordering. The output is sorted by trip count ascending; arrival
    /// strictly decreases as trip count increases.
    ///
    /// Returns an empty `Vec` if no journey exists.
    ///
    /// Allocates fresh scratch buffers on every call. For server use cases
    /// running thousands of queries against the same timetable, prefer
    /// [`Timetable::raptor_with_cache`] and reuse a [`RaptorCache`].
    fn raptor(
        &self,
        transfers: usize,
        tau: usize,
        ps: Self::Stop,
        pt: Self::Stop,
    ) -> Vec<Journey<Self::Route, Self::Stop>> {
        let mut cache = RaptorCache::new();
        self.raptor_with_cache(&mut cache, transfers, tau, ps, pt)
    }

    /// Same as [`Timetable::raptor`], but reuses scratch buffers from
    /// `cache`. The cache is reset at the start of the call.
    ///
    /// Use this when running many queries against the same timetable —
    /// the per-query allocation cost (label maps, the boarding tree, the
    /// Q map, the marked-stops set) becomes the dominant overhead
    /// otherwise. A single `RaptorCache` is *not* thread-safe; use one
    /// per worker thread.
    fn raptor_with_cache(
        &self,
        cache: &mut RaptorCache<Self::Route, Self::Stop>,
        transfers: usize,
        tau: usize,
        ps: Self::Stop,
        pt: Self::Stop,
    ) -> Vec<Journey<Self::Route, Self::Stop>> {
        cache.reset_for_query(transfers);
        let RaptorCache {
            labels,
            best_arrival,
            board_detail,
            marked_stops,
            q,
            walked_buf,
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
                for &route in self.get_routes_serving_stop(marked_stop).iter() {
                    let p_dash = q.entry(route).or_insert(marked_stop);

                    *p_dash = self.get_earlier_stop(route, marked_stop, *p_dash);
                }
            }

            marked_stops.clear();

            // scanning each route
            for (&route, &p) in q.iter() {
                let mut current_trip: Option<Self::Trip> = None;
                let mut boarding_stop = p;

                for &pi in self.get_stops_after(route, p).iter() {
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

        let mut journeys: Vec<Journey<Self::Route, Self::Stop>> = plans
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
/// All internal allocations the algorithm needs — round labels, the τ\*
/// table, the boarding tree, the marked-stops set, the per-round route
/// queue, and a small reusable scratch vector for footpath relaxation —
/// live here. Construct one with [`RaptorCache::new`] and pass it to
/// every query against the same timetable.
///
/// A `RaptorCache` is *not* thread-safe and must not be shared across
/// queries running concurrently. For parallel query workloads, give
/// each worker thread its own cache.
pub struct RaptorCache<R, S>
where
    R: Ord + Copy + Debug,
    S: Ord + Copy + Debug,
{
    labels: Vec<BTreeMap<S, Tau>>,
    best_arrival: BTreeMap<S, Tau>,
    board_detail: BoardingTree<R, S>,
    marked_stops: BTreeSet<S>,
    q: BTreeMap<R, S>,
    walked_buf: Vec<S>,
}

impl<R, S> RaptorCache<R, S>
where
    R: Ord + Copy + Debug,
    S: Ord + Copy + Debug,
{
    /// Constructs an empty cache.
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            best_arrival: BTreeMap::new(),
            board_detail: BTreeMap::new(),
            marked_stops: BTreeSet::new(),
            q: BTreeMap::new(),
            walked_buf: Vec::new(),
        }
    }

    /// Resets every buffer to "empty" while retaining the heap
    /// allocations. Called at the start of every query.
    fn reset_for_query(&mut self, transfers: K) {
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

impl<R, S> Default for RaptorCache<R, S>
where
    R: Ord + Copy + Debug,
    S: Ord + Copy + Debug,
{
    fn default() -> Self {
        Self::new()
    }
}
