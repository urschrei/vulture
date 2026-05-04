# Phase 1 (1.1 + 1.2 + 1.3 + 1.4) — data-representation rewrite

**Date:** 2026-05-04
**Status:** Approved (brainstorming complete; implementation plan to follow)
**Roadmap items covered:** 1.1 (interning), 1.2 (`Vec<Vec<Tau>>` labels), 1.3 (per-route departure tables), 1.4 (`FixedBitSet` for marked stops). Out of scope here: 1.6 (parallel queries — docs + integration test, separate change), 1.7 (real-feed benchmarks — separate change).

## Goal

Rewrite the algorithm's data plane around dense `u32` indices and contiguous `Vec`/bitset storage. Eliminate `BTreeMap`-keyed lookups from the hot loop and the `find_stop_time` linear scan from `get_arrival_time`/`get_departure_time`. The proptest harness is the safety net: every commit must keep `cargo nextest r -p raptor-proptest` green.

This is a breaking rewrite of the public API. Acceptable per project direction ("nobody's going to use this if it's slow").

## Design summary

- `Timetable` becomes a non-generic trait. All identifiers are newtypes around `u32`: `StopIdx`, `RouteIdx`, `TripIdx`. The trait gains `n_stops()` and `n_routes()`.
- The algorithm operates on `Vec<Vec<Tau>>` labels (round × stop) and a `FixedBitSet` of marked stops sized to `n_stops`. The per-round route queue becomes a sparse-set pair (`q_entry: Vec<Option<StopIdx>>` + `q_routes: Vec<RouteIdx>`).
- `RaptorCache` is non-generic, pre-bound by counts (not by reference), and asserts size match on every query.
- `GtfsTimetable` interns stops/routes/trips at `::new` time. Materialises per-route `arrival_times[route][stop_pos][trip_pos]` and `departure_times[...]` so `get_arrival_time`/`get_departure_time` become `O(1)`.
- `SimpleTimetable` becomes an interning builder generic over key types (`<S, R, T>` with `Default = u32`). Internal storage is dense u32 throughout. Tests keep using enums-as-keys via the builder.
- The proptest harness keeps generating `u8`/`u16` keys; `SimpleTimetable` interns them. The reference solver loses its generic params and reads through the `Timetable` trait.
- `Journey` becomes concrete: `Journey { plan: Vec<(RouteIdx, StopIdx)>, arrival: Tau }`.
- `gtfs::RouteId` is removed; the synthetic-route concept now lives on `RouteIdx` with semantics documented in the GTFS module docs.

## The new `Timetable` trait

```rust
pub trait Timetable {
    /// Number of stops. Stop indices are in 0..n_stops().
    fn n_stops(&self) -> usize;
    /// Number of routes (post-pattern-splitting). Route indices are in 0..n_routes().
    fn n_routes(&self) -> usize;

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx];
    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx;
    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx];

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx>;
    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau;
    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau;

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx];
    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau { 1 }

    fn raptor(&self, transfers: usize, tau: Tau, ps: StopIdx, pt: StopIdx) -> Vec<Journey> {
        let mut cache = RaptorCache::for_timetable(self);
        self.raptor_with_cache(&mut cache, transfers, tau, ps, pt)
    }
    fn raptor_with_cache(
        &self,
        cache: &mut RaptorCache,
        transfers: usize,
        tau: Tau,
        ps: StopIdx,
        pt: StopIdx,
    ) -> Vec<Journey> { /* algorithm body */ }
}
```

Changes from the current trait:

- No associated types. `Stop`/`Route`/`Trip` are gone.
- All slice returns are `&[T]`, not `Cow<'_, [T]>`. Adapters can always borrow from internal storage in the indexed world.
- `n_trips()` is omitted; the algorithm doesn't need it. Can be added later if a use case appears.
- The trait-level docs (footpath transitivity, no-overtaking within a route) carry forward verbatim — those are about semantics, not types.

## Newtype index types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StopIdx(u32);

impl StopIdx {
    pub const fn new(n: u32) -> Self { Self(n) }
    pub const fn get(self) -> u32 { self.0 }
    #[inline]
    pub(crate) fn idx(self) -> usize { self.0 as usize }
}

impl fmt::Display for StopIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

impl From<u32> for StopIdx { fn from(n: u32) -> Self { Self(n) } }
impl From<StopIdx> for u32 { fn from(s: StopIdx) -> Self { s.0 } }
```

`RouteIdx` and `TripIdx` are identical in shape (separate types for type safety; the small boilerplate is worth not being able to pass a stop where a route is expected).

- `idx(self) -> usize` is `pub(crate)` — used inside the algorithm and the in-tree adapters for `Vec` indexing. Public callers go through `.get() -> u32` and convert as needed.
- No `From<usize>`. Forces explicit conversion at the boundary, surfaces overflow.

## `RaptorCache` and the algorithm's data plane

```rust
pub struct RaptorCache {
    n_stops: u32,
    n_routes: u32,

    /// labels[k][stop.idx()] = earliest arrival at stop with at most k trips.
    /// Tau::MAX sentinel for "unreached".
    labels: Vec<Vec<Tau>>,

    /// τ* — best arrival at each stop across all rounds.
    best_arrival: Vec<Tau>,

    /// Boarding tree for journey reconstruction. Sparse — keyed by (round, stop)
    /// only for stops actually reached. Dense Vec would be (transfers+1) × n_stops
    /// of mostly-empty entries; reconstruction does point lookups, not range scans.
    board_detail: BTreeMap<(K, StopIdx), Step>,

    /// Bitset of marked stops, sized to n_stops.
    marked_stops: FixedBitSet,

    /// Per-round route queue. q_entry[route.idx()] = Some(boarding stop) when route
    /// is in queue; q_routes is the dense list of routes that have entries (so we
    /// iterate without scanning n_routes nulls).
    q_entry: Vec<Option<StopIdx>>,
    q_routes: Vec<RouteIdx>,

    /// Scratch buffer for footpath relaxation output.
    walked_buf: Vec<StopIdx>,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Boarded { from: StopIdx, route: RouteIdx },
    Walked { from: StopIdx },
}

impl RaptorCache {
    pub fn for_timetable(tt: &impl Timetable) -> Self {
        Self::with_capacity(tt.n_stops() as u32, tt.n_routes() as u32)
    }
    pub fn with_capacity(n_stops: u32, n_routes: u32) -> Self { /* ... */ }

    fn reset_for_query(&mut self, transfers: K, tt_n_stops: u32, tt_n_routes: u32) {
        assert_eq!(self.n_stops, tt_n_stops, "RaptorCache sized for different timetable");
        assert_eq!(self.n_routes, tt_n_routes, "RaptorCache sized for different timetable");
        // resize labels to (transfers + 1) Vecs of length n_stops, fill with Tau::MAX
        // clear best_arrival to Tau::MAX
        // clear marked_stops bitset
        // for r in q_routes.drain(..) { q_entry[r.idx()] = None; }
        // board_detail.clear(); walked_buf.clear();
    }
}
```

Hot-loop transformations:

- `labels[k].get(&stop).copied().unwrap_or(Tau::MAX)` → `labels[k][stop.idx()]`
- `best_arrival.get(&pt).copied().unwrap_or(Tau::MAX)` → `best_arrival[pt.idx()]`
- `marked_stops.insert(p)` → `marked_stops.insert(p.idx())` on the bitset
- `q.entry(route).or_insert(marked_stop)` and `*p_dash = self.get_earlier_stop(...)` → check `q_entry[r.idx()]`; if `None`, push to `q_routes` and store; if `Some(prev)`, fold via `get_earlier_stop`.

Sparse-set reset: walk `q_routes`, clear corresponding `q_entry` slots, clear `q_routes`. Per-round cost is `O(distinct routes touched)`, not `O(n_routes)`.

The `assert_eq!` on cache sizing is always-on. One branch per query, fails loudly on misuse, no lifetime contortions.

## `GtfsTimetable` changes

```rust
pub struct GtfsTimetable<'gtfs> {
    gtfs: &'gtfs Gtfs,

    // Forward tables: idx -> &'gtfs str (original GTFS IDs).
    stop_ids: Vec<&'gtfs str>,
    route_ids: Vec<&'gtfs str>,    // synthetic-route -> original GTFS route_id
    trip_ids: Vec<&'gtfs str>,

    // Reverse tables: &str -> idx (small footprint; kept for inverse lookup).
    stop_by_id: HashMap<&'gtfs str, StopIdx>,
    route_by_id: HashMap<&'gtfs str, RouteIdx>,    // first synthetic for that GTFS route_id
    routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>>,    // all synthetics
    trip_by_id: HashMap<&'gtfs str, TripIdx>,

    // Existing per-route tables, keyed by RouteIdx and storing index types.
    routes_for_stops: Vec<SmallVec<[RouteIdx; 8]>>,    // indexed by StopIdx
    stops_for_route: Vec<Vec<StopIdx>>,                 // indexed by RouteIdx
    trips_for_route: Vec<Vec<TripIdx>>,                 // indexed by RouteIdx

    // Per-route, per-stop-position, per-trip-position arrival/departure times.
    // departures[route][stop_pos][trip_pos] -> Tau. Replaces find_stop_time linear scans.
    arrival_times: Vec<Vec<Vec<Tau>>>,
    departure_times: Vec<Vec<Vec<Tau>>>,

    footpaths_for_stops: Vec<SmallVec<[StopIdx; 4]>>,    // indexed by StopIdx
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,
}
```

Construction (`GtfsTimetable::new`):

1. Walk `gtfs.stops` in iteration order, assign each a `StopIdx`, populate `stop_ids` and `stop_by_id`.
2. Run the existing route-pattern-splitting + non-overtaking logic (currently in `build_route_index`). After splitting, assign each synthetic route a `RouteIdx`, populate `route_ids` (idx → original GTFS `route_id`), `route_by_id` (first synthetic for each GTFS route), and `routes_by_gtfs_id` (all synthetics).
3. Walk every trip in synthetic-route order, assign `TripIdx`, populate `trip_ids` and `trip_by_id`.
4. Materialise `arrival_times` and `departure_times` by reading each trip's `stop_times` once. This is the data structure 1.3 wants.

Public lookup API:

```rust
impl<'gtfs> GtfsTimetable<'gtfs> {
    pub fn stop_id(&self, stop: StopIdx) -> &'gtfs str { ... }
    pub fn route_id(&self, route: RouteIdx) -> &'gtfs str { ... }
    pub fn trip_id(&self, trip: TripIdx) -> &'gtfs str { ... }

    pub fn stop_idx(&self, id: &str) -> Option<StopIdx> { ... }
    pub fn route_idx(&self, id: &str) -> Option<RouteIdx> { ... }    // first synthetic
    pub fn routes_for_gtfs_id(&self, id: &str) -> &[RouteIdx] { ... } // all synthetics
    pub fn trip_idx(&self, id: &str) -> Option<TripIdx> { ... }
}
```

`gtfs::RouteId` is removed. The synthetic-route semantics ("equivalence class of trips with identical stop sequences and pairwise non-overtaking schedules") move to the `GtfsTimetable` module-level docs and the `route_id`/`route_idx`/`routes_for_gtfs_id` doc comments.

The current `GtfsError::MissingDepartureTime` variant carries forward. Construction-time validation logic is unchanged; only the storage layout changes.

## `SimpleTimetable` — interning builder

```rust
pub struct SimpleTimetable<S = u32, R = u32, T = u32>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    // Forward + reverse interning tables for tests/benches that want their own keys.
    stop_keys: Vec<S>,
    stop_by_key: HashMap<S, StopIdx>,
    route_keys: Vec<R>,
    route_by_key: HashMap<R, RouteIdx>,
    trip_keys: Vec<T>,
    trip_by_key: HashMap<T, TripIdx>,

    // Internal storage in dense indices.
    routes: Vec<Vec<StopIdx>>,                       // RouteIdx -> stops
    trips: Vec<(RouteIdx, Vec<(Tau, Tau)>)>,         // TripIdx -> (route, per-stop times)
    routes_for_stop: Vec<Vec<RouteIdx>>,             // StopIdx -> routes
    footpaths: Vec<Vec<StopIdx>>,                    // StopIdx -> reachable
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,
}

impl<S, R, T> SimpleTimetable<S, R, T> { /* generic over S, R, T */
    pub fn new() -> Self { ... }
    pub fn route(self, id: R, stops: &[S], trips: &[(T, &[(Tau, Tau)])]) -> Self { ... }
    pub fn footpath(self, from: S, to: S) -> Self { ... }
    pub fn transfer_time(self, from: S, to: S, time: Tau) -> Self { ... }

    // Lookups for tests asserting on Journey.plan.
    pub fn stop_idx_of(&self, key: &S) -> StopIdx { self.stop_by_key[key] }
    pub fn route_idx_of(&self, key: &R) -> RouteIdx { self.route_by_key[key] }
}
```

The `Default = u32` type params let `SimpleTimetable::new()` work without turbofish for callers who want indices directly.

A `plan!` test-helper macro keeps assertions readable:

```rust
macro_rules! plan {
    ($tt:expr; $(($route:expr, $stop:expr)),* $(,)?) => {
        vec![$(($tt.route_idx_of(&$route), $tt.stop_idx_of(&$stop))),*]
    };
}
// usage: assert_eq!(best.plan, plan!(tt; (R2, B), (R3, D)));
```

## Migration

### Unit tests (`raptor/src/test.rs`)

About 15 tests. Each `assert_eq!(j.plan, vec![...])` adopts the `plan!` macro. Cache-related test (`raptor_with_cache_matches_fresh_run`) updates its `RaptorCache` construction to `RaptorCache::for_timetable(&tt)`. No conceptual changes.

### Examples (`raptor/examples/`)

- `simple.rs`, `reboarding.rs`, `two_journeys.rs` — same enum-keyed style as tests; same `plan!` macro pattern; journey display uses `tt.stop_idx_of` / `tt.route_idx_of` to translate keys.
- `gtfs-timetable.rs` — CLI args (`&str` start/target) translate via `tt.stop_idx(start_str).expect(...)` before calling `raptor`. Display loop iterates `(RouteIdx, StopIdx)` pairs and calls `tt.stop_id(stop)` / `tt.route_id(route)` to materialise strings. The current `tt.route_name(route_id)` call becomes `tt.route_id(route)`.

### Benchmarks (`raptor/benches/`)

- `raptor.rs` — `SimpleTimetable<usize, usize, usize>` from `builders.rs` carries over (interning `usize` keys is essentially identity). No assertion changes.
- `gtfs.rs` — construction-time only; trivially carries over.

### Proptest harness (`raptor-proptest/`)

- `spec.rs::render` — same builder calls; produces `SimpleTimetable<u8, u8, u16>` as today.
- `reference.rs` — operates directly on `NetworkSpec` and uses raw `u8`/`u16` types throughout. Unaffected by the trait change.
- `lib.rs::raptor_front` — currently generic over `<R, S>` because `Journey` is generic. With `Journey` concrete, `raptor_front` loses its generic params and takes `&[Journey]` directly.
- `lib.rs::run_property` — passes `spec.query.ps` (a `u8`) directly to `timetable.raptor`. After the change, it translates via the interning builder: `let ps = timetable.stop_idx_of(&spec.query.ps); let pt = timetable.stop_idx_of(&spec.query.pt);` before calling `raptor`. Same for the reference-solver comparison's expected output, which still uses `u8` (the reference solver hasn't moved).
- Proptest assertions compare two `BTreeSet<(u16, u8)>` front projections (arrival, trip count); unaffected.

### Workspace dependency

`fixedbitset = "0.5"` added to `raptor`'s `Cargo.toml`. Mature, no transitive deps.

## Testing strategy

The proptest harness is the contract. Every commit in the implementation plan must keep `cargo nextest r -p raptor-proptest` green across all three generator layers (no-footpath, small-footpath, full-network). That is non-negotiable: this is a rewrite of the data plane, and the harness is the only thing standing between us and a regression in correctness.

Order of operations in the implementation plan:

1. **Type and trait shift (1.1).** Introduce `StopIdx`/`RouteIdx`/`TripIdx`, drop `Timetable`'s associated types, add `n_stops`/`n_routes`. The algorithm body keeps its current shape — `RaptorCache` still holds `BTreeMap<StopIdx, Tau>` labels and `BTreeSet<StopIdx>` for marked stops, just rekeyed onto the new types. Adapters (`GtfsTimetable`, `SimpleTimetable`) get rewritten to intern at construction and expose the lookup API. Tests, examples, benches, and the proptest harness all migrate. Proptest goes green here.
2. **Storage rewrite (1.2 + 1.4).** Replace `BTreeMap<StopIdx, Tau>` labels with `Vec<Vec<Tau>>`, `BTreeSet<StopIdx>` marked-stops with `FixedBitSet`, and the per-round `BTreeMap<RouteIdx, StopIdx>` queue with the sparse-set pair. `RaptorCache::for_timetable` allocates everything at the right size. Pure data-structure swap; algorithm logic is unchanged. Proptest stays green.
3. **GTFS access path (1.3).** Materialise `arrival_times[route][stop_pos][trip_pos]` and `departure_times[...]` in `GtfsTimetable::new`. Reimplement `get_arrival_time` / `get_departure_time` against those tables, eliminating the `find_stop_time` linear scan. The algorithm calls don't change; only the adapter's response time per call. Proptest stays green.
4. **Doc and CHANGELOG pass.** Update README's `Implementing Timetable` section, the GTFS example narrative, and the CHANGELOG entry for v0.4.

Each step is independently committable. If any step breaks the proptest harness, that commit is the blast radius — easy to revert and isolate.

## Out of scope

- 1.6 (parallel queries — docs + integration test) and 1.7 (real-feed benchmarks) are separate, smaller changes.
- The label-trait restructuring for McRAPTOR (Phase 2) is unaffected by this work and not bundled in.
- The opt-in transitive-closure pass for footpaths (Phase 0.7 medium-term enhancement) is independent.
