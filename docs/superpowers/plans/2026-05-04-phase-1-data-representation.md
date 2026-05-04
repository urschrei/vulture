# Phase 1 (1.1+1.2+1.3+1.4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the algorithm's data plane around dense `u32` indices (`StopIdx`/`RouteIdx`/`TripIdx`), `Vec<Vec<Tau>>` labels, a `FixedBitSet` of marked stops, and per-route `(stop_pos, trip_pos) -> Tau` tables in `GtfsTimetable`.

**Architecture:** The `Timetable` trait collapses (no associated types; all identifiers are newtype `u32`s; gains `n_stops`/`n_routes`). `RaptorCache` is non-generic, pre-bound by counts. `GtfsTimetable` and `SimpleTimetable` intern at construction time. The proptest harness is the safety net — every commit must keep `cargo nextest r -p raptor-proptest` green.

**Tech Stack:** Rust 2024, `gtfs-structures`, `smallvec`, `fixedbitset` (new), `hegel` for proptests, `criterion` for benches. Project uses `jujutsu` (`jj`) for version control per `~/.claude/CLAUDE.md`.

**Spec:** `docs/superpowers/specs/2026-05-04-phase-1-data-representation-design.md`

---

## Phase A — Type and trait shift (1.1)

This phase is **atomic**: changing the `Timetable` trait shape breaks every adapter, every test, every example, and the proptest harness simultaneously. There is no meaningful intermediate compiling state. Land all of Phase A in a single `jj commit` after the final verification task. Work through tasks A1–A13 without committing in between.

The algorithm body (`raptor_with_cache`) keeps using `BTreeMap<StopIdx, Tau>` labels and `BTreeSet<StopIdx>` marked stops in this phase — the storage rewrite happens in Phase B. Only types change.

### Task A1: Add index newtypes

**Files:**
- Modify: `raptor/src/lib.rs` — add three newtype definitions at the top of the file, after the existing `Tau`/`K` type aliases.

- [ ] **Step 1: Add the newtypes**

Insert in `raptor/src/lib.rs` after the `pub type Tau = usize;` line (around line 38), before the `Journey` definition:

```rust
use std::fmt;

/// Dense index of a stop within a [`Timetable`]. Indices are in `0..tt.n_stops()`.
///
/// Constructed by adapters at timetable-construction time. Display formats as
/// the bare `u32`; round-trip via [`StopIdx::get`] / [`From<u32>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StopIdx(u32);

impl StopIdx {
    /// Construct from a raw `u32`. The caller is responsible for the value
    /// being a valid index for the timetable in question.
    pub const fn new(n: u32) -> Self { Self(n) }
    /// The underlying `u32`.
    pub const fn get(self) -> u32 { self.0 }
    #[inline]
    pub(crate) fn idx(self) -> usize { self.0 as usize }
}

impl fmt::Display for StopIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl From<u32> for StopIdx { fn from(n: u32) -> Self { Self(n) } }
impl From<StopIdx> for u32 { fn from(s: StopIdx) -> Self { s.0 } }

/// Dense index of a route within a [`Timetable`]. Indices are in `0..tt.n_routes()`.
///
/// In the GTFS adapter, a single GTFS `route_id` may map to multiple
/// `RouteIdx`s — one per equivalence class of trips with identical stop
/// sequences and pairwise non-overtaking schedules. See
/// [`gtfs::GtfsTimetable`] for the splitting rules and lookup APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteIdx(u32);

impl RouteIdx {
    pub const fn new(n: u32) -> Self { Self(n) }
    pub const fn get(self) -> u32 { self.0 }
    #[inline]
    pub(crate) fn idx(self) -> usize { self.0 as usize }
}

impl fmt::Display for RouteIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl From<u32> for RouteIdx { fn from(n: u32) -> Self { Self(n) } }
impl From<RouteIdx> for u32 { fn from(r: RouteIdx) -> Self { r.0 } }

/// Dense index of a trip within a [`Timetable`]. Indices are in `0..n_trips`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TripIdx(u32);

impl TripIdx {
    pub const fn new(n: u32) -> Self { Self(n) }
    pub const fn get(self) -> u32 { self.0 }
    #[inline]
    pub(crate) fn idx(self) -> usize { self.0 as usize }
}

impl fmt::Display for TripIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl From<u32> for TripIdx { fn from(n: u32) -> Self { Self(n) } }
impl From<TripIdx> for u32 { fn from(t: TripIdx) -> Self { t.0 } }
```

(Do **not** run `cargo build` yet — this file will not compile until later tasks finish.)

### Task A2: Make `Journey` and `Step` concrete

**Files:**
- Modify: `raptor/src/lib.rs` — replace the existing generic `Journey<Route, Stop>` and `Step<Route, Stop>` with concrete types.

- [ ] **Step 1: Rewrite `Journey`**

Replace the existing `Journey<Route, Stop>` definition with:

```rust
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
```

- [ ] **Step 2: Rewrite `Step` and `BoardingTree`**

Replace:

```rust
#[derive(Debug, Clone, Copy)]
enum Step<Route, Stop> {
    Boarded { from: Stop, route: Route },
    Walked { from: Stop },
}

type BoardingTree<Route, Stop> = BTreeMap<(K, Stop), Step<Route, Stop>>;
```

with:

```rust
#[derive(Debug, Clone, Copy)]
enum Step {
    Boarded { from: StopIdx, route: RouteIdx },
    Walked { from: StopIdx },
}

type BoardingTree = BTreeMap<(K, StopIdx), Step>;
```

### Task A3: Rewrite the `Timetable` trait

**Files:**
- Modify: `raptor/src/lib.rs` — replace the entire `pub trait Timetable { … }` block with the new shape.

- [ ] **Step 1: Replace the trait body**

Replace the existing `pub trait Timetable { … }` block with:

```rust
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
        // body lifted in Task A5
        unimplemented!("filled in by Task A5")
    }
}
```

### Task A4: Update `relax_footpaths_round` and `reconstruct_journey`

**Files:**
- Modify: `raptor/src/lib.rs` — both functions are above the `Timetable` trait definition.

- [ ] **Step 1: Update `relax_footpaths_round`**

Replace its signature and body with:

```rust
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
```

Changes from the existing version: signature uses `StopIdx` directly, `get_footpaths_from` returns `&[StopIdx]` (no `.iter()`).

- [ ] **Step 2: Update `reconstruct_journey`**

Replace its signature and body with:

```rust
fn reconstruct_journey(
    tree: &BoardingTree,
    ps: StopIdx,
    pt: StopIdx,
    transfers: K,
) -> Vec<Vec<(RouteIdx, StopIdx)>> {
    if tree.is_empty() {
        return Default::default();
    }

    let mut plans = Vec::new();

    for k in 1..=transfers {
        let mut plan = Vec::with_capacity(k);
        let mut parent = pt;
        let mut inner_k = k;
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
```

Changes: removes the `<R, S>` generics and bounds; everything else identical.

### Task A5: Update `raptor_with_cache` body

**Files:**
- Modify: `raptor/src/lib.rs` — replace the `unimplemented!()` body in the trait method.

- [ ] **Step 1: Lift the existing body, retyped**

Replace the `raptor_with_cache` body (currently `unimplemented!("filled in by Task A5")`) with:

```rust
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
        labels[k] = labels[k - 1].clone();

        q.clear();
        for &marked_stop in marked_stops.iter() {
            for &route in self.get_routes_serving_stop(marked_stop) {
                let p_dash = q.entry(route).or_insert(marked_stop);
                *p_dash = self.get_earlier_stop(route, marked_stop, *p_dash);
            }
        }

        marked_stops.clear();

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
                            Step::Boarded { from: boarding_stop, route },
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
```

The `..` in the destructure ignores the new `n_stops`/`n_routes` fields added to `RaptorCache` in Task A6. Body logic is identical to the current implementation; only types changed.

### Task A6: Rewrite `RaptorCache` scaffolding (BTreeMap kept)

**Files:**
- Modify: `raptor/src/lib.rs` — replace the `RaptorCache<R, S>` struct, its `impl`, and its `Default` impl.

- [ ] **Step 1: Replace the struct definition**

Replace the existing `pub struct RaptorCache<R, S> { … }` and surrounding `impl`s with:

```rust
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
```

Note: no `Default` impl. `RaptorCache` must be sized for a timetable; there is no meaningful "empty" cache.

### Task A7: Rewrite `GtfsTimetable`

**Files:**
- Modify: `raptor/src/gtfs.rs` — rewrite the struct, constructor, lookup methods, and `Timetable` impl. Remove `RouteId` (the synthetic-route concept now lives on `RouteIdx`).

- [ ] **Step 1: Update the module docs and imports**

Replace the file's top-of-file comment block (lines 1–17) with:

```rust
//! [`Timetable`] implementation backed by a GTFS feed.
//!
//! Wraps a parsed [`Gtfs`] object and pre-computes lookup indices for
//! efficient route, stop, and trip queries.
//!
//! ## Synthetic routes
//!
//! A "RAPTOR route" is an equivalence class of trips with identical stop
//! sequences (the paper, §3.1). A GTFS `route_id` is *not* a RAPTOR route
//! — it routinely groups trips with different stop patterns
//! (short-turns, branching, deadheads). At construction time, this
//! adapter splits each `route_id` into one or more synthetic routes,
//! identified by [`RouteIdx`]. Trips on a synthetic route are
//! additionally split into non-overtaking sub-groups so that the
//! algorithm's binary-search-by-departure assumption holds.
//!
//! Use [`GtfsTimetable::route_id`] to recover the original GTFS
//! `route_id` for display, and [`GtfsTimetable::routes_for_gtfs_id`] to
//! enumerate every synthetic derived from a given GTFS route.
```

Replace the imports and constants block (lines 19–28) with:

```rust
use std::collections::HashMap;

use gtfs_structures::Gtfs;
use smallvec::SmallVec;

use crate::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

const TYPICAL_ROUTES_PER_STOP: usize = 8;
const TYPICAL_TRANSFERS_PER_STOP: usize = 4;
const DEFAULT_TRANSFER_TIME_SECONDS: usize = 300;
```

- [ ] **Step 2: Remove the `RouteId` struct**

Delete the existing `RouteId` definition and its `impl` block (lines 30–44).

- [ ] **Step 3: Update the type aliases and `GtfsError`**

Replace the existing `RoutesForStops`/`FootpathsForStops` aliases with this fully-rewritten block:

```rust
/// Errors that can occur when constructing a [`GtfsTimetable`].
#[derive(thiserror::Error, Debug)]
pub enum GtfsError {
    /// A trip referenced in the feed was not found.
    #[error("trip not found: {0}")]
    MissingTrip(String),
    /// A stop referenced by a trip was not found.
    #[error("stop not found: {0}")]
    MissingStop(String),
    /// A trip has no stop times defined.
    #[error("trip has no stop_times: {0}")]
    MissingStopTimes(String),
    /// A trip has a stop_time without a departure time, which the algorithm
    /// needs for binary-search ordering.
    #[error("stop_time has no departure_time: trip {trip}, stop {stop}")]
    MissingDepartureTime {
        /// The trip the stop_time belongs to.
        trip: String,
        /// The stop the stop_time refers to.
        stop: String,
    },
}

type GtfsResult<T> = std::result::Result<T, GtfsError>;
```

(The `GtfsError` definition is unchanged from the current file; we are deleting the type aliases and keeping the rest.)

- [ ] **Step 4: Replace the struct and `impl GtfsTimetable`**

Replace the existing `pub struct GtfsTimetable<'gtfs> { … }`, the `impl<'a> GtfsTimetable<'a> { … }` block, and the `cache_footpaths_for_stops`/`build_route_index` private functions. The `split_non_overtaking`, `overtakes`, and `find_stop_time` helpers below them stay until Task A7's later steps.

Insert this large block in their place:

```rust
/// A [`Timetable`] implementation that wraps a parsed GTFS feed.
///
/// Constructed via [`GtfsTimetable::new`], which validates the feed,
/// interns stops/routes/trips to dense `u32` indices, splits each GTFS
/// `route_id` into one or more [`RouteIdx`]s by stop pattern and
/// overtaking, and builds the lookup indices the algorithm requires.
pub struct GtfsTimetable<'gtfs> {
    gtfs: &'gtfs Gtfs,

    // Forward tables: idx -> &'gtfs str (original GTFS IDs).
    stop_ids: Vec<&'gtfs str>,
    route_ids: Vec<&'gtfs str>,
    trip_ids: Vec<&'gtfs str>,

    // Reverse tables.
    stop_by_id: HashMap<&'gtfs str, StopIdx>,
    route_by_id: HashMap<&'gtfs str, RouteIdx>,
    routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>>,
    trip_by_id: HashMap<&'gtfs str, TripIdx>,

    // Per-route tables.
    routes_for_stop: Vec<SmallVec<[RouteIdx; TYPICAL_ROUTES_PER_STOP]>>,
    stops_for_route: Vec<Vec<StopIdx>>,
    trips_for_route: Vec<Vec<TripIdx>>,

    footpaths_for_stops: Vec<SmallVec<[StopIdx; TYPICAL_TRANSFERS_PER_STOP]>>,
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,
}

impl<'gtfs> GtfsTimetable<'gtfs> {
    /// Creates a new timetable from a parsed GTFS feed.
    ///
    /// Validates that every trip references existing stops and has
    /// stop_times with departure times, then interns identifiers to dense
    /// `u32` indices and splits each GTFS `route_id` into synthetic
    /// [`RouteIdx`]s as described in the module docs.
    ///
    /// # Footpath assumptions
    ///
    /// The adapter passes `transfers.txt` entries through to the
    /// [`Timetable::get_footpaths_from`] return as-is, without computing
    /// the transitive closure. The [`Timetable`] trait requires the
    /// footpath relation to be transitively closed (see the trait-level
    /// docs).
    pub fn new(gtfs: &'gtfs Gtfs) -> GtfsResult<Self> {
        // 1. Intern stops in iteration order.
        let mut stop_ids: Vec<&'gtfs str> = Vec::with_capacity(gtfs.stops.len());
        let mut stop_by_id: HashMap<&'gtfs str, StopIdx> = HashMap::with_capacity(gtfs.stops.len());
        for (stop_id, _) in &gtfs.stops {
            let idx = StopIdx::new(stop_ids.len() as u32);
            stop_ids.push(stop_id.as_str());
            stop_by_id.insert(stop_id.as_str(), idx);
        }

        // 2. Validate trips and group by (route_id, stop_sequence) using
        //    interned stop indices.
        let mut groups: std::collections::BTreeMap<
            (&'gtfs str, Vec<StopIdx>),
            Vec<&'gtfs str>,
        > = std::collections::BTreeMap::new();
        for (trip_id, trip) in &gtfs.trips {
            if trip.stop_times.is_empty() {
                return Err(GtfsError::MissingStopTimes(trip_id.clone()));
            }
            let mut stop_seq: Vec<StopIdx> = Vec::with_capacity(trip.stop_times.len());
            for st in &trip.stop_times {
                let raw_id = st.stop.id.as_str();
                let stop_idx = *stop_by_id
                    .get(raw_id)
                    .ok_or_else(|| GtfsError::MissingStop(raw_id.to_owned()))?;
                if st.departure_time.is_none() {
                    return Err(GtfsError::MissingDepartureTime {
                        trip: trip_id.clone(),
                        stop: raw_id.to_owned(),
                    });
                }
                stop_seq.push(stop_idx);
            }
            groups
                .entry((trip.route_id.as_str(), stop_seq))
                .or_default()
                .push(trip_id.as_str());
        }

        // 3. For each (route_id, stop_seq) group, sort trips by first-stop
        //    departure and split into non-overtaking sub-groups. Each
        //    sub-group becomes a synthetic RouteIdx; trips become TripIdxs
        //    in synthetic-route order.
        let mut route_ids: Vec<&'gtfs str> = Vec::new();
        let mut stops_for_route: Vec<Vec<StopIdx>> = Vec::new();
        let mut trips_for_route: Vec<Vec<TripIdx>> = Vec::new();
        let mut trip_ids: Vec<&'gtfs str> = Vec::new();
        let mut trip_by_id: HashMap<&'gtfs str, TripIdx> = HashMap::new();
        let mut route_by_id: HashMap<&'gtfs str, RouteIdx> = HashMap::new();
        let mut routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>> = HashMap::new();
        let mut routes_for_stop: Vec<SmallVec<[RouteIdx; TYPICAL_ROUTES_PER_STOP]>> =
            vec![SmallVec::new(); stop_ids.len()];

        for ((gtfs_route_id, stop_seq), trips) in groups {
            let mut trips_with_schedules: Vec<(&'gtfs str, &'gtfs [gtfs_structures::StopTime])> =
                trips
                    .into_iter()
                    .map(|trip_id| {
                        let trip = gtfs.get_trip(trip_id).expect("just inserted");
                        (trip_id, trip.stop_times.as_slice())
                    })
                    .collect();
            trips_with_schedules
                .sort_by_key(|(_, st)| st[0].departure_time.expect("validated above"));

            for sub_group in split_non_overtaking(&trips_with_schedules) {
                let route_idx = RouteIdx::new(route_ids.len() as u32);
                route_ids.push(gtfs_route_id);
                stops_for_route.push(stop_seq.clone());

                let mut sub_trip_idxs: Vec<TripIdx> = Vec::with_capacity(sub_group.len());
                for trip_id in &sub_group {
                    let trip_idx = TripIdx::new(trip_ids.len() as u32);
                    trip_ids.push(trip_id);
                    trip_by_id.insert(trip_id, trip_idx);
                    sub_trip_idxs.push(trip_idx);
                }
                trips_for_route.push(sub_trip_idxs);

                route_by_id.entry(gtfs_route_id).or_insert(route_idx);
                routes_by_gtfs_id
                    .entry(gtfs_route_id)
                    .or_default()
                    .push(route_idx);

                for &stop_idx in &stop_seq {
                    routes_for_stop[stop_idx.idx()].push(route_idx);
                }
            }
        }

        // 4. Footpaths and transfer times.
        let mut footpaths_for_stops: Vec<SmallVec<[StopIdx; TYPICAL_TRANSFERS_PER_STOP]>> =
            vec![SmallVec::new(); stop_ids.len()];
        let mut transfer_times: HashMap<(StopIdx, StopIdx), Tau> = HashMap::new();
        for (stop_id, stop) in &gtfs.stops {
            if stop.transfers.is_empty() {
                continue;
            }
            let from_idx = *stop_by_id.get(stop_id.as_str()).expect("stop interned");
            for t in &stop.transfers {
                let Some(&to_idx) = stop_by_id.get(t.to_stop_id.as_str()) else { continue };
                footpaths_for_stops[from_idx.idx()].push(to_idx);
                if let Some(min) = t.min_transfer_time {
                    transfer_times.insert((from_idx, to_idx), min as Tau);
                }
            }
        }

        Ok(Self {
            gtfs,
            stop_ids,
            route_ids,
            trip_ids,
            stop_by_id,
            route_by_id,
            routes_by_gtfs_id,
            trip_by_id,
            routes_for_stop,
            stops_for_route,
            trips_for_route,
            footpaths_for_stops,
            transfer_times,
        })
    }

    /// Returns the original GTFS `stop_id` for the given index.
    pub fn stop_id(&self, stop: StopIdx) -> &'gtfs str {
        self.stop_ids[stop.idx()]
    }

    /// Returns the original GTFS `route_id` for the given synthetic route.
    /// Several `RouteIdx`s may map to the same GTFS `route_id`.
    pub fn route_id(&self, route: RouteIdx) -> &'gtfs str {
        self.route_ids[route.idx()]
    }

    /// Returns the original GTFS `trip_id` for the given index.
    pub fn trip_id(&self, trip: TripIdx) -> &'gtfs str {
        self.trip_ids[trip.idx()]
    }

    /// Looks up the index of a stop by its GTFS `stop_id`.
    pub fn stop_idx(&self, id: &str) -> Option<StopIdx> {
        self.stop_by_id.get(id).copied()
    }

    /// Looks up the *first* synthetic route derived from a GTFS
    /// `route_id`. Use [`routes_for_gtfs_id`](Self::routes_for_gtfs_id) to
    /// enumerate every synthetic.
    pub fn route_idx(&self, id: &str) -> Option<RouteIdx> {
        self.route_by_id.get(id).copied()
    }

    /// Returns every synthetic route derived from a given GTFS `route_id`.
    pub fn routes_for_gtfs_id(&self, id: &str) -> &[RouteIdx] {
        self.routes_by_gtfs_id
            .get(id)
            .map(|sv| sv.as_slice())
            .unwrap_or(&[])
    }

    /// Looks up the index of a trip by its GTFS `trip_id`.
    pub fn trip_idx(&self, id: &str) -> Option<TripIdx> {
        self.trip_by_id.get(id).copied()
    }
}
```

The previously-private `cache_footpaths_for_stops` and `build_route_index` functions and the `RouteIndex` struct are all gone — their work is inlined into `new()`.

- [ ] **Step 5: Update `find_stop_time`'s callers**

Keep `find_stop_time` as-is for now (it operates on GTFS string IDs internally). It will be replaced in Phase C, but we still need it for the `Timetable` impl below.

Add at the top of the existing `find_stop_time` definition (or just leave the helper as-is):

```rust
fn find_stop_time<'a>(gtfs: &'a Gtfs, trip: &str, stop: &str) -> &'a gtfs_structures::StopTime {
    let trip = gtfs.get_trip(trip).expect("validated during construction");
    trip.stop_times
        .iter()
        .find(|st| st.stop.id == stop)
        .expect("valid inputs")
}
```

(unchanged from the existing file)

- [ ] **Step 6: Replace the `Timetable` impl**

Replace the existing `impl<'gtfs> Timetable for GtfsTimetable<'gtfs> { … }` block with:

```rust
impl<'gtfs> Timetable for GtfsTimetable<'gtfs> {
    fn n_stops(&self) -> usize {
        self.stop_ids.len()
    }

    fn n_routes(&self) -> usize {
        self.route_ids.len()
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        self.routes_for_stop[stop.idx()].as_slice()
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        let stops = &self.stops_for_route[route.idx()];
        let left_pos = stops.iter().position(|&s| s == left);
        let right_pos = stops.iter().position(|&s| s == right);
        match (left_pos, right_pos) {
            (Some(l), Some(r)) if l <= r => left,
            (Some(_), Some(_)) => right,
            _ => panic!("both stops should exist on route"),
        }
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        let stops = &self.stops_for_route[route.idx()];
        let pos = stops
            .iter()
            .position(|&s| s == stop)
            .expect("stop should exist on route");
        &stops[pos..]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        let trips = &self.trips_for_route[route.idx()];
        let stop_pos = self.stops_for_route[route.idx()]
            .iter()
            .position(|&s| s == stop)?;

        let departure_at_stop = |trip_idx: TripIdx| -> Tau {
            let raw = self.trip_ids[trip_idx.idx()];
            let trip = self
                .gtfs
                .get_trip(raw)
                .expect("validated during construction");
            trip.stop_times[stop_pos]
                .departure_time
                .expect("validated during construction") as Tau
        };

        let idx = trips.partition_point(|&trip_idx| departure_at_stop(trip_idx) < at);
        trips.get(idx).copied()
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let raw_trip = self.trip_ids[trip.idx()];
        let raw_stop = self.stop_ids[stop.idx()];
        find_stop_time(self.gtfs, raw_trip, raw_stop)
            .arrival_time
            .expect("valid inputs") as Tau
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let raw_trip = self.trip_ids[trip.idx()];
        let raw_stop = self.stop_ids[stop.idx()];
        find_stop_time(self.gtfs, raw_trip, raw_stop)
            .departure_time
            .expect("valid inputs") as Tau
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        self.footpaths_for_stops[stop.idx()].as_slice()
    }

    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
        self.transfer_times
            .get(&(from, to))
            .copied()
            .unwrap_or(DEFAULT_TRANSFER_TIME_SECONDS)
    }
}
```

- [ ] **Step 7: Update the `#[cfg(test)] mod tests` block**

The existing GTFS tests (`overtakes_*`, `split_*`) operate on `gtfs_structures::StopTime` slices and don't touch the new types — leave them untouched. They should still compile.

### Task A8: Rewrite `SimpleTimetable`

**Files:**
- Modify: `raptor/src/simple/mod.rs` — change generic bounds (Hash + Eq + Clone), add interning, add lookup methods, rewrite `Timetable` impl.

- [ ] **Step 1: Update imports and the struct**

Replace the file's top section through the existing `impl<S, R, T> SimpleTimetable<…>` block with:

```rust
/// Benchmark timetable builders.
#[cfg(feature = "internal")]
pub mod builders;

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use crate::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

/// A generic in-memory timetable backed by interning tables and dense `Vec`s.
///
/// Generic over key types `S`/`R`/`T` (with `Hash + Eq + Clone` bounds) so
/// tests and benches can use ergonomic key types (enums, integers) while the
/// internal storage is dense `u32` indices throughout.
pub struct SimpleTimetable<S = u32, R = u32, T = u32>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    stop_keys: Vec<S>,
    stop_by_key: HashMap<S, StopIdx>,
    route_keys: Vec<R>,
    route_by_key: HashMap<R, RouteIdx>,
    trip_keys: Vec<T>,
    trip_by_key: HashMap<T, TripIdx>,

    /// RouteIdx -> ordered stop sequence.
    routes: Vec<Vec<StopIdx>>,
    /// TripIdx -> (route, per-stop (arrival, departure) aligned with the route's stops).
    trips: Vec<(RouteIdx, Vec<(Tau, Tau)>)>,
    /// StopIdx -> reachable stops via footpath.
    footpaths: Vec<Vec<StopIdx>>,
    /// (from, to) -> transfer time.
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,
    /// StopIdx -> routes serving the stop (computed incrementally).
    routes_for_stop: Vec<Vec<RouteIdx>>,
}

impl<S, R, T> SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    /// Creates an empty timetable.
    pub fn new() -> Self {
        Self {
            stop_keys: Vec::new(),
            stop_by_key: HashMap::new(),
            route_keys: Vec::new(),
            route_by_key: HashMap::new(),
            trip_keys: Vec::new(),
            trip_by_key: HashMap::new(),
            routes: Vec::new(),
            trips: Vec::new(),
            footpaths: Vec::new(),
            transfer_times: HashMap::new(),
            routes_for_stop: Vec::new(),
        }
    }

    fn intern_stop(&mut self, key: S) -> StopIdx {
        if let Some(&idx) = self.stop_by_key.get(&key) {
            return idx;
        }
        let idx = StopIdx::new(self.stop_keys.len() as u32);
        self.stop_keys.push(key.clone());
        self.stop_by_key.insert(key, idx);
        self.footpaths.push(Vec::new());
        self.routes_for_stop.push(Vec::new());
        idx
    }

    fn intern_route(&mut self, key: R) -> RouteIdx {
        if let Some(&idx) = self.route_by_key.get(&key) {
            return idx;
        }
        let idx = RouteIdx::new(self.route_keys.len() as u32);
        self.route_keys.push(key.clone());
        self.route_by_key.insert(key, idx);
        self.routes.push(Vec::new());
        idx
    }

    fn intern_trip(&mut self, key: T) -> TripIdx {
        if let Some(&idx) = self.trip_by_key.get(&key) {
            return idx;
        }
        let idx = TripIdx::new(self.trip_keys.len() as u32);
        self.trip_keys.push(key.clone());
        self.trip_by_key.insert(key, idx);
        idx
    }

    /// Adds a route with its stops and trips.
    pub fn route(mut self, id: R, stops: &[S], trips: &[(T, &[(Tau, Tau)])]) -> Self
    where
        R: Debug,
        T: Debug,
    {
        let route_idx = self.intern_route(id);
        let stop_idxs: Vec<StopIdx> =
            stops.iter().map(|s| self.intern_stop(s.clone())).collect();
        self.routes[route_idx.idx()] = stop_idxs.clone();

        // Update the routes_for_stop reverse index (idempotent: we dedup within the per-stop list).
        for &s in &stop_idxs {
            let entry = &mut self.routes_for_stop[s.idx()];
            if !entry.contains(&route_idx) {
                entry.push(route_idx);
            }
        }

        for &(trip_id, times) in trips {
            assert_eq!(
                times.len(),
                stops.len(),
                "trip {trip_id:?} has {} times but route has {} stops",
                times.len(),
                stops.len()
            );
            let trip_idx = self.intern_trip(trip_id);
            // trips Vec must be at least trip_idx.idx() + 1 long
            while self.trips.len() <= trip_idx.idx() {
                self.trips.push((RouteIdx::new(0), Vec::new()));
            }
            self.trips[trip_idx.idx()] = (route_idx, times.to_vec());
        }
        self
    }

    /// Adds a footpath from one stop to another.
    pub fn footpath(mut self, from: S, to: S) -> Self {
        let from_idx = self.intern_stop(from);
        let to_idx = self.intern_stop(to);
        self.footpaths[from_idx.idx()].push(to_idx);
        self
    }

    /// Sets the transfer time for a footpath.
    pub fn transfer_time(mut self, from: S, to: S, time: Tau) -> Self {
        let from_idx = self.intern_stop(from);
        let to_idx = self.intern_stop(to);
        self.transfer_times.insert((from_idx, to_idx), time);
        self
    }

    /// Returns the index assigned to the given stop key. Panics if the
    /// key was never inserted via [`route`](Self::route),
    /// [`footpath`](Self::footpath), or [`transfer_time`](Self::transfer_time).
    pub fn stop_idx_of(&self, key: &S) -> StopIdx {
        *self
            .stop_by_key
            .get(key)
            .expect("stop key not registered with this SimpleTimetable")
    }

    /// Returns the index assigned to the given route key.
    pub fn route_idx_of(&self, key: &R) -> RouteIdx {
        *self
            .route_by_key
            .get(key)
            .expect("route key not registered with this SimpleTimetable")
    }

    /// Returns the index assigned to the given trip key.
    pub fn trip_idx_of(&self, key: &T) -> TripIdx {
        *self
            .trip_by_key
            .get(key)
            .expect("trip key not registered with this SimpleTimetable")
    }
}

impl<S, R, T> Default for SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Drop the existing `From<…>` impl**

Delete the existing `impl<S, R, T> From<(&[…], &[…])> for SimpleTimetable<…>` block. It exists for tuple-syntax construction in old tests; nothing in the rewritten codebase uses it, and the builder API covers the same use cases. (If anything in tests/benches/proptests turns out to need it, it can be added back later — but a grep should show nothing depends on it.)

- [ ] **Step 3: Replace the `Timetable` impl**

Replace the existing `impl<S, R, T> Timetable for SimpleTimetable<S, R, T> { … }` block with:

```rust
impl<S, R, T> Timetable for SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone + Debug,
    R: Hash + Eq + Clone + Debug,
    T: Hash + Eq + Clone + Debug,
{
    fn n_stops(&self) -> usize {
        self.stop_keys.len()
    }

    fn n_routes(&self) -> usize {
        self.route_keys.len()
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        self.routes_for_stop[stop.idx()].as_slice()
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        let stops = &self.routes[route.idx()];
        let l = stops.iter().position(|&s| s == left).unwrap();
        let r = stops.iter().position(|&s| s == right).unwrap();
        stops[l.min(r)]
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        let stops = &self.routes[route.idx()];
        let pos = stops.iter().position(|&s| s == stop).unwrap();
        &stops[pos..]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        let stops = &self.routes[route.idx()];
        let stop_pos = stops.iter().position(|&s| s == stop)?;

        self.trips
            .iter()
            .enumerate()
            .filter(|(_, (r, _))| *r == route)
            .filter(|(_, (_, times))| times[stop_pos].1 >= at)
            .min_by_key(|(_, (_, times))| times[stop_pos].1)
            .map(|(trip_idx, _)| TripIdx::new(trip_idx as u32))
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let (route_idx, times) = &self.trips[trip.idx()];
        let stops = &self.routes[route_idx.idx()];
        let pos = stops.iter().position(|&s| s == stop).unwrap();
        times[pos].0
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let (route_idx, times) = &self.trips[trip.idx()];
        let stops = &self.routes[route_idx.idx()];
        let pos = stops.iter().position(|&s| s == stop).unwrap();
        times[pos].1
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        self.footpaths[stop.idx()].as_slice()
    }

    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
        self.transfer_times.get(&(from, to)).copied().unwrap_or(1)
    }
}
```

- [ ] **Step 4: Update the `#[cfg(feature = "dotgraph")] impl` block**

The existing dotgraph rendering is keyed by user-supplied `S` (stop keys). After the rewrite, internal storage is `StopIdx` and route-stop arrays are `Vec<StopIdx>`. The `to_dot` method needs updating.

Replace the existing `#[cfg(feature = "dotgraph")] impl<S, R, T> SimpleTimetable<S, R, T> { … }` block with:

```rust
#[cfg(feature = "dotgraph")]
impl<S, R, T> SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone + Debug + std::fmt::Display,
    R: Hash + Eq + Clone + Debug,
    T: Hash + Eq + Clone + Debug,
{
    /// Renders the timetable as a Graphviz DOT string.
    pub fn to_dot(&self, name: &str) -> Result<String, std::io::Error> {
        use dot_graph::{Edge, Graph, Kind, Node, Style};

        let mut graph = Graph::new(name, Kind::Digraph);

        for (idx, key) in self.stop_keys.iter().enumerate() {
            let _ = idx;
            graph.add_node(Node::new(&format!("s{key}")));
        }

        const COLORS: &[&str] = &[
            "red", "blue", "green", "orange", "purple",
            "brown", "deeppink", "darkgreen", "navy", "goldenrod",
        ];

        for (route_idx, route_stops) in self.routes.iter().enumerate() {
            let color = COLORS[route_idx % COLORS.len()];
            let route_idx_typed = RouteIdx::new(route_idx as u32);
            let route_trips: Vec<(usize, &(RouteIdx, Vec<(Tau, Tau)>))> = self
                .trips
                .iter()
                .enumerate()
                .filter(|(_, (r, _))| *r == route_idx_typed)
                .collect();

            for (i, window) in route_stops.windows(2).enumerate() {
                let from_key = &self.stop_keys[window[0].idx()];
                let to_key = &self.stop_keys[window[1].idx()];
                let route_key = &self.route_keys[route_idx];
                let mut label = format!("R{route_key:?}");
                for (trip_idx, (_, times)) in &route_trips {
                    let trip_key = &self.trip_keys[*trip_idx];
                    let dep = times[i].1;
                    let arr = times[i + 1].0;
                    label.push_str(&format!("\\nT{trip_key:?}: {dep}\u{2192}{arr}"));
                }
                graph.add_edge(
                    Edge::new(&format!("s{from_key}"), &format!("s{to_key}"), &label)
                        .color(Some(color)),
                );
            }
        }

        for (from_idx, targets) in self.footpaths.iter().enumerate() {
            let from_key = &self.stop_keys[from_idx];
            let from_typed = StopIdx::new(from_idx as u32);
            for &to in targets {
                let to_key = &self.stop_keys[to.idx()];
                let time = self
                    .transfer_times
                    .get(&(from_typed, to))
                    .copied()
                    .unwrap_or(1);
                graph.add_edge(
                    Edge::new(&format!("s{from_key}"), &format!("s{to_key}"), &format!("t={time}"))
                        .style(Style::Dashed)
                        .color(Some("gray")),
                );
            }
        }

        graph.to_dot_string()
    }
}
```

### Task A9: Verify benchmark builders still compile

**Files:**
- Inspect: `raptor/src/simple/builders.rs` — used by `raptor/benches/raptor.rs`. The builders use `usize` keys.

- [ ] **Step 1: Confirm builders pass through unchanged**

The builders construct `SimpleTimetable<usize, usize, usize>` — `usize` is `Hash + Eq + Clone`, so the new bounds are satisfied. The builders use `tt.route(…)`, `tt.footpath(…)`, `tt.transfer_time(…)` — all existing API. No changes required.

(The bench file `raptor/benches/raptor.rs` calls `tt.raptor(transfers, tau, source, target)` where `source`/`target` are `usize`. After the trait change, `raptor` takes `StopIdx`. The call sites need updating — see Task A11.)

### Task A10: Migrate unit tests

**Files:**
- Modify: `raptor/src/test.rs` — add `Hash` to all enum derives, introduce a `plan!` helper macro, rewrite plan assertions.

- [ ] **Step 1: Add `Hash` to every test enum derive**

Every `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]` in `test.rs` needs `Hash` appended. There are roughly 15 enums (Stop / Route / Trip per test). Each becomes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
```

Apply replace-all from `Eq, PartialOrd, Ord)]` to `Eq, PartialOrd, Ord, Hash)]` across the file.

- [ ] **Step 2: Add the `plan!` helper macro**

At the top of `raptor/src/test.rs` (after the existing `use` statements), add:

```rust
macro_rules! plan {
    ($tt:expr; $(($route:expr, $stop:expr)),* $(,)?) => {
        vec![$(($tt.route_idx_of(&$route), $tt.stop_idx_of(&$stop))),*]
    };
}
```

- [ ] **Step 3: Rewrite plan assertions**

Each test that asserts on `journey.plan` directly with `vec![(R1, B), …]` needs the macro. Walk through `test.rs` and rewrite each assertion that compares against a `vec![…]` of route/stop pairs.

Examples (refer to current line numbers):
- `reboarding_picks_correct_boarding_stop`: `assert_eq!(best.plan, vec![(R2, B), (R3, D)]);` → `assert_eq!(best.plan, plan!(tt; (R2, B), (R3, D)));`
- `direct_journey_single_route`: `assert_eq!(journeys[0].plan, vec![(Route::R1, Stop::C)]);` → `assert_eq!(journeys[0].plan, plan!(tt; (Route::R1, Stop::C)));`
- `exact_time_connection`: `assert_eq!(best.plan, vec![(Route::R1, Stop::B), (Route::R2, Stop::C)]);` → `plan!(…)`
- `two_transfer_journey`: same shape
- `footpath_enables_connection`: same shape
- `raptor_with_cache_matches_fresh_run`: the test uses `RaptorCache::<Route, Stop>::new()`. Replace with `RaptorCache::for_timetable(&tt)`. The test compares `b.plan` and `c.plan` directly, which is already `Vec<(RouteIdx, StopIdx)>` on both sides — no `plan!` needed.

For each test file, scan for `assert_eq!(.*plan,` and confirm every such assertion is rewritten.

- [ ] **Step 4: Update `raptor` call signatures**

The trait method `raptor(&self, transfers, tau, ps, pt)` now takes `StopIdx` for `ps` and `pt`. Test calls like `tt.raptor(3, 0, S, D)` (where `S` and `D` are enum variants) need `tt.stop_idx_of(&S)`/`tt.stop_idx_of(&D)`:

```rust
let journeys = tt.raptor(3, 0, tt.stop_idx_of(&S), tt.stop_idx_of(&D));
```

Walk every test calling `tt.raptor(…)`.

### Task A11: Migrate examples

**Files:**
- Modify: `raptor/examples/simple.rs`
- Modify: `raptor/examples/reboarding.rs`
- Modify: `raptor/examples/two_journeys.rs`
- Modify: `raptor/examples/gtfs-timetable.rs`
- Modify: `raptor/benches/raptor.rs` (call site update)

- [ ] **Step 1: Rewrite `examples/simple.rs`**

Replace the entire file with:

```rust
use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

// a single route with stops [0..10]
struct SingleRoute;

impl Timetable for SingleRoute {
    fn n_stops(&self) -> usize { 10 }
    fn n_routes(&self) -> usize { 1 }

    fn get_routes_serving_stop(&self, _stop: StopIdx) -> &[RouteIdx] {
        const ROUTES: [RouteIdx; 1] = [RouteIdx::new(0)];
        &ROUTES
    }

    fn get_earlier_stop(&self, _route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        if left.get() <= right.get() { left } else { right }
    }

    fn get_stops_after(&self, _route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        const STOPS: [StopIdx; 10] = [
            StopIdx::new(0), StopIdx::new(1), StopIdx::new(2), StopIdx::new(3),
            StopIdx::new(4), StopIdx::new(5), StopIdx::new(6), StopIdx::new(7),
            StopIdx::new(8), StopIdx::new(9),
        ];
        &STOPS[stop.get() as usize..]
    }

    fn get_arrival_time(&self, _trip: TripIdx, stop: StopIdx) -> Tau {
        (stop.get() as Tau) * 10
    }

    fn get_departure_time(&self, _trip: TripIdx, stop: StopIdx) -> Tau {
        (stop.get() as Tau) * 10 + 5
    }

    fn get_footpaths_from(&self, _stop: StopIdx) -> &[StopIdx] {
        &[]
    }

    fn get_earliest_trip(&self, _route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        (at < self.get_departure_time(TripIdx::new(0), stop)).then_some(TripIdx::new(0))
    }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let mock = SingleRoute;
    let journey = mock.raptor(10, 0, StopIdx::new(0), StopIdx::new(9));

    println!("{journey:#?}");
}
```

- [ ] **Step 2: Rewrite `examples/reboarding.rs`**

The current example uses `char` stops and `&'static str` routes. Rewrite to use `StopIdx`/`RouteIdx`/`TripIdx` directly. Replace the entire file with:

```rust
//! Example showing a multi-route RAPTOR query where a passenger reboards
//! a shared route at a later stop reached via a faster feeder route.

use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

// Stop indices: S=0, A=1, B=2, C=3, D=4
// Route indices: R1=0 (S->A), R2=1 (S->B), R3=2 (A->B->C->D)
// Trip indices: R1T1=0, R2T1=1, R3early=2, R3late=3
struct ReBoardingTimetable;

const S: StopIdx = StopIdx::new(0);
const A: StopIdx = StopIdx::new(1);
const B: StopIdx = StopIdx::new(2);
const C: StopIdx = StopIdx::new(3);
const D: StopIdx = StopIdx::new(4);

const R1: RouteIdx = RouteIdx::new(0);
const R2: RouteIdx = RouteIdx::new(1);
const R3: RouteIdx = RouteIdx::new(2);

const R1_T1: TripIdx = TripIdx::new(0);
const R2_T1: TripIdx = TripIdx::new(1);
const R3_EARLY: TripIdx = TripIdx::new(2);
const R3_LATE: TripIdx = TripIdx::new(3);

impl Timetable for ReBoardingTimetable {
    fn n_stops(&self) -> usize { 5 }
    fn n_routes(&self) -> usize { 3 }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        match stop.get() {
            0 => &[R1, R2],   // S
            1 => &[R1, R3],   // A
            2 => &[R2, R3],   // B
            3 | 4 => &[R3],   // C, D
            _ => &[],
        }
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        let order: &[StopIdx] = match route.get() {
            0 => &[S, A],
            1 => &[S, B],
            2 => &[A, B, C, D],
            _ => return left,
        };
        let l = order.iter().position(|&c| c == left).unwrap_or(99);
        let r = order.iter().position(|&c| c == right).unwrap_or(99);
        order[l.min(r)]
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        let order: &[StopIdx] = match route.get() {
            0 => &[S, A],
            1 => &[S, B],
            2 => &[A, B, C, D],
            _ => return &[],
        };
        let pos = order.iter().position(|&c| c == stop).unwrap_or(0);
        &order[pos..]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        match route.get() {
            0 => {
                let dep = match stop.get() {
                    0 => 0,
                    1 => 100,
                    _ => return None,
                };
                (at <= dep).then_some(R1_T1)
            }
            1 => {
                let dep = match stop.get() {
                    0 => 0,
                    2 => 30,
                    _ => return None,
                };
                (at <= dep).then_some(R2_T1)
            }
            2 => {
                let (early_dep, late_dep) = match stop.get() {
                    1 => (25, 105),
                    2 => (30, 110),
                    3 => (40, 120),
                    4 => (50, 130),
                    _ => return None,
                };
                if at <= early_dep {
                    Some(R3_EARLY)
                } else if at <= late_dep {
                    Some(R3_LATE)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        match (trip.get(), stop.get()) {
            (0, 1) => 100,                               // R1_T1 at A
            (1, 2) => 30,                                // R2_T1 at B
            (3, 2) => 110, (3, 3) => 120, (3, 4) => 130, // R3_LATE
            (2, 2) => 30,  (2, 3) => 40,  (2, 4) => 50,  // R3_EARLY
            _ => Tau::MAX,
        }
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        match (trip.get(), stop.get()) {
            (0, 0) => 0,                                 // R1_T1 at S
            (1, 0) => 0,                                 // R2_T1 at S
            (3, 1) => 105, (3, 2) => 110, (3, 3) => 120, // R3_LATE
            (2, 1) => 25,  (2, 2) => 30,  (2, 3) => 40,  // R3_EARLY
            _ => Tau::MAX,
        }
    }

    fn get_footpaths_from(&self, _: StopIdx) -> &[StopIdx] { &[] }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    println!("Query: S -> D, departure time 0");
    println!("Expected: S --(R2)--> B --(R3/early)--> D, arrives @ t=50\n");

    let timetable = ReBoardingTimetable;
    let journeys = timetable.raptor(3, 0, S, D);

    println!("{journeys:#?}");
}
```

- [ ] **Step 3: Rewrite `examples/two_journeys.rs`**

Replace the entire file with:

```rust
use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

const R0_STOPS: [StopIdx; 10] = [
    StopIdx::new(0), StopIdx::new(1), StopIdx::new(2), StopIdx::new(3),
    StopIdx::new(4), StopIdx::new(5), StopIdx::new(6), StopIdx::new(7),
    StopIdx::new(8), StopIdx::new(9),
];

const R1_STOPS: [StopIdx; 4] = [
    StopIdx::new(2), StopIdx::new(10), StopIdx::new(11), StopIdx::new(9),
];

struct TwoRoutes;

impl Timetable for TwoRoutes {
    fn n_stops(&self) -> usize { 12 }
    fn n_routes(&self) -> usize { 2 }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        let in_r0 = R0_STOPS.contains(&stop);
        let in_r1 = R1_STOPS.contains(&stop);
        match (in_r0, in_r1) {
            (true, true) => &[RouteIdx::new(0), RouteIdx::new(1)],
            (true, false) => &[RouteIdx::new(0)],
            (false, true) => &[RouteIdx::new(1)],
            (false, false) => &[],
        }
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        if route.get() == 0 {
            if left.get() <= right.get() { left } else { right }
        } else {
            let l = R1_STOPS.iter().position(|&a| a == left).unwrap();
            let r = R1_STOPS.iter().position(|&a| a == right).unwrap();
            R1_STOPS[l.min(r)]
        }
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        if route.get() == 0 {
            &R0_STOPS[stop.get() as usize..]
        } else {
            let pos = R1_STOPS.iter().position(|&a| a == stop).unwrap();
            &R1_STOPS[pos..]
        }
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        if route.get() == 0 {
            (at < self.get_departure_time(TripIdx::new(0), stop)).then_some(TripIdx::new(0))
        } else {
            (at < self.get_departure_time(TripIdx::new(1), stop)).then_some(TripIdx::new(1))
        }
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        if trip.get() == 0 {
            (stop.get() as Tau) * 10
        } else {
            let pos = R1_STOPS.iter().position(|&a| a == stop).unwrap();
            (pos + 2) * 10
        }
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        self.get_arrival_time(trip, stop) + 5
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        if stop.get() == 2 {
            const SELF: [StopIdx; 1] = [StopIdx::new(2)];
            &SELF
        } else {
            &[]
        }
    }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let mock = TwoRoutes;
    let journey = mock.raptor(10, 0, StopIdx::new(1), StopIdx::new(9));

    println!("{journey:#?}");
}
```

- [ ] **Step 4: Rewrite `examples/gtfs-timetable.rs`**

Replace the entire file with:

```rust
// Usage: cargo run --example gtfs-timetable <path_to_zip> <start_stop> <target_stop>

use gtfs_structures::Gtfs;
use humantime::format_duration;
use raptor::{Journey, Timetable, gtfs::GtfsTimetable};
use std::{env, time::Duration};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!(
            "Usage: {} <path_to_zip> <start_stop> <target_stop>",
            args[0]
        );
        std::process::exit(1);
    }

    let path = &args[1];
    let start = args[2].as_str();
    let target = args[3].as_str();

    let gtfs = Gtfs::new(path)?;
    let timetable = GtfsTimetable::new(&gtfs)?;

    let start_idx = timetable
        .stop_idx(start)
        .ok_or_else(|| anyhow::anyhow!("unknown start stop: {start}"))?;
    let target_idx = timetable
        .stop_idx(target)
        .ok_or_else(|| anyhow::anyhow!("unknown target stop: {target}"))?;

    let departure_time = 19 * 3600 + 15 * 60;
    let journeys = timetable.raptor(10, departure_time, start_idx, target_idx);

    if journeys.is_empty() {
        println!("No journeys found.");
        return Ok(());
    }

    for (i, journey) in journeys.iter().enumerate() {
        let travel_time = Duration::from_secs((journey.arrival - departure_time) as u64);
        println!("Journey {} ({}):", i + 1, format_duration(travel_time));
        print_journey(&gtfs, &timetable, journey, start);
        println!();
    }

    log::debug!("{journeys:#?}");

    Ok(())
}

fn print_journey<'gtfs>(
    gtfs: &'gtfs Gtfs,
    timetable: &GtfsTimetable<'gtfs>,
    journey: &Journey,
    start: &'gtfs str,
) {
    let start_name = gtfs
        .stops
        .get(start)
        .and_then(|s| s.name.as_deref())
        .unwrap_or(start);
    print!("\"{}\" ", start_name);

    for (route, stop) in journey.plan.iter() {
        let route_id = timetable.route_id(*route);
        let route_name = gtfs
            .routes
            .get(route_id)
            .and_then(|r| r.short_name.as_deref().or(r.long_name.as_deref()))
            .unwrap_or(route_id);

        let raw_stop_id = timetable.stop_id(*stop);
        let stop_name = gtfs
            .stops
            .get(raw_stop_id)
            .and_then(|s| s.name.as_deref())
            .unwrap_or(raw_stop_id);

        print!("-[\"{}\"]-> \"{}\" ", route_name, stop_name);
    }
}
```

- [ ] **Step 5: Update `benches/raptor.rs` call sites**

The bench file calls `tt.raptor(3, 0, 0, stops - 1)` etc. with `usize` source/target. After the change, those become `StopIdx`. Walk through `benches/raptor.rs`; each `b.iter(|| tt.raptor(…, source, target))` needs `tt.stop_idx_of(&source)` and `tt.stop_idx_of(&target)`.

Example (linear bench):

```rust
group.bench_with_input(
    BenchmarkId::new("raptor", format!("{stops}s_{trips}t")),
    &tt,
    |b, tt| {
        let source = tt.stop_idx_of(&0usize);
        let target = tt.stop_idx_of(&(stops - 1));
        b.iter(|| tt.raptor(3, 0, source, target))
    },
);
```

Apply analogous updates in `bench_grid`, `bench_hub_spoke`, `bench_transfer_scaling`, and the chain bench. Hoist the `stop_idx_of` calls outside `b.iter` so they don't measure interning lookups.

### Task A12: Migrate the proptest harness

**Files:**
- Modify: `raptor-proptest/src/lib.rs` — drop `raptor_front`'s generics; update `run_property` to translate `u8` keys to `StopIdx` via the timetable's interning lookup; update tests in the inner `mod tests` to use the new `Journey` type.

- [ ] **Step 1: Update `raptor_front`**

Replace the existing function with:

```rust
use raptor::{Journey, RouteIdx, StopIdx};

/// Project the algorithm's `Vec<Journey>` to a Pareto front of
/// `(arrival, trip_count)`, sorted by trip count ascending, keeping only
/// points where arrival is *strictly* less than the best seen so far.
pub fn raptor_front(journeys: &[Journey]) -> BTreeSet<(u16, u8)> {
    let mut points: Vec<(u16, u8)> = journeys
        .iter()
        .map(|j| {
            let arr = u16::try_from(j.arrival)
                .expect("arrival exceeds u16::MAX — generator range exceeded?");
            let k = u8::try_from(j.plan.len())
                .expect("plan length exceeds u8::MAX — should never happen");
            (arr, k)
        })
        .collect();
    points.sort_by_key(|&(t, k)| (k, t));
    let mut best = u16::MAX;
    let mut out = BTreeSet::new();
    for (arr, k) in points {
        if arr < best {
            best = arr;
            out.insert((arr, k));
        }
    }
    out
}
```

(Top-of-file `use raptor::Journey;` becomes `use raptor::{Journey, RouteIdx, StopIdx};` — remove the redundant `use raptor::Journey;` line if both exist.)

- [ ] **Step 2: Update `run_property`**

Replace the existing function with:

```rust
#[cfg(test)]
fn run_property(tc: &hegel::TestCase, spec: &spec::NetworkSpec) {
    let timetable = spec::render(spec);
    let ps_idx = timetable.stop_idx_of(&spec.query.ps);
    let pt_idx = timetable.stop_idx_of(&spec.query.pt);
    let ours = timetable.raptor(
        spec.query.max_transfers as usize,
        spec.query.tau as usize,
        ps_idx,
        pt_idx,
    );
    let theirs = reference::reference_solve(
        spec,
        spec.query.ps,
        spec.query.pt,
        spec.query.tau,
        spec.query.max_transfers,
    );
    let our_front = raptor_front(&ours);
    if our_front != theirs {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("raptor:     {:?}", ours));
        tc.note(&format!("ours_front: {:?}", our_front));
        tc.note(&format!("theirs:     {:?}", theirs));
    }
    assert_eq!(our_front, theirs);
}
```

- [ ] **Step 3: Update the inner `#[cfg(test)] mod tests` block**

Replace the existing tests block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use raptor::{Journey, RouteIdx, StopIdx};

    fn j(arrival: usize, plan: Vec<(u32, u32)>) -> Journey {
        Journey {
            plan: plan
                .into_iter()
                .map(|(r, s)| (RouteIdx::new(r), StopIdx::new(s)))
                .collect(),
            arrival,
        }
    }

    #[test]
    fn raptor_front_drops_dominated_higher_trip_journeys() {
        let journeys = vec![
            j(100, vec![(0, 1)]),
            j(100, vec![(0, 2), (1, 3)]),
            j(80, vec![(0, 2), (1, 3)]),
        ];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(100u16, 1u8), (80u16, 2u8)].into_iter().collect();
        assert_eq!(f, expected);
    }

    #[test]
    fn raptor_front_empty_input_is_empty() {
        let f = raptor_front(&[]);
        assert!(f.is_empty());
    }

    #[test]
    fn raptor_front_strict_monotonicity_drops_ties() {
        let journeys = vec![j(50, vec![(0, 1)]), j(50, vec![(0, 1), (1, 2)])];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(50u16, 1u8)].into_iter().collect();
        assert_eq!(f, expected);
    }
}
```

- [ ] **Step 4: Update `spec.rs` tests that exercise `Timetable` accessors**

Two tests in `raptor-proptest/src/spec.rs` (`render_single_route_two_stops_one_trip` and `render_emits_transitively_closed_footpaths`) call accessors on the rendered `SimpleTimetable`. They currently pass `0u8`/`1u8` as `Stop` because the trait's associated type was `u8`. Now the accessors take `StopIdx` and return `&[…]` (not `Cow<…>`).

Rewrite the bodies:

```rust
#[test]
fn render_single_route_two_stops_one_trip() {
    use raptor::Timetable;
    let spec = NetworkSpec {
        n_stops: 2,
        routes: vec![RouteSpec {
            stop_sequence: vec![0, 1],
            trips: vec![TripSpec {
                first_dep: 100,
                leg_durations: vec![20],
                dwell_times: vec![5, 0],
            }],
        }],
        footpaths: vec![],
        query: QuerySpec {
            ps: 0,
            pt: 1,
            tau: 0,
            max_transfers: 1,
        },
    };
    let tt = render(&spec);

    let s0 = tt.stop_idx_of(&0u8);
    let s1 = tt.stop_idx_of(&1u8);
    let r0 = tt.route_idx_of(&0u8);

    let routes_at_0 = tt.get_routes_serving_stop(s0);
    assert_eq!(routes_at_0, &[r0]);

    let trip = tt.get_earliest_trip(r0, 0, s0).expect("trip exists");
    assert_eq!(tt.get_arrival_time(trip, s0), 100);
    assert_eq!(tt.get_departure_time(trip, s0), 105);
    assert_eq!(tt.get_arrival_time(trip, s1), 125);
    assert_eq!(tt.get_departure_time(trip, s1), 125);
}

#[test]
fn render_emits_transitively_closed_footpaths() {
    use raptor::Timetable;
    let spec = NetworkSpec {
        n_stops: 3,
        routes: vec![],
        footpaths: vec![
            FootpathSpec { from: 0, to: 1, walk_time: 3 },
            FootpathSpec { from: 1, to: 2, walk_time: 4 },
        ],
        query: QuerySpec {
            ps: 0,
            pt: 2,
            tau: 0,
            max_transfers: 1,
        },
    };
    let tt = render(&spec);

    let s0 = tt.stop_idx_of(&0u8);
    let s1 = tt.stop_idx_of(&1u8);
    let s2 = tt.stop_idx_of(&2u8);

    let from_0: Vec<raptor::StopIdx> = tt.get_footpaths_from(s0).to_vec();
    assert!(from_0.contains(&s1), "direct A->B");
    assert!(from_0.contains(&s2), "transitive A->C must be present");
    assert_eq!(tt.get_transfer_time(s0, s2), 7);
}
```

Note: `stop_idx_of` only returns an index for keys actually inserted. The renderer in `spec.rs::render` adds every stop reachable via routes/footpaths, but does not call `intern_stop` on unrouted/unwalked stops. `s2` in the second test is reachable via the footpath chain so it's interned; `s0`/`s1`/`s2` in the second test are all touched by `tt.footpath`. ✅

### Task A13: Verify and commit Phase A

- [ ] **Step 1: Format**

```bash
cargo fmt
```

- [ ] **Step 2: Build**

```bash
cargo build --workspace --all-targets
```

Expected: clean build. If errors remain, fix them — likely candidates are:
- A `Cow::Borrowed` conversion that didn't get rewritten.
- A missing `Hash` derive on a test enum.
- A `tt.raptor(…)` call passing a non-`StopIdx`.

- [ ] **Step 3: Run unit tests**

```bash
cargo nextest r -p raptor
```

Expected: PASS.

- [ ] **Step 4: Run proptest harness — the contract**

```bash
cargo nextest r -p raptor-proptest
```

Expected: all three layers (`layer1_matches_reference`, `layer2_matches_reference`, `layer3_matches_reference`) PASS along with the unit tests. **If anything fails, do not proceed to Phase B.** The harness is the safety net; debug here.

- [ ] **Step 5: Run clippy**

```bash
cargo clippy --workspace --all-targets
```

Address any lints. The `pub(crate) fn idx` helpers may trigger `dead_code` warnings if a particular adapter doesn't use them — `#[allow(dead_code)]` on the helper is fine (we know it'll be used by the storage-rewrite phase if not now).

- [ ] **Step 6: Commit Phase A**

```bash
jj fix
jj commit -m "Phase 1.1: collapse Timetable trait, intern to dense u32 indices"
```

---

## Phase B — Storage rewrite (1.2 + 1.4)

This phase swaps `BTreeMap`/`BTreeSet` algorithm storage for `Vec<Vec<Tau>>`, `FixedBitSet`, and a sparse-set route queue. The `Timetable` trait shape from Phase A is unchanged. Adapters and tests don't move.

### Task B1: Add the `fixedbitset` dependency

**Files:**
- Modify: `raptor/Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `raptor/Cargo.toml`'s `[dependencies]` section, add:

```toml
fixedbitset = "0.5"
```

Run `cargo build -p raptor` to confirm it resolves.

### Task B2: Rewrite `RaptorCache` fields

**Files:**
- Modify: `raptor/src/lib.rs`

- [ ] **Step 1: Update imports**

At the top of `raptor/src/lib.rs`, add:

```rust
use fixedbitset::FixedBitSet;
```

- [ ] **Step 2: Replace the `RaptorCache` struct and impl**

Replace the existing `pub struct RaptorCache { … }` and its `impl` block with:

```rust
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

    /// Per-round route queue. `q_entry[r.idx()] = Some(boarding_stop)` when
    /// route r has been entered this round; `q_routes` is the dense list
    /// of routes that have entries (for cheap iteration without scanning
    /// `n_routes` empty slots).
    q_entry: Vec<Option<StopIdx>>,
    q_routes: Vec<RouteIdx>,

    /// Scratch buffer for footpath relaxation output.
    walked_buf: Vec<StopIdx>,
}

impl RaptorCache {
    pub fn for_timetable<T: Timetable + ?Sized>(tt: &T) -> Self {
        Self::with_capacity(tt.n_stops() as u32, tt.n_routes() as u32)
    }

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
```

### Task B3: Rewrite `relax_footpaths_round`

**Files:**
- Modify: `raptor/src/lib.rs`

- [ ] **Step 1: Replace the function**

Replace the existing `fn relax_footpaths_round` (post-Phase-A) with:

```rust
fn relax_footpaths_round<T: Timetable + ?Sized>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<Tau>],
    best_arrival: &mut [Tau],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt: StopIdx,
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
                let cur_best = best_arrival[p_dash.idx()];
                if via_walk < cur_best {
                    best_arrival[p_dash.idx()] = via_walk;
                }
                if via_walk < best_arrival[pt.idx()] {
                    out.push(p_dash);
                }
            }
        }
    }
}
```

### Task B4: Rewrite `raptor_with_cache` body

**Files:**
- Modify: `raptor/src/lib.rs`

- [ ] **Step 1: Replace the algorithm body**

Replace the body of `raptor_with_cache` in the `Timetable` trait with:

```rust
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
        ..
    } = cache;

    labels[0][ps.idx()] = tau;
    best_arrival[ps.idx()] = tau;
    marked_stops.insert(ps.idx());

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
    for s in walked_buf.drain(..) {
        marked_stops.insert(s.idx());
    }

    for k in 1..=transfers {
        // Carry forward labels[k-1] into labels[k].
        let (prev_labels, this_labels) = labels.split_at_mut(k);
        let src = &prev_labels[k - 1];
        let dst = &mut this_labels[0];
        dst.copy_from_slice(src);

        // Build the route queue for this round.
        for stop_bit in marked_stops.ones() {
            let marked_stop = StopIdx::new(stop_bit as u32);
            for &route in self.get_routes_serving_stop(marked_stop) {
                match q_entry[route.idx()] {
                    None => {
                        q_entry[route.idx()] = Some(marked_stop);
                        q_routes.push(route);
                    }
                    Some(prev) => {
                        let earlier = self.get_earlier_stop(route, marked_stop, prev);
                        q_entry[route.idx()] = Some(earlier);
                    }
                }
            }
        }

        marked_stops.clear();

        for &route in q_routes.iter() {
            let p = q_entry[route.idx()].expect("route in q_routes must have an entry");
            let mut current_trip: Option<TripIdx> = None;
            let mut boarding_stop = p;

            for &pi in self.get_stops_after(route, p) {
                if let Some(arr) = current_trip.map(|trip| self.get_arrival_time(trip, pi)) {
                    let best_to_pt = best_arrival[pt.idx()];
                    let best_to_pi = best_arrival[pi.idx()];
                    let time_to_beat = best_to_pi.min(best_to_pt);

                    if arr < time_to_beat {
                        board_detail.insert(
                            (k, pi),
                            Step::Boarded { from: boarding_stop, route },
                        );
                        labels[k][pi.idx()] = arr;
                        best_arrival[pi.idx()] = arr;
                        marked_stops.insert(pi.idx());
                    }
                }

                let t_prev_pi = labels[k - 1][pi.idx()];
                let dep_at_pi = current_trip
                    .map(|trip| self.get_departure_time(trip, pi))
                    .unwrap_or(Tau::MAX);
                if t_prev_pi <= dep_at_pi {
                    current_trip = self.get_earliest_trip(route, t_prev_pi, pi);
                    boarding_stop = pi;
                }
            }
        }

        // Sparse-set reset of the route queue.
        for r in q_routes.drain(..) {
            q_entry[r.idx()] = None;
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
        for s in walked_buf.drain(..) {
            marked_stops.insert(s.idx());
        }

        if marked_stops.count_ones(..) == 0 {
            break;
        }
    }

    let plans = reconstruct_journey(board_detail, ps, pt, transfers);

    let mut journeys: Vec<Journey> = plans
        .into_iter()
        .map(|plan| {
            let arrival = labels[plan.len()][pt.idx()];
            Journey { plan, arrival }
        })
        .collect();

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
```

Notes on the rewrite:
- `labels[k] = labels[k-1].clone()` becomes `dst.copy_from_slice(src)` after splitting the labels Vec to satisfy the borrow checker.
- Bitset iteration uses `marked_stops.ones()` returning `usize` bit positions; reconstruct `StopIdx::new(bit as u32)`.
- `marked_stops.is_empty()` would be a method on `FixedBitSet` but the equivalent is `marked_stops.count_ones(..) == 0`.

### Task B5: Verify and commit Phase B

- [ ] **Step 1: Format and build**

```bash
cargo fmt
cargo build --workspace --all-targets
```

- [ ] **Step 2: Unit tests**

```bash
cargo nextest r -p raptor
```

- [ ] **Step 3: Proptest harness**

```bash
cargo nextest r -p raptor-proptest
```

All layers must remain green. Debug here if not.

- [ ] **Step 4: Clippy**

```bash
cargo clippy --workspace --all-targets
```

- [ ] **Step 5: Commit**

```bash
jj fix
jj commit -m "Phase 1.2 + 1.4: Vec<Vec<Tau>> labels, FixedBitSet marked stops, sparse-set route queue"
```

---

## Phase C — GTFS access path (1.3)

This phase materialises per-route arrival/departure tables in `GtfsTimetable` and reimplements `get_arrival_time`/`get_departure_time` against them. Eliminates the `find_stop_time` linear scan.

### Task C1: Add the arrival/departure tables to the struct

**Files:**
- Modify: `raptor/src/gtfs.rs`

- [ ] **Step 1: Add the fields**

In the `GtfsTimetable` struct, add after `trips_for_route`:

```rust
    /// arrival_times[route.idx()][stop_pos][trip_pos] = Tau
    arrival_times: Vec<Vec<Vec<Tau>>>,
    /// departure_times[route.idx()][stop_pos][trip_pos] = Tau
    departure_times: Vec<Vec<Vec<Tau>>>,
```

### Task C2: Populate the tables in `new`

**Files:**
- Modify: `raptor/src/gtfs.rs`

- [ ] **Step 1: Populate during the inner loop**

Inside the `for ((gtfs_route_id, stop_seq), trips) in groups` loop, *after* `trips_for_route.push(sub_trip_idxs);`, add:

```rust
                // Per-route arrival/departure tables: shape [stop_pos][trip_pos].
                let n_stops_in_route = stop_seq.len();
                let n_trips_in_route = sub_group.len();
                let mut arr_table: Vec<Vec<Tau>> =
                    vec![vec![Tau::MAX; n_trips_in_route]; n_stops_in_route];
                let mut dep_table: Vec<Vec<Tau>> =
                    vec![vec![Tau::MAX; n_trips_in_route]; n_stops_in_route];
                for (trip_pos, trip_id) in sub_group.iter().enumerate() {
                    let trip = gtfs.get_trip(trip_id).expect("validated above");
                    for (stop_pos, st) in trip.stop_times.iter().enumerate() {
                        if let Some(a) = st.arrival_time {
                            arr_table[stop_pos][trip_pos] = a as Tau;
                        }
                        let d = st.departure_time.expect("validated at construction");
                        dep_table[stop_pos][trip_pos] = d as Tau;
                    }
                }
                arrival_times.push(arr_table);
                departure_times.push(dep_table);
```

Add the matching `let mut arrival_times: Vec<Vec<Vec<Tau>>> = Vec::new();` and `let mut departure_times: Vec<Vec<Vec<Tau>>> = Vec::new();` declarations alongside `route_ids`/`trips_for_route`/etc.

Add `arrival_times` and `departure_times` to the final `Ok(Self { … })` constructor.

### Task C3: Reimplement `get_arrival_time` / `get_departure_time` / `get_earliest_trip`

**Files:**
- Modify: `raptor/src/gtfs.rs`

- [ ] **Step 1: Remove `find_stop_time`**

Delete the `fn find_stop_time` helper.

- [ ] **Step 2: Rewrite the three accessors**

Replace `get_earliest_trip`, `get_arrival_time`, and `get_departure_time` in the `Timetable` impl with:

```rust
    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        let trips = &self.trips_for_route[route.idx()];
        let stop_pos = self.stops_for_route[route.idx()]
            .iter()
            .position(|&s| s == stop)?;
        let dep_row = &self.departure_times[route.idx()][stop_pos];
        let idx = dep_row.partition_point(|&dep| dep < at);
        trips.get(idx).copied()
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        // Find the route this trip belongs to. We don't store a reverse map,
        // so we look up via the route's trips_for_route entry.
        let route_idx = self
            .trips_for_route
            .iter()
            .position(|trips| trips.contains(&trip))
            .expect("trip belongs to some route");
        let stop_pos = self.stops_for_route[route_idx]
            .iter()
            .position(|&s| s == stop)
            .expect("stop on route");
        let trip_pos = self.trips_for_route[route_idx]
            .iter()
            .position(|&t| t == trip)
            .expect("trip on route");
        self.arrival_times[route_idx][stop_pos][trip_pos]
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let route_idx = self
            .trips_for_route
            .iter()
            .position(|trips| trips.contains(&trip))
            .expect("trip belongs to some route");
        let stop_pos = self.stops_for_route[route_idx]
            .iter()
            .position(|&s| s == stop)
            .expect("stop on route");
        let trip_pos = self.trips_for_route[route_idx]
            .iter()
            .position(|&t| t == trip)
            .expect("trip on route");
        self.departure_times[route_idx][stop_pos][trip_pos]
    }
```

The `get_arrival_time`/`get_departure_time` rewrites still scan `trips_for_route` to find the route — to make these truly O(1), add a reverse map.

- [ ] **Step 3: Add `route_for_trip` reverse map**

Add a field on `GtfsTimetable`:

```rust
    /// route_for_trip[trip.idx()] = (route_idx, position-within-route)
    route_for_trip: Vec<(RouteIdx, usize)>,
```

Populate during construction: inside the `for (trip_pos, trip_id) in sub_group.iter().enumerate()` loop in `new`, when assigning `trip_idx`, also extend `route_for_trip` (the trip indices are dense and assigned in order, so a `Vec` indexed by `trip_idx.idx()` works):

```rust
                while route_for_trip.len() <= trip_idx.idx() {
                    route_for_trip.push((route_idx, 0));
                }
                route_for_trip[trip_idx.idx()] = (route_idx, /* trip_pos within sub_group */ sub_trip_idxs.len() - 1);
```

(The `sub_trip_idxs.len() - 1` works because we just pushed `trip_idx` to it.)

Declare `let mut route_for_trip: Vec<(RouteIdx, usize)> = Vec::new();` alongside the other declarations and add it to the final `Ok(Self { … })`.

- [ ] **Step 4: Use `route_for_trip` in the accessors**

Replace the bodies of `get_arrival_time` and `get_departure_time`:

```rust
    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let (route_idx, trip_pos) = self.route_for_trip[trip.idx()];
        let stop_pos = self.stops_for_route[route_idx.idx()]
            .iter()
            .position(|&s| s == stop)
            .expect("stop on route");
        self.arrival_times[route_idx.idx()][stop_pos][trip_pos]
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let (route_idx, trip_pos) = self.route_for_trip[trip.idx()];
        let stop_pos = self.stops_for_route[route_idx.idx()]
            .iter()
            .position(|&s| s == stop)
            .expect("stop on route");
        self.departure_times[route_idx.idx()][stop_pos][trip_pos]
    }
```

(`stop_pos` is still a linear scan; further optimisation — flattening to `[route][stop_pos][trip_pos]` access via a reverse `(route, stop) -> stop_pos` map — is left for a later pass if profiling shows it as a hot spot.)

### Task C4: Verify and commit Phase C

- [ ] **Step 1: Format and build**

```bash
cargo fmt
cargo build --workspace --all-targets
```

- [ ] **Step 2: Run all tests**

```bash
cargo nextest r --workspace
```

- [ ] **Step 3: Clippy**

```bash
cargo clippy --workspace --all-targets
```

- [ ] **Step 4: Quick smoke test against the bundled GTFS**

Run the GTFS example to confirm construction and a query work end-to-end:

```bash
cargo run --release --example gtfs-timetable -- aux/dmrc_gtfs.zip <stop_id_a> <stop_id_b>
```

(Pick any two stop IDs from `aux/dmrc_gtfs.zip`. If unsure, run `cargo run --example gtfs-timetable -- aux/dmrc_gtfs.zip foo bar` and let the "unknown start stop: foo" error guide you to a real ID.)

- [ ] **Step 5: Commit**

```bash
jj fix
jj commit -m "Phase 1.3: per-route arrival/departure tables in GtfsTimetable"
```

---

## Phase D — Documentation and CHANGELOG

### Task D1: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the "Implementing Timetable" section**

Read the current `README.md`. Locate the section describing the `Timetable` trait (associated types `Stop`/`Route`/`Trip`). Rewrite to describe the new shape: index newtypes, `n_stops`/`n_routes`, `&[T]` returns, default `raptor` method. Update any code samples in the README that show `Journey<R, S>` or `Cow<…>`-returning methods.

- [ ] **Step 2: Update the GTFS example narrative**

The README's GTFS section likely shows constructing a `GtfsTimetable` and calling `tt.raptor(…, &str_start, &str_target)`. Update to show:

```rust
let start_idx = tt.stop_idx(start_str).expect("unknown stop");
let target_idx = tt.stop_idx(target_str).expect("unknown stop");
let journeys = tt.raptor(10, departure_time, start_idx, target_idx);
```

- [ ] **Step 3: Update the "Performance: reusing a RaptorCache" section**

The README currently shows `RaptorCache::new()`. Update to:

```rust
let mut cache = RaptorCache::for_timetable(&tt);
let journeys = tt.raptor_with_cache(&mut cache, transfers, tau, ps_idx, pt_idx);
```

### Task D2: Update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Prepend the v0.4.0 entry**

Insert at the top (after the document title, before the `## [0.3.0]` heading):

```markdown
## [0.4.0] — 2026-05-04

This release closes the data-representation half of Phase 1: the
algorithm's hot loop is now branch-friendly array indexing, the GTFS
adapter pre-computes per-route departure/arrival tables, and the
`Timetable` trait is non-generic.

### Breaking changes

- The `Timetable` trait no longer has associated `Stop`/`Route`/`Trip`
  types. All identifiers are now newtypes around `u32`: [`StopIdx`],
  [`RouteIdx`], [`TripIdx`]. Implementors must add `n_stops()` and
  `n_routes()` methods; slice-returning accessors return `&[T]` instead
  of `Cow<[T]>`.
- `Journey` is now non-generic: `Journey { plan: Vec<(RouteIdx, StopIdx)>,
  arrival: Tau }`.
- `RaptorCache` is non-generic and constructed via
  [`RaptorCache::for_timetable`] (or [`RaptorCache::with_capacity`] for
  the count-only path). Reusing a cache against a differently-sized
  timetable now panics on entry to `raptor_with_cache`.
- `GtfsTimetable`'s associated types are gone; lookup string IDs via
  [`GtfsTimetable::stop_id`] / [`GtfsTimetable::route_id`] /
  [`GtfsTimetable::trip_id`] and inverse-lookup via [`GtfsTimetable::stop_idx`]
  / [`GtfsTimetable::route_idx`] / [`GtfsTimetable::trip_idx`].
- `gtfs::RouteId` is removed; the synthetic-route concept now lives on
  [`RouteIdx`] with the same semantics.
- `SimpleTimetable<S, R, T>` has new generic bounds (`Hash + Eq + Clone`)
  and now interns its keys to dense `u32` indices. Construction API is
  unchanged; tests asserting on plans use the new
  [`SimpleTimetable::stop_idx_of`] / [`SimpleTimetable::route_idx_of`]
  helpers (and the in-test `plan!` macro).

### Performance

- Round labels are now `Vec<Vec<Tau>>` indexed by `(round, stop_idx)`
  rather than `Vec<BTreeMap<Stop, Tau>>` — all label reads/writes in the
  hot loop are array indexing.
- Marked stops are a [`fixedbitset::FixedBitSet`] sized to `n_stops`;
  insertion is a single bit write, iteration walks set bits.
- The per-round route queue is a sparse-set pair (`Vec<Option<StopIdx>>`
  + a dense `Vec<RouteIdx>`) instead of `BTreeMap<RouteIdx, StopIdx>`,
  giving `O(distinct routes touched)` reset cost per round.
- `GtfsTimetable` now holds `arrival_times[route][stop_pos][trip_pos]`
  and `departure_times[…]` arrays computed once at construction;
  `get_arrival_time` / `get_departure_time` are no longer
  `O(stops_in_route)` per call.

### Added

- New dependency: `fixedbitset = "0.5"` (used for marked-stops bitset).
- [`GtfsTimetable::routes_for_gtfs_id`] to enumerate every synthetic
  [`RouteIdx`] derived from a given GTFS `route_id`.
```

### Task D3: Bump the version

**Files:**
- Modify: `raptor/Cargo.toml`

- [ ] **Step 1: Bump to 0.4.0**

In `raptor/Cargo.toml`, change `version = "0.3.0"` to `version = "0.4.0"`.

### Task D4: Final verification and commit

- [ ] **Step 1: Build and test**

```bash
cargo build --workspace --all-targets
cargo nextest r --workspace
cargo clippy --workspace --all-targets
cargo fmt
```

- [ ] **Step 2: Commit**

```bash
jj fix
jj commit -m "Release 0.4.0: docs and CHANGELOG for Phase 1 data-representation"
```

---

## Self-review notes

**Spec coverage:**
- 1.1 (interning + new trait shape) → Phase A
- 1.2 (Vec<Vec<Tau>> labels) → Phase B (Tasks B2, B4)
- 1.3 (per-route arr/dep tables) → Phase C
- 1.4 (FixedBitSet marked stops + sparse-set route queue) → Phase B (Tasks B2, B3, B4)
- Display lookup API on `GtfsTimetable` → Task A7 step 4
- Inverse lookup API → Task A7 step 4
- `routes_for_gtfs_id` → Task A7 step 4
- `SimpleTimetable` interning + `stop_idx_of`/`route_idx_of` → Task A8
- Test `plan!` macro → Task A10
- Proptest harness migration → Task A12
- Examples migration → Task A11
- Bench call-site update → Task A11 step 5
- README + CHANGELOG + version bump → Phase D

**Type consistency:**
- `StopIdx::new`/`get`/`idx` are stable across all tasks.
- `RaptorCache::for_timetable<T: Timetable + ?Sized>` matches the trait's `&self` receiver.
- `Journey` is concrete from Task A2 onwards; tests in A12 construct it via `Journey { plan: …, arrival: … }` directly.
- `BoardingTree` is `BTreeMap<(K, StopIdx), Step>` from Task A2 onwards; both Phase A and Phase B reference it the same way.

**Risk areas to watch in execution:**
- The `get_arrival_time`/`get_departure_time` accessors in Phase C use a reverse map (`route_for_trip`) populated incrementally during route construction. The order of operations matters — the field must be in scope inside the trip-id-assignment loop. Task C3 step 3 shows the precise placement.
- Phase A's `RaptorCache` keeps `BTreeMap<StopIdx, Tau>` labels deliberately. The destructure pattern in `raptor_with_cache` uses `..` to allow Phase B to add fields without breaking Phase A's body.
- The sparse-set route queue in Phase B requires careful borrow management because `q_routes` is iterated while `q_entry` is read. The implementation pattern (`for &route in q_routes.iter()` then `q_entry[route.idx()].expect(…)`) is the right shape — read but don't mutate `q_entry` inside the iteration.
