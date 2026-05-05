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
//!     print!("arrives {}s, plan: ", journey.arrival());
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
//! dense indices at construction. The trait's only required soundness
//! contract is no-overtaking within a route. Footpaths returned by
//! [`Timetable::get_footpaths_from`] describe direct walks only — the
//! algorithm chains them within a round so transitive closure is not
//! required (adapters whose relation *is* closed can opt into a faster
//! single-pass relaxation via
//! [`Timetable::footpaths_are_transitively_closed`]).
//!
//! Based on the paper: *Round-Based Public Transit Routing* by Daniel
//! Delling, Thomas Pajor, and Renato F. Werneck.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;

use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

pub mod gtfs;
pub mod labels;
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

/// A label attached to a `(round, stop)` cell during the RAPTOR scan.
///
/// In single-criterion routing this is just an arrival time
/// ([`ArrivalTime`]). The trait is the seam where multi-criterion
/// label types (walking time, transfer slack, fare zones) plug in
/// without touching the core algorithm. The algorithm maintains a
/// Pareto front (a *bag* of mutually non-dominated labels) per
/// `(round, stop)`, so multi-criterion impls produce real Pareto
/// fronts at the targets rather than a single tiebroken label.
/// Single-criterion `ArrivalTime` bags stay size 1, with no
/// behaviour change versus a non-bag implementation.
pub trait Label: Copy + std::fmt::Debug {
    /// The "unreached" sentinel. The algorithm initialises every
    /// `(round, stop)` cell to this value before seeding origins.
    const UNREACHED: Self;

    /// Initial label at an origin stop departing at time `tau`.
    fn from_departure(tau: Tau) -> Self;

    /// New label produced by alighting from a trip at this stop with
    /// arrival time `arrival_tau`. `self` is the label at the boarding
    /// stop. For multi-criterion impls, components like accumulated
    /// walking time inherit from `self`.
    fn extend_by_trip(self, arrival_tau: Tau) -> Self;

    /// New label after walking a footpath of duration `walk_time`.
    fn extend_by_footpath(self, walk_time: Tau) -> Self;

    /// `self` weakly dominates `other` (every criterion of `self` is
    /// at most the corresponding criterion of `other`). The default
    /// implementation uses [`Label::arrival`], which is correct for
    /// single-criterion impls.
    fn dominates(&self, other: &Self) -> bool {
        self.arrival() <= other.arrival()
    }

    /// Effective arrival time at the labelled stop. Used by the
    /// algorithm for target-threshold comparisons and by [`Journey`]
    /// output. Always returns [`Tau::MAX`] for [`Label::UNREACHED`].
    fn arrival(&self) -> Tau;
}

/// Single-criterion label = arrival time at a stop. Default `L`
/// throughout the algorithm. Constructing from a `Tau` is direct;
/// extracting back is `arrival()`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrivalTime(pub Tau);

impl Label for ArrivalTime {
    const UNREACHED: Self = ArrivalTime(Tau::MAX);

    #[inline]
    fn from_departure(tau: Tau) -> Self {
        ArrivalTime(tau)
    }

    #[inline]
    fn extend_by_trip(self, arrival_tau: Tau) -> Self {
        ArrivalTime(arrival_tau)
    }

    #[inline]
    fn extend_by_footpath(self, walk_time: Tau) -> Self {
        ArrivalTime(self.0.saturating_add(walk_time))
    }

    #[inline]
    fn arrival(&self) -> Tau {
        self.0
    }
}

/// A journey found by the RAPTOR algorithm.
///
/// Each journey consists of a sequence of (route, alight stop) steps and a
/// final label. Multiple journeys may be returned for a single query,
/// representing pareto-optimal trade-offs between fewer transfers and earlier
/// arrival.
///
/// `origin` is whichever of the user-supplied origin stops this journey
/// actually started from — relevant for multi-source queries (e.g. "any
/// platform of this station") where the algorithm picks the best origin
/// internally. Similarly `target` is the target stop reached.
///
/// `L` defaults to [`ArrivalTime`] for single-criterion routing.
#[derive(Debug, Clone)]
pub struct Journey<L: Label = ArrivalTime> {
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
    /// The label at the target stop, with the target's walk-time offset
    /// already folded in. For [`ArrivalTime`], `label.arrival()` is the
    /// effective arrival time in seconds since midnight.
    pub label: L,
}

impl<L: Label> Journey<L> {
    /// Convenience accessor: `self.label.arrival()`. The effective
    /// arrival time at the chosen target, with the target's
    /// walk-time offset already applied.
    pub fn arrival(&self) -> Tau {
        self.label.arrival()
    }

    /// Walk the plan against `tt` to recover the specific trip ridden
    /// for each leg, plus per-leg departure and arrival times.
    /// `tau` is the original query departure time and `origin_walk`
    /// is the walk-time offset for `self.origin` from the original
    /// origins slice (typically 0 for single-stop queries).
    ///
    /// Each returned [`TimedLeg`] reports the route, boarding stop,
    /// alighting stop, the specific [`TripIdx`] caught, and the
    /// boarding / alighting times. If the previous leg's alight
    /// stop is not directly served by the next leg's route, this
    /// scans the previous alight stop's direct footpath neighbours
    /// for one that *is* served, walks there, and uses that as the
    /// next leg's `board`. The walking time advances `depart`
    /// without producing a separate leg — callers detect a walking
    /// transfer by comparing `legs[n].alight` with `legs[n+1].board`.
    ///
    /// Returns `None` if the plan can't be matched against `tt`,
    /// for example:
    ///
    /// - A route doesn't serve the claimed alighting stop.
    /// - No trip departs at or after the rider's available time.
    /// - The transfer between two legs needs a walk chain longer
    ///   than one direct footpath hop. Multi-hop walk reconstruction
    ///   would require either a stored boarding stop per leg or a
    ///   per-call walk-graph relaxation; neither is shipped today.
    ///
    /// In practice the first two should not happen for a `Journey`
    /// produced by the same `tt` and `tau` — they're soundness
    /// escape hatches.
    ///
    /// **Loop routes:** if `route` revisits the boarding stop on
    /// its sequence, this picks the *earliest* qualifying position
    /// (matching what [`Timetable::get_routes_serving_stop`]
    /// reports). For non-loop routes (the common case) this is
    /// unambiguous; for loop-heavy networks the reconstructed trip
    /// matches the algorithm's choice in practice.
    pub fn with_timing<T: Timetable>(
        &self,
        tt: &T,
        tau: Tau,
        origin_walk: Tau,
    ) -> Option<Vec<TimedLeg>> {
        let mut legs = Vec::with_capacity(self.plan.len());
        let mut current_time = tau.saturating_add(origin_walk);
        let mut current_stop = self.origin;

        for &(route, alight) in &self.plan {
            // Find a stop on `route` reachable from current_stop —
            // either current_stop itself or, failing that, a one-hop
            // footpath neighbour that the route serves. Pick the
            // first matching neighbour in iteration order; for the
            // typical case of one neighbour on the route this is
            // unambiguous.
            let serving_here = tt.get_routes_serving_stop(current_stop);
            let (board, board_pos, walk_time) =
                if let Some(&(_, pos)) = serving_here.iter().find(|(r, _)| *r == route) {
                    (current_stop, pos, 0)
                } else {
                    let mut found = None;
                    for &neighbour in tt.get_footpaths_from(current_stop) {
                        if let Some(&(_, pos)) = tt
                            .get_routes_serving_stop(neighbour)
                            .iter()
                            .find(|(r, _)| *r == route)
                        {
                            let walk = tt.get_transfer_time(current_stop, neighbour);
                            found = Some((neighbour, pos, walk));
                            break;
                        }
                    }
                    found?
                };

            current_time = current_time.saturating_add(walk_time);

            let trip = tt.get_earliest_trip(route, current_time, board_pos)?;
            let depart = tt.get_departure_time(trip, board_pos);

            // Find the alight position by scanning forward from board_pos.
            let stops_ahead = tt.get_stops_after(route, board_pos);
            let alight_offset = stops_ahead.iter().position(|&s| s == alight)?;
            let alight_pos = board_pos + alight_offset as u32;
            let arrive = tt.get_arrival_time(trip, alight_pos);

            legs.push(TimedLeg {
                route,
                board,
                alight,
                trip,
                depart,
                arrive,
            });

            current_time = arrive;
            current_stop = alight;
        }

        Some(legs)
    }
}

/// One transit leg of a [`Journey`] with reconstructed timing.
/// Produced by [`Journey::with_timing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedLeg {
    /// The route boarded.
    pub route: RouteIdx,
    /// Stop where the rider boards.
    pub board: StopIdx,
    /// Stop where the rider alights.
    pub alight: StopIdx,
    /// The specific trip ridden — the earliest one departing at or
    /// after the rider's available time at `board`.
    pub trip: TripIdx,
    /// Departure time at `board`, in seconds since midnight.
    pub depart: Tau,
    /// Arrival time at `alight`, in seconds since midnight.
    pub arrive: Tau,
}

/// One reconstructable step in a journey: either a transit boarding event
/// (route-scan) or a walk along a footpath. Walks do not consume a round —
/// they happen *within* round `k` at the stop they alight on — so the
/// reconstruction logic chains through walk entries without decrementing
/// the round index.
///
/// `parent_arrival` disambiguates which Pareto-optimal label at the
/// parent stop (in the parent stop's bag) was extended to produce
/// the label this step belongs to. For single-criterion `ArrivalTime`
/// the parent bag is size-1, so this field is redundant; for
/// multi-criterion impls it lets reconstruction follow the right
/// label through the bag.
#[derive(Debug, Clone, Copy)]
enum Step {
    Boarded {
        from: StopIdx,
        route: RouteIdx,
        parent_arrival: Tau,
    },
    Walked {
        from: StopIdx,
        parent_arrival: Tau,
    },
}

/// Boarding tree key: `(round, stop, label_arrival)`. The third
/// component disambiguates Pareto-optimal labels with distinct
/// arrival times in the same `(round, stop)` bag.
type BoardingTree = BTreeMap<(K, StopIdx, Tau), Step>;

/// A Pareto front of [`Label`]s at a single `(round, stop)` cell.
/// Backed by `SmallVec<[L; 8]>` — for single-criterion `ArrivalTime`
/// the bag is always size 1 and stays inline; for multi-criterion
/// impls it grows up to 8 inline before spilling.
#[derive(Debug, Clone)]
struct LabelBag<L: Label> {
    items: SmallVec<[L; 8]>,
}

impl<L: Label> LabelBag<L> {
    fn new() -> Self {
        Self {
            items: SmallVec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn iter(&self) -> impl Iterator<Item = &L> {
        self.items.iter()
    }

    /// Try to insert `new`. Returns `true` if added (and removes any
    /// items it strictly dominates); returns `false` if some existing
    /// item weakly dominates `new` (no change).
    fn insert(&mut self, new: L) -> bool {
        for item in &self.items {
            if item.dominates(&new) {
                return false;
            }
        }
        self.items.retain(|item| !new.dominates(item));
        self.items.push(new);
        true
    }

    /// Minimum `arrival()` across the bag, or `Tau::MAX` if empty.
    fn min_arrival(&self) -> Tau {
        self.items
            .iter()
            .map(|l| l.arrival())
            .min()
            .unwrap_or(Tau::MAX)
    }

    fn clear(&mut self) {
        self.items.clear();
    }
}

impl<L: Label> Default for LabelBag<L> {
    fn default() -> Self {
        Self::new()
    }
}

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
/// Try to insert `(label, step)` into `labels[k][stop]` and the
/// boarding tree. Updates `best_arrival[stop]` and marks the stop
/// in `out` if the new label improves on `pt_threshold`. Returns
/// `true` if any insertion happened.
#[allow(clippy::too_many_arguments)]
fn insert_into_bag<L: Label>(
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    out: &mut Vec<StopIdx>,
    ever_reached: &mut FixedBitSet,
    pt_threshold: Tau,
    k: K,
    stop: StopIdx,
    label: L,
    step: Step,
) -> bool {
    let added = labels[k][stop.idx()].insert(label);
    if !added {
        return false;
    }
    board_detail.insert((k, stop, label.arrival()), step);
    best_arrival[stop.idx()].insert(label);
    ever_reached.insert(stop.idx());
    if label.arrival() < pt_threshold {
        out.push(stop);
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn relax_footpaths_round_closed<T: Timetable + ?Sized, L: Label>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: Tau,
    out: &mut Vec<StopIdx>,
    ever_reached: &mut FixedBitSet,
) {
    // Walk each source bag's labels once. With closure, chained walks
    // are unnecessary because every reachable pair is already a direct
    // edge in the relation.
    let mut staged: SmallVec<[L; 8]> = SmallVec::new();
    for stop_bit in sources.ones() {
        let stop = StopIdx::new(stop_bit as u32);
        if labels[k][stop.idx()].is_empty() {
            continue;
        }
        // Snapshot the source bag so we don't iterate while mutating
        // labels[k] (which can include the source itself).
        staged.clear();
        staged.extend(labels[k][stop.idx()].iter().copied());

        for &p_dash in timetable.get_footpaths_from(stop) {
            let walk = timetable.get_transfer_time(stop, p_dash);
            for source_label in &staged {
                let via_walk = source_label.extend_by_footpath(walk);
                insert_into_bag(
                    labels,
                    best_arrival,
                    board_detail,
                    out,
                    ever_reached,
                    pt_threshold,
                    k,
                    p_dash,
                    via_walk,
                    Step::Walked {
                        from: stop,
                        parent_arrival: source_label.arrival(),
                    },
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn relax_footpaths_round<T: Timetable + ?Sized, L: Label>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: Tau,
    out: &mut Vec<StopIdx>,
    heap: &mut BinaryHeap<Reverse<(Tau, u32)>>,
    ever_reached: &mut FixedBitSet,
) {
    // Multi-source Dijkstra over the footpath graph at round `k`,
    // ordered by `LabelBag::min_arrival()`. Each source's initial
    // priority is its current bag's minimum arrival; transfer times
    // are non-negative so Dijkstra is sound. Uses lazy deletion —
    // stale heap entries are skipped on pop.
    //
    // For multi-criterion bags this propagates only the min-arrival
    // label per stop; non-min labels in a source bag don't drive
    // further relaxation. This is sound for arrival-time pruning but
    // may miss Pareto-optimal walk chains where a non-min-arrival
    // label dominates downstream — a v0.12 concern.
    heap.clear();
    for bit in sources.ones() {
        let min_arr = labels[k][bit].min_arrival();
        if min_arr != Tau::MAX {
            heap.push(Reverse((min_arr, bit as u32)));
        }
    }

    let mut staged: SmallVec<[L; 8]> = SmallVec::new();
    while let Some(Reverse((arrival, stop_bit))) = heap.pop() {
        let stop = StopIdx::new(stop_bit);
        let stop_min = labels[k][stop.idx()].min_arrival();
        // Skip stale entries — a strictly better label was popped earlier.
        if arrival > stop_min {
            continue;
        }
        staged.clear();
        staged.extend(labels[k][stop.idx()].iter().copied());

        for &p_dash in timetable.get_footpaths_from(stop) {
            let walk = timetable.get_transfer_time(stop, p_dash);
            let mut any_added = false;
            for source_label in &staged {
                let via_walk = source_label.extend_by_footpath(walk);
                if insert_into_bag(
                    labels,
                    best_arrival,
                    board_detail,
                    out,
                    ever_reached,
                    pt_threshold,
                    k,
                    p_dash,
                    via_walk,
                    Step::Walked {
                        from: stop,
                        parent_arrival: source_label.arrival(),
                    },
                ) {
                    any_added = true;
                }
            }
            if any_added {
                heap.push(Reverse((
                    labels[k][p_dash.idx()].min_arrival(),
                    p_dash.get(),
                )));
            }
        }
    }
}

/// Returns the minimum of `best_arrival[t].min_arrival() + w` across
/// all `(t, w)` in `targets`, saturating on overflow. Returns
/// `Tau::MAX` if every target is unreached.
fn best_to_any_target<L: Label>(best_arrival: &[LabelBag<L>], targets: &[(StopIdx, Tau)]) -> Tau {
    targets
        .iter()
        .map(|&(t, w)| best_arrival[t.idx()].min_arrival().saturating_add(w))
        .min()
        .unwrap_or(Tau::MAX)
}

/// Reconstruct a single candidate plan terminating at the target
/// label `(pt, target_arrival)` at round `k`. Traces back through
/// the boarding tree (which is keyed on `(round, stop, label_arrival)`
/// to disambiguate Pareto-optimal labels in the same bag). Returns
/// `Some((origin, plan))` if the trace reaches some stop in `origins`,
/// `None` otherwise.
fn reconstruct_journey(
    tree: &BoardingTree,
    origins: &FixedBitSet,
    pt: StopIdx,
    target_arrival: Tau,
    k: K,
) -> Option<(StopIdx, Vec<(RouteIdx, StopIdx)>)> {
    if tree.is_empty() {
        return None;
    }

    let mut plan = Vec::with_capacity(k);
    let mut parent = pt;
    let mut parent_arrival = target_arrival;
    let mut inner_k = k;
    // Defensive bound to avoid pathological loops; 100 walk-hops per
    // round is well beyond anything realistic.
    let mut budget = (k + 1) * 100;

    while !origins.contains(parent.idx()) && budget > 0 {
        budget -= 1;

        let Some(step) = tree.get(&(inner_k, parent, parent_arrival)).copied() else {
            break;
        };

        match step {
            Step::Boarded {
                from,
                route,
                parent_arrival: pa,
            } => {
                plan.push((route, parent));
                parent = from;
                parent_arrival = pa;
                if inner_k == 0 {
                    break;
                }
                inner_k -= 1;
            }
            Step::Walked {
                from,
                parent_arrival: pa,
            } => {
                parent = from;
                parent_arrival = pa;
                // walks do not consume a round
            }
        }
    }

    if !plan.is_empty() && origins.contains(parent.idx()) {
        plan.reverse();
        Some((parent, plan))
    } else {
        None
    }
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
        self.raptor_with_label::<ArrivalTime>(transfers, tau, origins, targets)
    }

    /// Generic variant of [`Timetable::raptor`] over a custom
    /// [`Label`] type. Use this when you've implemented your own
    /// label (e.g. for accumulated walking time) and want to drive
    /// the algorithm with it. Single-criterion users want
    /// [`Timetable::raptor`].
    fn raptor_with_label<L: Label>(
        &self,
        transfers: usize,
        tau: Tau,
        origins: &[(StopIdx, Tau)],
        targets: &[(StopIdx, Tau)],
    ) -> Vec<Journey<L>>
    where
        Self: Sized,
    {
        let mut cache = RaptorCache::<L>::for_timetable(self);
        self.raptor_with_cache_and_label(&mut cache, transfers, tau, origins, targets)
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
        self.raptor_with_cache_and_label::<ArrivalTime>(cache, transfers, tau, origins, targets)
    }

    /// Generic variant of [`Timetable::raptor_with_cache`] over a
    /// custom [`Label`] type. The label parameter `L` is inferred
    /// from `cache: &mut RaptorCache<L>`, so callers don't need to
    /// turbofish.
    fn raptor_with_cache_and_label<L: Label>(
        &self,
        cache: &mut RaptorCache<L>,
        transfers: usize,
        tau: Tau,
        origins: &[(StopIdx, Tau)],
        targets: &[(StopIdx, Tau)],
    ) -> Vec<Journey<L>>
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
            ever_reached,
            ..
        } = cache;

        // Clear and populate the origin set used by reconstruction.
        origin_set.clear();
        for &(o, _) in origins {
            origin_set.insert(o.idx());
        }

        // Seed labels for each origin at tau + its walk-time offset.
        // Reconstruction breaks the trace loop when it hits an origin
        // (origin_set bit is set), so origins don't need a Step entry.
        for &(o, walk) in origins {
            let t = tau.saturating_add(walk);
            let seed = L::from_departure(t);
            if labels[0][o.idx()].insert(seed) {
                best_arrival[o.idx()].insert(seed);
                marked_stops.insert(o.idx());
                ever_reached.insert(o.idx());
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
                ever_reached,
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
                ever_reached,
            );
        }
        for s in walked_buf.drain(..) {
            marked_stops.insert(s.idx());
        }
        pt_threshold = best_to_any_target(best_arrival, targets);

        for k in 1..=transfers {
            // Sparse carry-forward: only clone bags at stops ever
            // reached so far this query. For real feeds the reached
            // set is small (<10% of stops in typical city queries),
            // so this avoids the 50k-stop dense memcpy that would
            // otherwise dominate per-round overhead.
            let (prev_labels, this_labels) = labels.split_at_mut(k);
            let src = &prev_labels[k - 1];
            let dst = &mut this_labels[0];
            for bit in ever_reached.ones() {
                dst[bit] = src[bit].clone();
            }

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

            // Per-route Pareto bag of riding entries. Each entry =
            // (boarding_label, current_trip, boarding_stop); a single
            // entry suffices for `ArrivalTime` (size-1 stop bags) but
            // multi-criterion impls may carry multiple Pareto-optimal
            // entries that boarded different trips.
            let mut route_bag: SmallVec<[(L, TripIdx, StopIdx); 8]> = SmallVec::new();
            let mut staged: SmallVec<[L; 8]> = SmallVec::new();

            for &route in q_routes.iter() {
                let p_pos = q_entry[route.idx()].expect("route in q_routes must have an entry");
                route_bag.clear();

                for (offset, &pi) in self.get_stops_after(route, p_pos).iter().enumerate() {
                    let pos = p_pos + offset as u32;

                    // 1. Alight every active riding entry at pi.
                    for &(boarding_label, trip, boarding_stop) in route_bag.iter() {
                        let arr = self.get_arrival_time(trip, pos);
                        let best_to_pi = best_arrival[pi.idx()].min_arrival();
                        let time_to_beat = best_to_pi.min(pt_threshold);
                        if arr >= time_to_beat {
                            continue;
                        }
                        let new_label = boarding_label.extend_by_trip(arr);
                        if labels[k][pi.idx()].insert(new_label) {
                            best_arrival[pi.idx()].insert(new_label);
                            board_detail.insert(
                                (k, pi, new_label.arrival()),
                                Step::Boarded {
                                    from: boarding_stop,
                                    route,
                                    parent_arrival: boarding_label.arrival(),
                                },
                            );
                            marked_stops.insert(pi.idx());
                            ever_reached.insert(pi.idx());
                        }
                    }

                    // 2. Try to extend route_bag with labels from
                    //    labels[k-1][pi] that can catch a trip on this
                    //    route at pi. Snapshot first to avoid aliasing.
                    staged.clear();
                    staged.extend(labels[k - 1][pi.idx()].iter().copied());
                    for candidate in &staged {
                        let cand_arr = candidate.arrival();
                        let trip = match self.get_earliest_trip(route, cand_arr, pos) {
                            Some(t) => t,
                            None => continue,
                        };
                        let trip_dep = self.get_departure_time(trip, pos);

                        // Redundancy check: existing route_bag entry
                        // dominates candidate AND boards an at-or-earlier
                        // trip at pi → candidate is redundant.
                        let mut redundant = false;
                        for &(l_existing, t_existing, _) in route_bag.iter() {
                            let existing_dep = self.get_departure_time(t_existing, pos);
                            if l_existing.dominates(candidate) && existing_dep <= trip_dep {
                                redundant = true;
                                break;
                            }
                        }
                        if redundant {
                            continue;
                        }

                        // Remove entries strictly dominated by candidate.
                        route_bag.retain(|&mut (l_existing, t_existing, _)| {
                            let existing_dep = self.get_departure_time(t_existing, pos);
                            !(candidate.dominates(&l_existing) && trip_dep <= existing_dep)
                        });
                        route_bag.push((*candidate, trip, pi));
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
                    ever_reached,
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
                    ever_reached,
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

        // For each target stop, enumerate every Pareto-optimal label
        // in the target's bag at every round and reconstruct its plan.
        // Effective label = bag label extended by the target's walk
        // time offset.
        let mut journeys: Vec<Journey<L>> = Vec::new();
        for &(target, walk) in targets {
            #[allow(clippy::needless_range_loop)]
            for k in 1..=transfers {
                // Snapshot to avoid borrowing labels through the trace loop.
                let bag_snapshot: SmallVec<[L; 8]> =
                    labels[k][target.idx()].iter().copied().collect();
                for raw_label in &bag_snapshot {
                    let raw_arr = raw_label.arrival();
                    if raw_arr == Tau::MAX {
                        continue;
                    }
                    let Some((origin, plan)) =
                        reconstruct_journey(board_detail, origin_set, target, raw_arr, k)
                    else {
                        continue;
                    };
                    let label = raw_label.extend_by_footpath(walk);
                    journeys.push(Journey {
                        origin,
                        target,
                        plan,
                        label,
                    });
                }
            }
        }

        // Output-side Pareto filter on (trip count, label). For any two
        // returned journeys neither weakly dominates the other on the
        // pair (plan.len, label). For single-criterion `ArrivalTime`
        // this collapses to "strictly decreasing arrival as trip count
        // increases" (the v0.10 contract); for multi-criterion impls
        // it preserves Pareto-incomparable journeys (e.g. faster but
        // more walking vs. slower but less walking).
        //
        // Sorted by plan.len ascending, then by `arrival()` ascending
        // as a tiebreaker so the iteration order is deterministic.
        journeys.sort_by_key(|j| (j.plan.len(), j.arrival()));
        let mut front: Vec<Journey<L>> = Vec::with_capacity(journeys.len());
        'outer: for j in journeys {
            for f in &front {
                if f.plan.len() <= j.plan.len() && f.label.dominates(&j.label) {
                    continue 'outer;
                }
            }
            front.retain(|f| !(j.plan.len() <= f.plan.len() && j.label.dominates(&f.label)));
            front.push(j);
        }
        front
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
pub struct RaptorCache<L: Label = ArrivalTime> {
    n_stops: u32,
    n_routes: u32,

    /// labels[k][stop.idx()] = Pareto bag of labels at stop with at
    /// most k trips. Empty bag means unreached.
    labels: Vec<Vec<LabelBag<L>>>,

    /// τ* — Pareto bag of best labels at each stop across all rounds.
    best_arrival: Vec<LabelBag<L>>,

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

    /// Bitset of stops with a non-empty bag in any round seen so
    /// far this query. Used to make per-round carry-forward sparse:
    /// only clone bags at set bits, not all `n_stops`.
    ever_reached: FixedBitSet,
}

impl<L: Label> RaptorCache<L> {
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
            best_arrival: (0..n_stops).map(|_| LabelBag::new()).collect(),
            board_detail: BTreeMap::new(),
            marked_stops: FixedBitSet::with_capacity(n_stops as usize),
            q_entry: vec![None; n_routes as usize],
            q_routes: Vec::new(),
            walked_buf: Vec::new(),
            origin_set: FixedBitSet::with_capacity(n_stops as usize),
            relax_heap: BinaryHeap::new(),
            ever_reached: FixedBitSet::with_capacity(n_stops as usize),
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

        // Resize labels: (transfers + 1) Vecs, each n_stops long, all empty bags.
        let needed = transfers + 1;
        for v in self.labels.iter_mut() {
            v.iter_mut().for_each(|b| b.clear());
        }
        if self.labels.len() < needed {
            self.labels.resize_with(needed, || {
                (0..self.n_stops).map(|_| LabelBag::new()).collect()
            });
        } else {
            self.labels.truncate(needed);
        }

        for v in &mut self.best_arrival {
            v.clear();
        }

        self.board_detail.clear();
        self.marked_stops.clear();
        self.ever_reached.clear();

        // Sparse-set reset: walk q_routes, clear corresponding q_entry slots.
        for r in self.q_routes.drain(..) {
            self.q_entry[r.idx()] = None;
        }

        self.walked_buf.clear();
    }
}
