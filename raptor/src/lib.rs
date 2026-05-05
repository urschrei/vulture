#![deny(missing_docs)]

//! Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.
//!
//! RAPTOR finds all Pareto-optimal journeys between two stops in a transit
//! network, trading off between fewer transfers and earlier arrival.
//!
//! # Quick start: query a GTFS feed
//!
//! Most users start with the bundled [`gtfs`] adapter, which wraps a parsed
//! GTFS feed and implements [`Timetable`] for it.
//!
//! ```no_run
//! use gtfs_structures::Gtfs;
//! use jiff::civil::date;
//! use raptor::Timetable;
//! use raptor::gtfs::GtfsTimetable;
//!
//! # fn main() -> anyhow::Result<()> {
//! let gtfs = Gtfs::new("path/to/gtfs.zip")?;
//! // GTFS timetables are pinned to a specific service date — trips whose
//! // service_id is not active on this day are filtered out at construction.
//! let timetable = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;
//!
//! // `raptor` takes dense u32 indices, not GTFS string IDs — resolve first.
//! let start = timetable.stop_idx("dilshad_garden").expect("unknown stop");
//! let target = timetable.stop_idx("vishwavidyalaya").expect("unknown stop");
//!
//! // 10 = max transfers; 32400 = depart at 09:00 (seconds since midnight).
//! let journeys = timetable.raptor(10, 32400, &[(start, 0)], &[(target, 0)]);
//!
//! for journey in &journeys {
//!     print!("arrives {}s, plan: ", journey.arrival);
//!     for (route_idx, stop_idx) in &journey.plan {
//!         let route = timetable.route_id(*route_idx);
//!         let stop = timetable.stop_id(*stop_idx);
//!         print!("[{route} -> {stop}] ");
//!     }
//!     println!();
//! }
//! # Ok(())
//! # }
//! ```
//!
//! A returned [`Journey`]'s `plan` is a `Vec<(RouteIdx, StopIdx)>` — each
//! entry means "take this route, get off at this stop", with the source stop
//! implicit. The [`Journey`] type-level docs describe how walk legs at the
//! end of a journey are represented.
//!
//! For server use cases doing many queries against the same timetable, reuse
//! a [`RaptorCache`] via [`Timetable::raptor_with_cache`] to amortise
//! scratch-buffer allocation.
//!
//! # Implementing [`Timetable`] for a custom backend
//!
//! If your data is not in GTFS form, implement the [`Timetable`] trait
//! directly. Identifiers are dense `u32` newtypes ([`StopIdx`], [`RouteIdx`],
//! [`TripIdx`]); your adapter is responsible for interning external IDs to
//! dense indices at construction. The trait's docs spell out two contracts
//! the algorithm relies on: footpath transitivity and no-overtaking within a
//! route.
//!
//! Based on the paper: *Round-Based Public Transit Routing* by Daniel
//! Delling, Thomas Pajor, and Renato F. Werneck.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;

use fixedbitset::FixedBitSet;

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
///
/// `origin` is whichever of the user-supplied origin stops this journey
/// actually started from — relevant for multi-source queries (e.g. "any
/// platform of this station") where the algorithm picks the best origin
/// internally. Similarly `target` is the target stop reached.
#[derive(Debug, Clone)]
pub struct Journey {
    /// The origin stop this journey starts from, picked from the
    /// user-supplied origin set.
    pub origin: StopIdx,
    /// The target stop this journey ends at, picked from the user-supplied
    /// target set.
    pub target: StopIdx,
    /// Sequence of steps, each a (route, alight stop) pair.
    ///
    /// The origin stop is implicit — it is not part of the plan. Each entry
    /// means "take this route until this stop". The first step boards at the
    /// origin stop, and each subsequent step boards at the stop where the
    /// previous step got off (possibly via an intermediate footpath).
    pub plan: Vec<(RouteIdx, StopIdx)>,
    /// Effective arrival time, in seconds since midnight. Includes the
    /// user-supplied walk-time offset for the chosen `target` stop.
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

/// Relax footpaths from every stop in `sources` at round `k`, propagating
/// to a fixed point.
///
/// Iteratively improves `labels[k][p_dash]` and the τ\* table
/// (`best_arrival`) until no walk produces a strictly better arrival.
/// Each improvement records a `Walked` step in the boarding tree so that
/// `reconstruct_journey` can later trace back through the walk leg.
///
/// Because relaxation iterates to a fixed point, the trait's footpath
/// relation does **not** need to be transitively closed: if A→B and B→C
/// are both in the relation, the algorithm chains them within a single
/// round and reaches C with the combined walk time.
///
/// Stops that should be added to the marked set for the next round are
/// pushed onto `out`, gated by `pt_threshold` (the current best effective
/// arrival at any target). Caller drains `out` between calls.
/// Single-pass relaxation for adapters that report `true` from
/// [`Timetable::footpaths_are_transitively_closed`]. One walk per
/// source — chained walks are unnecessary because every reachable
/// pair is already a direct edge in the relation.
///
/// `O(E)` per round, no heap. Always sound when the closure
/// precondition holds; produces the same labels and boarding tree as
/// [`relax_footpaths_round`] in that case.
#[allow(clippy::too_many_arguments)]
fn relax_footpaths_round_closed<T: Timetable + ?Sized>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<Tau>],
    best_arrival: &mut [Tau],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: Tau,
    out: &mut Vec<StopIdx>,
) {
    for stop_bit in sources.ones() {
        let stop = StopIdx::new(stop_bit as u32);
        let stop_arrival = labels[k][stop.idx()];
        if stop_arrival == Tau::MAX {
            continue;
        }
        for &p_dash in timetable.get_footpaths_from(stop) {
            let via_walk = stop_arrival.saturating_add(timetable.get_transfer_time(stop, p_dash));
            let cur = labels[k][p_dash.idx()];
            if via_walk < cur {
                labels[k][p_dash.idx()] = via_walk;
                board_detail.insert((k, p_dash), Step::Walked { from: stop });
                if via_walk < best_arrival[p_dash.idx()] {
                    best_arrival[p_dash.idx()] = via_walk;
                }
                if via_walk < pt_threshold {
                    out.push(p_dash);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn relax_footpaths_round<T: Timetable + ?Sized>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<Tau>],
    best_arrival: &mut [Tau],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: Tau,
    out: &mut Vec<StopIdx>,
    heap: &mut BinaryHeap<Reverse<(Tau, u32)>>,
) {
    // Multi-source Dijkstra over the footpath graph at round `k`. Each
    // source's initial label is its current `labels[k]` value; transfer
    // times are non-negative so Dijkstra is sound. Uses lazy deletion —
    // stale heap entries are skipped on pop.
    //
    // O(E log V) per round; replaces an earlier LIFO Vec-based queue
    // that degenerated to O(V·E) on dense walking graphs.
    heap.clear();
    for bit in sources.ones() {
        let arrival = labels[k][bit];
        if arrival != Tau::MAX {
            heap.push(Reverse((arrival, bit as u32)));
        }
    }

    while let Some(Reverse((arrival, stop_bit))) = heap.pop() {
        let stop = StopIdx::new(stop_bit);
        // Skip stale entries — a strictly better label was popped earlier.
        if arrival > labels[k][stop.idx()] {
            continue;
        }
        for &p_dash in timetable.get_footpaths_from(stop) {
            let via_walk = arrival.saturating_add(timetable.get_transfer_time(stop, p_dash));
            let cur = labels[k][p_dash.idx()];
            if via_walk < cur {
                labels[k][p_dash.idx()] = via_walk;
                board_detail.insert((k, p_dash), Step::Walked { from: stop });
                let cur_best = best_arrival[p_dash.idx()];
                if via_walk < cur_best {
                    best_arrival[p_dash.idx()] = via_walk;
                }
                if via_walk < pt_threshold {
                    out.push(p_dash);
                }
                heap.push(Reverse((via_walk, p_dash.get())));
            }
        }
    }
}

/// Returns the minimum of `best_arrival[t] + w` across all `(t, w)`
/// in `targets`, saturating on overflow. Returns `Tau::MAX` if every
/// target is unreached.
fn best_to_any_target(best_arrival: &[Tau], targets: &[(StopIdx, Tau)]) -> Tau {
    targets
        .iter()
        .map(|&(t, w)| best_arrival[t.idx()].saturating_add(w))
        .min()
        .unwrap_or(Tau::MAX)
}

/// Reconstruct candidate plans terminating at `pt`. For each k from 1
/// to `transfers`, traces back through the boarding tree; if the trace
/// reaches some stop in `origins`, emits a plan along with that origin.
fn reconstruct_journey(
    tree: &BoardingTree,
    origins: &FixedBitSet,
    pt: StopIdx,
    transfers: K,
) -> Vec<(StopIdx, Vec<(RouteIdx, StopIdx)>)> {
    if tree.is_empty() {
        // Either no trips were taken, or we never reached target. The latter is
        // possible if origin and target are nodes of a disjoint graph
        return Default::default();
    }

    let mut plans = Vec::new();

    for k in 1..=transfers {
        let mut plan = Vec::with_capacity(k);
        let mut parent = pt;
        let mut inner_k = k;
        // Bound the trace length to avoid pathological loops on a malformed
        // tree. With fixed-point footpath relaxation a walk chain within a
        // round can in principle visit every stop at most once, but the
        // tree is well-formed by construction (each insertion overwrites
        // the previous entry at the same (k, stop) key, so no cycles).
        // The budget is just defensive: 100 walk-hops per round is well
        // beyond anything realistic.
        let mut budget = (k + 1) * 100;

        log::debug!("outer_k = {k} | parent = {parent:?} | plans = {plans:?}");

        while !origins.contains(parent.idx()) && budget > 0 {
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

        if !plan.is_empty() && origins.contains(parent.idx()) {
            plan.reverse();
            plans.push((parent, plan));
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
/// # Footpaths
///
/// The footpath relation returned by [`get_footpaths_from`] does **not**
/// need to be transitively closed: if you can walk `A → B` and `B → C`,
/// the algorithm will chain them within a single round, reaching `C`
/// from `A` with the combined walk time. Footpath relaxation iterates to
/// a fixed point per round.
///
/// Closure can still be useful as an optimisation — pre-closed graphs
/// have fewer edges to traverse — but it is not a soundness
/// prerequisite.
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
    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, pos: u32) -> Option<TripIdx>;

    /// Returns the arrival time of a trip at the given position within its
    /// route's sequence.
    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> Tau;

    /// Returns the departure time of a trip at the given position within its
    /// route's sequence.
    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> Tau;

    /// Returns all stops directly reachable from the given stop via
    /// walking (footpaths).
    ///
    /// The relation does not need to be transitively closed — the
    /// algorithm chains walks within a round. See the trait-level docs.
    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx];

    /// Returns the walking transfer time between two stops, in seconds.
    /// The default implementation returns `1`.
    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
        let (_, _) = (from, to);
        1
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

    /// Runs the RAPTOR algorithm and returns all Pareto-optimal journeys
    /// from any of the `origins` to any of the `targets`.
    ///
    /// Each `(stop, walk)` entry in `origins` says "the user can reach
    /// this stop at time `tau + walk`". Each entry in `targets` says
    /// "reaching this stop is worth `walk` more seconds of walking to
    /// arrive at the user's actual destination". The algorithm minimises
    /// effective arrival = `arrival_at_target_stop + walk_time`.
    ///
    /// For a single-stop query, pass `&[(stop, 0)]`. For a station with
    /// multiple platforms, pass each platform with its walk time from the
    /// station entrance (often 0 if the user is willing to use any
    /// platform). Multi-source/multi-target also fits geocoding: pass all
    /// nearby stops with their walking times from the user's GPS.
    ///
    /// Allocates fresh scratch buffers on every call. For server use cases
    /// running thousands of queries against the same timetable, prefer
    /// [`Timetable::raptor_with_cache`] and reuse a [`RaptorCache`].
    fn raptor(
        &self,
        transfers: usize,
        tau: Tau,
        origins: &[(StopIdx, Tau)],
        targets: &[(StopIdx, Tau)],
    ) -> Vec<Journey>
    where
        Self: Sized,
    {
        let mut cache = RaptorCache::for_timetable(self);
        self.raptor_with_cache(&mut cache, transfers, tau, origins, targets)
    }

    /// Same as [`Timetable::raptor`], but reuses scratch buffers from
    /// `cache`. The cache is reset at the start of the call. Panics if the
    /// cache was sized for a different timetable.
    fn raptor_with_cache(
        &self,
        cache: &mut RaptorCache,
        transfers: usize,
        tau: Tau,
        origins: &[(StopIdx, Tau)],
        targets: &[(StopIdx, Tau)],
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
            q_entry,
            q_routes,
            walked_buf,
            origin_set,
            relax_heap,
            ..
        } = cache;

        // Clear and populate the origin set used by reconstruction.
        origin_set.clear();
        for &(o, _) in origins {
            origin_set.insert(o.idx());
        }

        // Seed labels for each origin at tau + its walk-time offset.
        for &(o, walk) in origins {
            let t = tau.saturating_add(walk);
            if t < labels[0][o.idx()] {
                labels[0][o.idx()] = t;
                best_arrival[o.idx()] = t;
                marked_stops.insert(o.idx());
            }
        }

        let mut pt_threshold = best_to_any_target(best_arrival, targets);

        // Pick the per-round footpath relaxation strategy once. Closed
        // graphs use a single-pass O(E) walk; non-closed graphs need
        // multi-source Dijkstra to chain walks to a fixed point.
        let footpaths_closed = self.footpaths_are_transitively_closed();

        if footpaths_closed {
            relax_footpaths_round_closed(
                self,
                0,
                labels,
                best_arrival,
                board_detail,
                marked_stops,
                pt_threshold,
                walked_buf,
            );
        } else {
            relax_footpaths_round(
                self,
                0,
                labels,
                best_arrival,
                board_detail,
                marked_stops,
                pt_threshold,
                walked_buf,
                relax_heap,
            );
        }
        for s in walked_buf.drain(..) {
            marked_stops.insert(s.idx());
        }
        pt_threshold = best_to_any_target(best_arrival, targets);

        for k in 1..=transfers {
            // Carry forward labels[k-1] into labels[k].
            let (prev_labels, this_labels) = labels.split_at_mut(k);
            let src = &prev_labels[k - 1];
            let dst = &mut this_labels[0];
            dst.copy_from_slice(src);

            // Build the route queue for this round. Each entry pairs a
            // route with the earliest position on that route from which we
            // can board this round. Stored positions are folded with `min`
            // so multiple marked stops on the same route resolve to the
            // earliest one.
            for stop_bit in marked_stops.ones() {
                let marked_stop = StopIdx::new(stop_bit as u32);
                for &(route, pos) in self.get_routes_serving_stop(marked_stop) {
                    match q_entry[route.idx()] {
                        None => {
                            q_entry[route.idx()] = Some(pos);
                            q_routes.push(route);
                        }
                        Some(prev_pos) => {
                            if pos < prev_pos {
                                q_entry[route.idx()] = Some(pos);
                            }
                        }
                    }
                }
            }

            marked_stops.clear();

            for &route in q_routes.iter() {
                let p_pos = q_entry[route.idx()].expect("route in q_routes must have an entry");
                let mut current_trip: Option<TripIdx> = None;
                let mut boarding_stop = self.stop_at(route, p_pos);

                for (offset, &pi) in self.get_stops_after(route, p_pos).iter().enumerate() {
                    let pos = p_pos + offset as u32;

                    if let Some(arr) = current_trip.map(|trip| self.get_arrival_time(trip, pos)) {
                        let best_to_pi = best_arrival[pi.idx()];
                        let time_to_beat = best_to_pi.min(pt_threshold);

                        if arr < time_to_beat {
                            board_detail.insert(
                                (k, pi),
                                Step::Boarded {
                                    from: boarding_stop,
                                    route,
                                },
                            );
                            labels[k][pi.idx()] = arr;
                            best_arrival[pi.idx()] = arr;
                            marked_stops.insert(pi.idx());
                        }
                    }

                    let t_prev_pi = labels[k - 1][pi.idx()];
                    let dep_at_pi = current_trip
                        .map(|trip| self.get_departure_time(trip, pos))
                        .unwrap_or(Tau::MAX);
                    if t_prev_pi <= dep_at_pi {
                        current_trip = self.get_earliest_trip(route, t_prev_pi, pos);
                        boarding_stop = pi;
                    }
                }
            }

            // Sparse-set reset of the route queue.
            for r in q_routes.drain(..) {
                q_entry[r.idx()] = None;
            }

            // Refresh target threshold after the route scan, then run
            // footpath relax. Refresh again after the footpath round so
            // the next round's boarding decisions use a current threshold.
            pt_threshold = best_to_any_target(best_arrival, targets);
            if footpaths_closed {
                relax_footpaths_round_closed(
                    self,
                    k,
                    labels,
                    best_arrival,
                    board_detail,
                    marked_stops,
                    pt_threshold,
                    walked_buf,
                );
            } else {
                relax_footpaths_round(
                    self,
                    k,
                    labels,
                    best_arrival,
                    board_detail,
                    marked_stops,
                    pt_threshold,
                    walked_buf,
                    relax_heap,
                );
            }
            for s in walked_buf.drain(..) {
                marked_stops.insert(s.idx());
            }
            pt_threshold = best_to_any_target(best_arrival, targets);

            if marked_stops.is_clear() {
                break;
            }
        }

        // For each target stop the algorithm reached, reconstruct candidate
        // plans and pair each with its origin (one of the user's origins,
        // chosen during the trace). Effective arrival = label at target +
        // target's walk-time offset.
        let mut journeys: Vec<Journey> = Vec::new();
        for &(target, walk) in targets {
            let plans = reconstruct_journey(board_detail, origin_set, target, transfers);
            for (origin, plan) in plans {
                let raw_arrival = labels[plan.len()][target.idx()];
                if raw_arrival == Tau::MAX {
                    continue;
                }
                let arrival = raw_arrival.saturating_add(walk);
                journeys.push(Journey {
                    origin,
                    target,
                    plan,
                    arrival,
                });
            }
        }

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

    /// labels[k][stop.idx()] = earliest arrival at stop with at most k trips.
    /// Tau::MAX sentinel for "unreached".
    labels: Vec<Vec<Tau>>,

    /// τ* — best arrival at each stop across all rounds.
    best_arrival: Vec<Tau>,

    /// Boarding tree for journey reconstruction.
    board_detail: BoardingTree,

    /// Bitset of marked stops, sized to n_stops.
    marked_stops: FixedBitSet,

    /// Per-round route queue. `q_entry[r.idx()] = Some(boarding_pos)` when
    /// route r has been entered this round; `q_routes` is the dense list
    /// of routes that have entries (for cheap iteration without scanning
    /// `n_routes` empty slots).
    q_entry: Vec<Option<u32>>,
    q_routes: Vec<RouteIdx>,

    /// Scratch buffer for footpath relaxation output.
    walked_buf: Vec<StopIdx>,

    /// Bitset of origin stops for the current query. Reconstruction
    /// terminates when the trace reaches any bit set here.
    origin_set: FixedBitSet,

    /// Min-heap reused across footpath relaxations. Entries are
    /// `(arrival_time, stop_bit)`; lazy deletion via the time field.
    relax_heap: BinaryHeap<Reverse<(Tau, u32)>>,
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
            best_arrival: vec![Tau::MAX; n_stops as usize],
            board_detail: BTreeMap::new(),
            marked_stops: FixedBitSet::with_capacity(n_stops as usize),
            q_entry: vec![None; n_routes as usize],
            q_routes: Vec::new(),
            walked_buf: Vec::new(),
            origin_set: FixedBitSet::with_capacity(n_stops as usize),
            relax_heap: BinaryHeap::new(),
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

        // Resize labels: (transfers + 1) Vecs, each n_stops long, all Tau::MAX.
        let needed = transfers + 1;
        for v in self.labels.iter_mut() {
            v.iter_mut().for_each(|x| *x = Tau::MAX);
        }
        if self.labels.len() < needed {
            self.labels
                .resize_with(needed, || vec![Tau::MAX; self.n_stops as usize]);
        } else {
            self.labels.truncate(needed);
        }

        for v in &mut self.best_arrival {
            *v = Tau::MAX;
        }

        self.board_detail.clear();
        self.marked_stops.clear();

        // Sparse-set reset: walk q_routes, clear corresponding q_entry slots.
        for r in self.q_routes.drain(..) {
            self.q_entry[r.idx()] = None;
        }

        self.walked_buf.clear();
    }
}
