# Changelog

## Unreleased

Two range-query improvements: rRAPTOR for the serial path (single
reverse-chronological scan replacing the naïve batch), and a Rayon-based
parallel batch behind a new default-on `parallel` feature flag.

### New API

- `pub struct RaptorCachePool<L>` – a `Sync` freelist of `RaptorCache`s
  sized for one timetable. `for_timetable(&tt)` / `with_capacity(n_stops,
  n_routes)` constructors; `checkout()` returns a `PooledCache<'_, L>`
  RAII guard that derefs to `&mut RaptorCache<L>` and returns the cache
  on drop. Available unconditionally – works with std threads, async
  tasks, or Rayon.
- `Query<RangeDeparture>::run_par() -> Vec<RangeJourney<L>>` – fans the
  per-departure work across Rayon's global thread pool. Allocates an
  internal pool. Available with the `parallel` feature.
- `Query<RangeDeparture>::run_with_pool(&pool) -> Vec<RangeJourney<L>>`
  – same parallelism, caller-supplied pool. Available with the
  `parallel` feature.

### Performance

60-departure window on the bundled Delhi feed (Apple Silicon, 8 cores):
2-trip query 2.38 ms → 0.45 ms (5.3x); 3-trip query 5.37 ms → 0.83 ms
(6.5x). Speedup scales with the number of departures in the window.

### Cargo features

- `parallel` (new, default-on) – pulls in `rayon`, gates `.run_par()` /
  `.run_with_pool(...)`. Opt out with `default-features = false` for
  wasm or minimal builds; `RaptorCachePool` itself stays available.

### rRAPTOR for serial range queries

The serial range-query path (`Query::run` / `.run_with_cache` for
`L = ArrivalTime`) now runs the rRAPTOR algorithm from §4 of the
original paper – a single reverse-chronological scan that reuses
labels across departures, instead of N independent RAPTOR runs.

Output shape and semantics unchanged. Bench numbers (60-departure
window, Delhi feed): 2-trip 2.38 ms → 1.35 ms (~43% faster);
3-trip 5.37 ms → 1.84 ms (~66% faster). The trivial 1-trip case
is slightly slower (~547 µs → 1.12 ms) because the per-τ overhead
doesn't amortise on a single-route query; rRAPTOR wins where label
reuse pays off, which is most non-trivial queries.

### Breaking changes

`Query<L, RangeDeparture>` no longer exposes `.run()` /
`.run_with_cache()` for arbitrary `L: Label`; those are restricted to
`L = ArrivalTime`. Custom-Label range queries use `.run_par()` /
`.run_with_pool()` which keep the naïve-batch semantics for any
`L: Label + Send + Sync`.

## [0.14.0] – 2026-05-05

API ergonomics pass. The query surface collapses from six methods to one
typestate builder; primitive `usize` timestamps and walk-time offsets become
typed `SecondOfDay` / `Duration` / `Transfers` newtypes; the `simple` adapter
renames to `manual`. Mass breakage with no compatibility shim – pre-release,
the cleaner shape wins.

### New types

- `pub struct SecondOfDay(pub u32)` – was `pub type SecondOfDay = usize`. A timestamp,
  seconds since midnight. `ZERO`, `MAX`, `from_secs`, `hms`, `as_secs`,
  `as_hms`, `From<u32>`, `Display`, plus the arithmetic impls below.
- `pub struct Duration(pub u32)` – a length of time, distinct from `SecondOfDay`.
  Walk-time offsets, transfer times, dwell times all use `Duration`.
- `pub struct Transfers(pub u8)` – user-facing transfer cap (255 is plenty;
  RAPTOR's interest is in low-transfer journeys).
- `pub struct Endpoints { stops: SmallVec<[(StopIdx, Duration); 50]> }` and
  `pub trait IntoEndpoints` so single-stop, slice, and Vec inputs all
  unify at the call site.
- `pub struct Query<'tt, T, L = ArrivalTime, M = NeedsDeparture>` —
  typestate builder; `M` transitions through `NeedsDeparture` →
  `SingleDeparture` / `RangeDeparture`.

### Arithmetic

- `SecondOfDay + Duration → SecondOfDay` (saturating).
- `SecondOfDay - SecondOfDay → Duration` (saturating to `Duration::ZERO`).
- `SecondOfDay - Duration → SecondOfDay` (saturating to `SecondOfDay::ZERO`).
- `Duration + Duration → Duration` (saturating).

### Trait surface

The `Timetable` trait's query API is now:

- `tt.query() -> Query<...>` – entry point for the builder.
- `tt.query_with_label::<L>() -> Query<..., L, ...>` – same, custom label.

The six methods that were here in v0.13.x (`raptor`, `raptor_with_label`,
`raptor_with_cache`, `raptor_with_cache_and_label`, `raptor_range`,
`raptor_range_with_cache`) are gone. The two algorithm-entry methods
(`raptor_with_cache_and_label`, `raptor_range_with_cache`) remain on the
trait but are `#[doc(hidden)]` – `Query::run` / `Query::run_with_cache`
dispatch through them.

### Builder usage

```rust,ignore
let journeys = tt
    .query()
    .from(start)              // bare StopIdx, slice, or Vec
    .to(end)
    .max_transfers(10u8)      // u8, defaults to Transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();

// Range query – same shape, swap the departure method:
let profile = tt
    .query()
    .from(start).to(end)
    .max_transfers(10u8)
    .depart_in_window((17 * 3600..18 * 3600).step_by(60).map(SecondOfDay::from_secs))
    .run();

// Reuse a cache:
let mut cache = RaptorCache::for_timetable(&tt);
let journeys = tt.query().from(start).to(end)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run_with_cache(&mut cache);
```

`.run()` is only callable once a departure mode is set – the typestate
prevents `tt.query().from(s).to(t).run()` from compiling, since there's
no `.run()` method on `Query<..., NeedsDeparture>`.

### Module rename

`raptor::simple` → `raptor::manual`. The new name reflects what the
adapter is – a `Timetable` you build by hand with `.route(...)` /
`.footpath(...)` calls – rather than reading as "example code". External
imports change accordingly.

### Migration

Every existing call site of the removed `raptor*` methods needs to be
rewritten to the builder. The shape is mechanical:

```rust,ignore
// Before:
tt.raptor(K, T, &[(s, 0)], &[(t, 0)])
// After:
tt.query().from(s).to(t).max_transfers(K as u8).depart_at(T).run()

// Before:
tt.raptor_with_cache(&mut cache, K, T, &origins, &targets)
// After:
tt.query().from(&origins).to(&targets).max_transfers(K as u8)
    .depart_at(T).run_with_cache(&mut cache)
```

For multi-criterion (custom `Label`):

```rust,ignore
// Before:
tt.raptor_with_label::<MyLabel>(K, T, &origins, &targets)
// After:
tt.query_with_label::<MyLabel>().from(&origins).to(&targets)
    .max_transfers(K as u8).depart_at(T).run()
```

`SecondOfDay` literals must be wrapped – `0` becomes `SecondOfDay(0)`, `SecondOfDay::ZERO`,
or `SecondOfDay::from_secs(0)`. `Duration` walk-time offsets become
`Duration::ZERO` or `Duration(N)`. The cross-city benchmark example,
the test suite, the proptest harness, and the dotgraph adapter were
all migrated automatically by a paren-aware Python script with a
handful of hand-edits for the awkward shapes.

### Tests

- `into_endpoints_accepts_natural_input_shapes` exercises 6 input
  shapes for the same query.
- `query_builder_single_departure` covers the bare flow,
  `max_transfers(N)`, `run_with_cache(&mut cache)`, and the
  `query_with_label::<L>` path.
- `query_builder_range_departure` covers `.depart_in_window(...)`
  against a 3-trip route.
- 58/58 tests + proptests + doctests green.

## [0.13.1] – 2026-05-05

Naïve batch range query API. Lands the user-facing range-query
feature (Phase 3.1) ahead of the planned API ergonomics pass and the
real rRAPTOR algorithm rewrite. The output shape is the one the
proper rRAPTOR will produce, so callers writing against this API
today won't have to migrate.

### Added

- `Timetable::raptor_range(transfers, departures, origins, targets)
  -> Vec<RangeJourney>` – for each `SecondOfDay` in `departures` (any
  `IntoIterator`), runs the algorithm and Pareto-filters the
  combined results into a profile. Common pattern:
  `tt.raptor_range(10, (t_start..t_end).step_by(60), &origins,
  &targets)` for one query per minute over a window.
- `Timetable::raptor_range_with_cache::<L>(cache, …)` – generic
  variant taking a `RaptorCache<L>`. The label parameter is
  inferred from `cache`. For server workloads.
- `pub struct RangeJourney<L: Label = ArrivalTime> { depart, journey }`
  – one entry per Pareto-optimal `(depart, journey)` pair.

### Pareto contract

The returned profile is Pareto-optimal on the triple
`(later depart, fewer transfers, dominated label)` – concretely,
for any two returned entries, neither has all of `(depart ≥,
plan.len ≤, label.dominates)` holding. So a caller can read the
result as a true profile: each entry is either strictly better
than every other on at least one axis, or incomparable.

### Implementation note

This is a naïve batch – each departure is a fresh
`raptor_with_cache` call with no cross-departure state sharing.
The proper rRAPTOR algorithm processes events in reverse
chronological order and reuses labels, dropping the per-call
overhead. Roadmap §3.1 has the details; that lands after the
API ergonomics pass so the new public surface uses the new
shape.

### Tests

- `raptor_range_returns_pareto_profile_across_departures`:
  three-trip route with departures `[0, 5, 10, 15, 20]`
  produces profile `[(0, arr 10), (10, arr 20), (20, arr 30)]`
  – the intermediate `(5, arr 20)` and `(15, arr 30)` entries
  are correctly dropped as Pareto-dominated by their
  later-departure counterparts. 55/55 tests + proptests +
  doctests green.

## [0.13.0] – 2026-05-05

Canned multi-criterion `Label` impl + label-aware journey output.
Closes Phase 2.4 step 3 in minimum-viable form: ships one vetted
multi-criterion label so the v0.11 bag-of-labels machinery actually
delivers Pareto fronts to users, plus the output filter fix that
makes that visible.

### Added

- `pub mod raptor::labels` – canned `Label` impls.
- `pub struct labels::ArrivalAndWalk { arrival, walk_time }` – two-
  criterion label that tracks accumulated walking time alongside
  arrival time. Trip rides preserve the boarding label's walking
  time; footpaths add to both arrival and walking time. Pareto
  dominance is component-wise.

### Changed

- The journey output Pareto filter is now label-aware. Previously
  it dropped any journey whose `arrival()` was not strictly less
  than the best seen so far for an equal-or-greater trip count;
  this collapsed multi-criterion fronts to single-criterion. The
  new filter keeps any journey not weakly dominated on
  `(plan.len, label)` by another. For single-criterion
  `ArrivalTime` this collapses to the v0.10 contract – same
  behaviour, same journeys; for multi-criterion impls like
  `ArrivalAndWalk` it returns the actual Pareto front.

### Caveats

- The algorithm's per-stop pruning during the route scan is still
  arrival-only (`if arr >= time_to_beat { continue; }`). For most
  multi-criterion queries this is fine – Pareto-incomparable labels
  reach the bag via independent paths (different routes, footpath
  chains) and the bag's `dominates` check preserves them. But a
  candidate label with worse arrival than the current best at the
  same stop, even if its other components are Pareto-better, is
  dropped before the bag sees it. Fully multi-criterion-correct
  pruning is queued for v0.14+ – see `Label` trait docs.

### Tests

- `arrival_and_walk_label_tracks_accumulated_walk_time` (renamed
  from `custom_label_tracks_accumulated_walk_time`) now uses the
  public `labels::ArrivalAndWalk`.
- `arrival_and_walk_returns_pareto_front` constructs a network
  where two paths to the target have Pareto-incomparable
  (arrival, walk_time) pairs (15s/5walk vs 21s/1walk); verifies
  `ArrivalTime` returns 1 journey and `ArrivalAndWalk` returns 2.
  53/53 tests + proptests + doctests green.

## [0.12.0] – 2026-05-05

`Journey::with_timing` – recover per-leg trip and timing info from a
returned journey. Previously `Journey.plan` was just topology
(`Vec<(RouteIdx, StopIdx)>`); users wanting "board route X at A at
08:03, arrive B at 08:17" had to reconstruct it themselves.

### Added

- `Journey::with_timing(&tt, tau, origin_walk) -> Option<Vec<TimedLeg>>`
  walks the plan against the timetable and returns one `TimedLeg`
  per transit boarding. Each leg reports `route`, `board`, `alight`,
  the specific `trip: TripIdx` caught, and `depart` / `arrive`
  times. Returns `None` if the plan can't be matched (defensive
  escape hatch – should not happen for journeys produced by the
  same timetable).
- `pub struct TimedLeg` with the fields above. Plain `Copy` data,
  no algorithm state.

### Notes

- Walking transfers between consecutive transit legs are implicit:
  if leg `N`'s `alight` differs from leg `N+1`'s `board`, the gap
  reflects walking time. The user can derive walking-leg detail by
  consulting `tt.get_transfer_time(alight_n, board_n_plus_1)`.
- Loop routes (a route's stop sequence revisits a boarding stop)
  pick the *earliest* qualifying position, matching the algorithm's
  own `get_routes_serving_stop` return contract. For loop-heavy
  networks this could in principle pick the wrong occurrence; in
  practice the reconstructed trip matches the algorithm's choice.

### Tests

- `with_timing_recovers_per_leg_trip_and_times` exercises a
  two-leg journey and verifies the right trip is picked per leg
  along with correct departure/arrival times. 52/52 tests +
  proptests green.

## [0.11.0] – 2026-05-05

Bag-of-labels representation. Each `(round, stop)` cell now holds a
Pareto front of `Label`s rather than a single label, enabling
multi-criterion impls (e.g. `ArrivalAndWalk` from v0.10) to actually
produce Pareto-optimal journey sets rather than a tiebroken single
label. Single-criterion `ArrivalTime` bags stay at size 1 – same
journeys, real but bounded perf hit.

### Algorithm

- `LabelBag<L>` (private) backs every `(round, stop)` cell – a
  `SmallVec<[L; 8]>` with Pareto insert/dominate. Insert returns
  `false` when the new label is weakly dominated by an existing one
  and removes any items the new label strictly dominates.
- Route scan now maintains a per-route `route_bag` of riding entries
  `(boarding_label, trip, boarding_stop)`. At each stop on the route
  every active entry alights (one new label per `(label, trip)`
  pair); labels in `labels[k-1][pi]` that catch a trip and are not
  dominated by an existing entry join the bag.
- Footpath relax (both Dijkstra and single-pass) extends every label
  in the source bag along each footpath edge.
- Boarding tree key is now `(K, StopIdx, SecondOfDay)` – the third component
  is the label's effective arrival, disambiguating Pareto-optimal
  labels with distinct arrival times in the same bag. `Step` variants
  carry a `parent_arrival: SecondOfDay` field so reconstruction can pick the
  right parent label out of the parent bag.
- Reconstruction enumerates per-label per-round at every target,
  deduplicating by `(target, raw_arrival, k, walk)`. Multi-criterion
  queries can produce more journeys per target than v0.10.

### Performance trade-off

- Single-criterion ArrivalTime queries regress 2-3× vs v0.9-v0.10
  on the cross-city bench. The bench's worst regression is Berlin
  Hbf → Alex hand-picked: 107 µs → 672 µs (6×); Paris Châtelet →
  Versailles is the mildest: 16.7 ms → 32.3 ms (1.9×).
- Carry-forward overhead was the worst contributor (cloning all 50k
  bag entries per round, even empties). A `ever_reached: FixedBitSet`
  added to `RaptorCache` gates the carry-forward to stops actually
  touched this query, recovering ~30% of the regression.
- The remaining gap is from bag operations themselves – for ArrivalTime
  size-1 bags the route scan does 2-3 trait method calls per stop
  visit (insert, dominates, snapshot) where v0.10 used direct `SecondOfDay`
  comparisons. A future v0.12 specialisation can reclaim more for
  pure single-criterion users; for now correctness on multi-criterion
  is the priority and the bench numbers stay well under interactive
  thresholds.

### Reconstruction note

- Helsinki Rautatientori → Pasila now returns 4 Pareto-optimal
  journeys (was 3 in v0.10) at the same min-arrival 09:07:00. The
  extra journeys are alternative plans the v0.10 single-label
  reconstruction collapsed into one – an honest improvement, not a
  regression.

### Tests

- All 51 tests + proptests green; the proptest harness exercises
  multi-criterion correctness implicitly via the `custom_label_*`
  test from v0.10 and the existing layer 1-3 coverage.

## [0.10.0] – 2026-05-05

McRAPTOR-ready `Label` trait. The algorithm is now generic over a
caller-supplied label type – single-criterion users see no behaviour
change (the default `ArrivalTime` impl inlines to v0.9 code), but
custom labels can now ride along (e.g. accumulated walking time) and
be reported on the resulting `Journey`. The bag-of-labels
representation needed for true multi-criterion (Pareto fronts per
stop) is planned for v0.11.

### Added

- `pub trait Label: Copy + Debug` with associated `UNREACHED` const
  and methods `from_departure`, `extend_by_trip`,
  `extend_by_footpath`, `dominates`, `arrival`. The default
  `dominates` impl uses `arrival` (correct for single-criterion).
- `pub struct ArrivalTime(pub SecondOfDay)` – the only `Label` impl shipped.
  Default for both `Journey<L>` and `RaptorCache<L>`.
- `Timetable::raptor_with_label::<L>` and
  `Timetable::raptor_with_cache_and_label::<L>` – generic variants
  that take/return a custom label. The non-generic `raptor` and
  `raptor_with_cache` are unchanged and call into the generic
  versions with `L = ArrivalTime`.

### Breaking changes

- `Journey` lost its `pub arrival: SecondOfDay` field; replaced by `pub label:
  L` plus a convenience `Journey::arrival(&self) -> SecondOfDay` method.
  Callers reading `j.arrival` need to use `j.arrival()` instead. For
  the default `ArrivalTime` label, `j.label.0` is also the bare `SecondOfDay`.
- `Journey` and `RaptorCache` are now generic over `L: Label` with
  default `ArrivalTime`. Type inference covers most call sites
  unchanged; explicit annotations like `let j: Vec<Journey> = …`
  continue to mean `Vec<Journey<ArrivalTime>>`.

### Performance

- Generic refactor adds no measurable overhead – `ArrivalTime`'s
  trait methods inline to direct `SecondOfDay` operations. Cross-city bench
  numbers are unchanged from v0.9 within measurement noise.

### Tests

- `custom_label_tracks_accumulated_walk_time` exercises the generic
  path with a custom `ArrivalAndWalk` label, verifies the extra
  walk-time component is correctly threaded through trip rides
  (no-op) and footpath relaxations (accumulates).
- All 51 tests + proptests green.

## [0.9.0] – 2026-05-05

Closed-graph fast path for footpath relaxation. Reclaims most of the v0.8
performance regression on closed-transfer-graph feeds (Berlin, Paris)
without giving up the v0.8 correctness work for non-closed graphs.

### Added

- New trait method `Timetable::footpaths_are_transitively_closed(&self)
  -> bool`, defaulting to `false`. When `true`, the algorithm uses a
  single-pass `O(E)` per-round footpath relaxation; when `false`, it
  uses the v0.8 multi-source Dijkstra (`O(E log V)`).
- `GtfsTimetable::assert_footpaths_closed(self) -> Self` – opt-in
  builder for users who know their `transfers.txt` is the entire
  intended footpath relation (no chaining required, which is the
  publisher convention for most curated feeds). Resets to `false`
  whenever `with_walking_footpaths` is subsequently called.

### Algorithm

- New private helper `relax_footpaths_round_closed` mirrors the v0.7
  single-pass logic. Algorithm reads
  `footpaths_are_transitively_closed()` once per query and dispatches
  to the right relax function for every round.

### Cross-city benchmark

The bench now calls `assert_footpaths_closed()` for every feed without
walking footpaths, recovering most of the v0.8 regression:

- Paris Châtelet → Gare du Nord: 8.7 ms → 0.77 ms (11× faster)
- Paris Châtelet → La Défense: 14.0 ms → 0.88 ms (16× faster)
- Paris Châtelet → Versailles: 34.5 ms → 16.7 ms (2× faster)
- Berlin queries unchanged within noise (the regression was small
  there to begin with).

Helsinki stays on the Dijkstra path because `with_walking_footpaths`
clears the closure flag – coordinate-derived edges within a radius are
not transitively closed (the algorithm reaches farther-away stops by
chaining short walks). Helsinki latencies and answers are unchanged
from v0.8.

### Soundness note

Asserting closure on a non-closed relation is a soundness violation —
the algorithm will miss journeys whose optimal path requires chaining
direct walks within a round. Only call `assert_footpaths_closed()`
when you know the relation is closed (or when you intend the
publisher's `transfers.txt` to be treated as the entire intended
relation, which matches v0.7 semantics).

## [0.8.0] – 2026-05-05

Transfer-graph density. The `Timetable` trait no longer requires the
footpath relation to be transitively closed: if `A → B` and `B → C` are
both reported as direct walks, the algorithm chains them within a single
round and reaches `C` from `A` with the combined walk time. Closure is
still useful as an optimisation (fewer edges to traverse) but is no
longer a soundness prerequisite.

### Trait change

- `get_footpaths_from` now returns *direct* walks only – the relation
  does not need to be transitively closed. Existing custom adapters that
  pre-closed the relation continue to work without modification (closure
  is a valid superset of the direct-walks relation). The trait-level docs
  have been rewritten accordingly.

### Algorithm

- `relax_footpaths_round` now runs a multi-source Dijkstra over the
  footpath graph at each round, propagating walk improvements to a fixed
  point. Standard lazy-deletion min-heap; `O(E log V)` per round.
  Replaces an earlier single-pass relaxation that required the input to
  be closed; a Bellman-Ford-style intermediate form turned out to
  degenerate to `O(V·E)` on dense walking graphs.
- `RaptorCache` gains a `relax_heap` field (`BinaryHeap<Reverse<(SecondOfDay,
  u32)>>`) reused across rounds. Construction API is unchanged
  (`RaptorCache::for_timetable` / `with_capacity`).

### Added

- `GtfsTimetable::with_walking_footpaths(&gtfs, max_distance_m,
  walking_speed_m_per_s) -> Self`: augments the footpath graph with
  bidirectional walking edges between every pair of stops within
  `max_distance_m` straight-line distance. Builds an R-tree over an
  equirectangular projection anchored at the feed's mean latitude
  (accurate to ~0.5% at city scale) and queries it with
  `locate_within_distance`. Existing `transfers.txt` entries are
  preserved – coordinate-derived edges are only added where no explicit
  transfer between the pair already exists.
- New dependency: `rstar = "0.12"` (spatial index for the new builder).

### Cross-city benchmark

- The Helsinki Rautatientori → Pasila query now returns a 7-minute
  journey via walking footpaths. Previously this returned no journey at
  all because the HSL feed has no `transfers.txt` and the two stations
  use different parent IDs – the only path was a metro/bus combination
  that needed a walking interchange the algorithm couldn't see. The
  bench now passes `walking_footpaths_m: Some(500.0)` for Helsinki to
  exercise the new builder.
- Paris IDFM query latencies regressed by roughly an order of magnitude
  on this run (Châtelet → Gare du Nord 0.4 ms → 8.7 ms; Châtelet →
  Versailles 9.8 ms → 34.5 ms). The cause is the Dijkstra heap overhead:
  on graphs whose `transfers.txt` is already close to transitively
  closed, the old single-pass relaxation visited each footpath edge
  exactly once with no heap ops; Dijkstra still visits each edge once
  but pays an `O(log V)` heap operation per visit. Berlin (closed
  transfers.txt, similar density to Paris) shows only a ~20% regression
  on the hand-picked-platforms query (88 µs → 106 µs) and still finishes
  the station-to-station query in 385 µs – the regression scale depends
  on how many footpath edges the round actually touches.

- A detect-closed-graph optimisation (skip the heap when the graph is
  known to be transitively closed) is a reasonable future addition; for
  now correctness on non-closed graphs is the priority.

## [0.7.0] – 2026-05-05

Multi-source / multi-target queries. The `Timetable::raptor` and
`raptor_with_cache` signatures now take `&[(StopIdx, SecondOfDay)]` slices for
both origins and targets – each tuple is a `(stop, walk_time_offset)`
pair. The user supplies the candidate stops near their actual origin
(and destination) along with how long it takes to walk to each;
the algorithm minimises effective arrival = arrival_at_target_stop +
walk_time, picking the best combination internally.

Motivating use case: GTFS feeds use a parent-station / child-platform
model where trips serve specific platforms (`location_type=0`) and
the named station is a separate parent stop (`location_type=1`) with
no trips. Querying with the parent's stop ID returns no journey;
users were forced to know which child platform served their direction
of travel. The new shape lets callers pass *all* of a station's child
platforms as origins/targets and have the algorithm pick correctly.

### Breaking changes

- `Timetable::raptor` and `raptor_with_cache` change from
  `(transfers, tau, ps: StopIdx, pt: StopIdx)` to
  `(transfers, tau, origins: &[(StopIdx, SecondOfDay)], targets: &[(StopIdx, SecondOfDay)])`.
  Single-stop queries become `&[(stop, 0)]`.
- `Journey` gains two fields: `origin: StopIdx` (which of the supplied
  origins this journey actually started from) and `target: StopIdx`
  (which target stop was reached). `arrival` now includes the
  user-supplied walk-time offset for the chosen target.

### Added

- `GtfsTimetable::station_stops(parent_id) -> &[(StopIdx, SecondOfDay)]`:
  returns the child platforms of a parent station, ready to pass
  directly to `raptor` as origins or targets. Each entry has walk
  time 0 by default; callers wanting platform-specific walk times
  can clone and adjust.

### Test infrastructure

- Three new unit tests cover multi-source / multi-target semantics:
  picking the best origin from a set, walk-time offsets changing the
  preferred origin, and walk-time offsets changing the preferred
  target.
- `raptor-proptest` continues to pass – the existing harness wraps
  single-stop queries as `&[(stop, 0)]` and the algorithm degrades to
  the old single-stop behaviour cleanly.

### Cross-city benchmark

The bench's Berlin query now appears in two forms: hand-picked S-Bahn
platforms (the v0.6 form, 20m 36s including 15-minute wait at a
specific eastbound platform) and station-to-station (Hbf parent to
Alex parent, 7m 6s – the algorithm picks the best of 301 × 50 platform
combinations and finds the actually-fastest direct S-Bahn). The
station-to-station form is ~3-4× slower in latency (~370 µs vs ~110
µs) because of the extra origins/targets to consider, but still
well under a millisecond on the 42k-stop Berlin feed.

## [0.6.0] – 2026-05-04

Phase 0.10 (soundness). The GTFS adapter now filters trips by
`calendar.txt` / `calendar_dates.txt` at construction. Without this,
the algorithm previously considered every trip in the feed regardless
of which day it actually ran, producing technically-not-wrong but
practically-bizarre answers on multi-day feeds.

### Breaking changes

- `GtfsTimetable::new(&gtfs)` is now
  `GtfsTimetable::new(&gtfs, service_date: jiff::civil::Date)`. Trips
  whose `service_id` is not active on `service_date` are filtered out;
  the constructed timetable contains only the day's trips.
- `GtfsTimetable` now exposes `n_trips()` returning the count of trips
  active on the timetable's service date.

### Added

- `jiff` dependency (used for the public service-date type).
- `chrono` dependency (already present transitively via
  `gtfs-structures`; now declared directly because the calendar
  resolution code needs `chrono::NaiveDate` at the
  `gtfs-structures` boundary).
- `gtfs::is_service_active` (private) – six new unit tests cover the
  calendar/calendar_dates resolution rules.

### Performance

- Calendar filtering reduces the synthetic-route count by 38–68% on
  the cross-city benchmark feeds (Helsinki HSL: 2,851 → 912; Berlin
  VBB: 18,194 → 10,757; Paris IDFM: 13,848 → 8,622). Query latency
  drops correspondingly:
  - Paris Châtelet → Versailles: 35 ms → 9.75 ms
  - Helsinki Rautatientori → Pasila search: 14.6 ms → 2.4 ms

## [0.5.0] – 2026-05-04

Phase 0.11 (soundness). Fixes a bug surfaced by cross-city
benchmarking against real-world GTFS feeds: trips that revisit a stop
within their `stop_sequence` (bus loops, terminus turnarounds – common
in Berlin, Paris, and most US city feeds) caused silently-wrong
journey output, including journeys with arrival times *before* the
query departure time. The fix is a position-aware redesign of the
`Timetable` trait's accessors, eliminating the `Vec::position()`
ambiguity that was the root cause.

### Breaking changes

- `Timetable` trait accessors now take a position-within-route (`u32`)
  instead of a `StopIdx` where the algorithm needs to disambiguate
  which visit of a stop on a route it means:
  - `get_routes_serving_stop` returns `&[(RouteIdx, u32)]` (each
    serving route paired with the *earliest* position of the stop on
    that route).
  - `get_stops_after(route, pos: u32) -> &[StopIdx]` (was
    `(route, stop)`).
  - `get_arrival_time(trip, pos: u32) -> SecondOfDay` (was `(trip, stop)`).
  - `get_departure_time(trip, pos: u32) -> SecondOfDay` (was `(trip, stop)`).
  - `get_earliest_trip(route, at, pos: u32) -> Option<TripIdx>` (was
    `(route, at, stop)`).
- New trait method: `stop_at(route, pos: u32) -> StopIdx` – looks up
  the stop at the given position within a route's sequence.
- Removed: `get_earlier_stop`. The algorithm now folds boarding
  positions via `min(prev_pos, new_pos)` directly.

Custom `Timetable` implementors must update their impls. The shape of
the change is mechanical: replace `position(|&s| s == stop)` lookups
with the supplied `pos` argument, dedup `routes_for_stop`-style
reverse maps so each route appears once with its earliest position.

### Performance

- Eliminating the per-call `Vec::position()` lookup in
  `GtfsTimetable::get_arrival_time` / `get_departure_time` makes them
  genuinely `O(1)`. Query latency improves 45–60% across all bundled
  benchmark feeds:
  - Delhi direct 1-trip: 3.5 µs → 1.9 µs
  - Delhi 2-trip transfer: 43 µs → 17 µs
  - Delhi 3-trip cross-line: 91 µs → 35 µs
  - Helsinki direct metro: 30 µs → 26 µs
  - Berlin algorithm cost: 33 ms → 17 ms
  - Paris Châtelet→Versailles: 163 ms → 38 ms

### Test infrastructure

- `raptor-proptest`: layer 3 of the proptest harness now generates
  loop trips (`LayerBounds.allow_loops` toggle; `unique(true)` is
  dropped on stop_sequence when set). The brute-force reference
  solver already handled loops; the algorithm now passes against it
  on layer 3.

### Removed

- `examples/diagnose-paris.rs` and `examples/check-symmetry.rs`. These
  were one-shot diagnostics used to identify the loop-route bug; the
  proptest harness now covers the class.

## [0.4.0] – 2026-05-04

This release closes the data-representation half of Phase 1: the
algorithm's hot loop is now branch-friendly array indexing, the GTFS
adapter pre-computes per-route departure/arrival tables, and the
`Timetable` trait is non-generic.

### Breaking changes

- The `Timetable` trait no longer has associated `Stop`/`Route`/`Trip`
  types. All identifiers are now newtypes around `u32`: `StopIdx`,
  `RouteIdx`, `TripIdx`. Implementors must add `n_stops()` and
  `n_routes()` methods; slice-returning accessors return `&[T]` instead
  of `Cow<[T]>`.
- `Journey` is now non-generic:
  `Journey { plan: Vec<(RouteIdx, StopIdx)>, arrival: SecondOfDay }`.
- `RaptorCache` is non-generic and constructed via
  `RaptorCache::for_timetable` (or `RaptorCache::with_capacity` for the
  count-only path). Reusing a cache against a differently-sized timetable
  now panics on entry to `raptor_with_cache`.
- `GtfsTimetable`'s associated types are gone; look up string IDs via
  `GtfsTimetable::stop_id` / `GtfsTimetable::route_id` /
  `GtfsTimetable::trip_id` and inverse-lookup via
  `GtfsTimetable::stop_idx` / `GtfsTimetable::route_idx` /
  `GtfsTimetable::trip_idx`. The previously-public `gtfs: &'gtfs Gtfs`
  field has been removed (no accessor reads it after the rewrite); the
  `'gtfs` lifetime remains in use for interned `&str`s and the
  `GtfsTimetable::new(gtfs: &'gtfs Gtfs)` signature is unchanged.
- `gtfs::RouteId` is removed; the synthetic-route concept now lives on
  `RouteIdx` with the same semantics.
- `SimpleTimetable<S, R, T>` has new generic bounds (`Hash + Eq + Clone`)
  and now interns its keys to dense `u32` indices. Construction API is
  unchanged; tests asserting on plans use the new
  `SimpleTimetable::stop_idx_of` / `SimpleTimetable::route_idx_of`
  helpers (and the in-test `plan!` macro). The internal `trips` field
  is now `Vec<Option<...>>` rather than a placeholder-padded `Vec<...>`,
  so trip slots that have not yet been populated are explicit.

### Performance

- Round labels are now `Vec<Vec<SecondOfDay>>` indexed by `(round, stop_idx)`
  rather than `Vec<BTreeMap<Stop, SecondOfDay>>` – all label reads/writes in the
  hot loop are array indexing.
- Marked stops are a `fixedbitset::FixedBitSet` sized to `n_stops`;
  insertion is a single bit write, iteration walks set bits.
- The per-round route queue is a sparse-set pair (`Vec<Option<StopIdx>>`
  plus a dense `Vec<RouteIdx>`) instead of `BTreeMap<RouteIdx, StopIdx>`,
  giving `O(distinct routes touched)` reset cost per round.
- `GtfsTimetable` now holds `arrival_times[route][stop_pos][trip_pos]`
  and `departure_times[…]` arrays computed once at construction. The
  table read itself is `O(1)`; a `route_for_trip[trip]` reverse map
  resolves the route in `O(1)`. The `stop_pos` lookup remains an
  `O(stops_in_route)` linear scan, so `get_arrival_time` /
  `get_departure_time` are now `O(stops_in_route)` once per call rather
  than `O(stops_in_route)` per stop scanned inside a trip lookup. A
  `(route, stop) → stop_pos` reverse map would close this gap; deferred
  to a follow-up.

### Added

- New dependency: `fixedbitset = "0.5"` (used for the marked-stops
  bitset).
- `GtfsTimetable::routes_for_gtfs_id` to enumerate every synthetic
  `RouteIdx` derived from a given GTFS `route_id`.
- `SimpleTimetable::register_stop` to intern a stop into the timetable
  builder without otherwise modifying it (useful when you need a stop
  reachable only via footpaths, with no boarding events of its own).

## [0.3.0] – 2026-05-03

This release closes Phase 0 of the production roadmap: the algorithm now
produces correct results, and the GTFS adapter no longer silently
returns wrong answers on real-world feeds.

### Breaking changes

- `gtfs::GtfsTimetable`'s associated `Route` type is now
  `gtfs::RouteId` (a `u32` newtype) rather than `&str`. A single GTFS
  `route_id` is split at construction into one or more synthetic
  `RouteId`s – one per equivalence class of trips with identical,
  non-overtaking stop sequences. Recover the original `route_id` for
  display via `GtfsTimetable::route_name(RouteId)`.

  Migration: where you previously matched on a `&str` route in a
  `Journey::plan` entry, call `timetable.route_name(*route_id)` first
  to get the GTFS `route_id`, then look up further metadata as before.
  See the updated `examples/gtfs-timetable.rs`.

### Algorithm correctness fixes

The full Phase 0 list. Each was a soundness gap on `v0.2.0`; details
are in `soundness.md` (issues A–I, all moved to Resolved Issues).

- **Carry-forward round labels** (issue A): labels are now stored as
  `Vec<BTreeMap<Stop, SecondOfDay>>` indexed by round, with `labels[k] =
  labels[k-1].clone()` at the top of each round. Stops reached in
  earlier rounds remain usable as boarding points and footpath
  origins.
- **Footpath relaxation from the source in round 0** (issue B): the
  footpath stage now runs once at init so journeys that begin with a
  walk are discoverable in round 1.
- **τ\* updated in the footpath stage** (issue C): walk-derived label
  improvements are mirrored into `best_arrival` so local and target
  pruning see them.
- **Target pruning in the footpath stage** (issue D): walk-reached
  stops are only marked when their arrival improves on
  `best_arrival[pt]`.
- **GTFS route-pattern splitting** (issue E): synthetic `RouteId`s as
  described above; `get_earliest_trip` is now sound on real GTFS
  feeds.
- **Pareto-filtered output** (issue F): `Timetable::raptor` sorts the
  result by trip count ascending and drops journeys whose arrival is
  not strictly better than the best seen so far. Output is
  deterministic.
- **Walk-leg journey reconstruction** (issue I): the boarding tree
  records both `Boarded` and `Walked` steps; reconstruction chains
  through walks within a round so walk-then-board, board-walk-board,
  and board-then-walk-to-pt journeys all reach the user.
- **Saturating arithmetic** (issue G): `relax_footpaths_round` uses
  `saturating_add`; the rest of the algorithm performs no `SecondOfDay`
  arithmetic.
- **Documented invariants** (issue H): the `Timetable` trait spells
  out the footpath-transitivity and no-overtaking contracts.

### Added

- `RaptorCache<Route, Stop>` and `Timetable::raptor_with_cache` for
  reusing scratch buffers across queries – recommended for server use
  cases running many queries against the same timetable.
- Hegel-based property test harness in the new `raptor-proptest`
  workspace crate that checks the algorithm against a brute-force
  multi-criterion Dijkstra reference solver. Three generator layers
  (no-footpath, small-footpath, full-network) all green on this
  release.
- `gtfs::GtfsError::MissingDepartureTime` variant: `GtfsTimetable::new`
  now refuses to construct from a feed whose `stop_times.txt` is
  missing departure times the algorithm relies on for ordering.

### Documentation

- Top-level `Timetable` trait now documents the footpath-transitivity
  and no-overtaking contracts that implementors must uphold.
- `Timetable::raptor`'s doc spells out the Pareto-optimality contract
  (sorted by trip count ascending; arrival strictly decreasing).
- README has a new "Implementing `Timetable`" section, an updated
  GTFS example reflecting the `RouteId` change, and a "Performance:
  reusing a `RaptorCache`" section.

## [0.2.0] – 2026-03-01

### Added
- Benchmarks with different networks and a `dotgraph` visualization tool (#17)
- Test infrastructure with `TestTimetable` and property-based test cases (#9)
- CI/CD workflow for PR checks (#10)

### Changed
- Switched to `Cow`-based API for zero-copy support (#13)
- Improved GTFS implementation: named constants, upfront cache validation, default transfer time (#11)
- Removed code duplication in the routing algorithm (#12)
- Rewrote documentation and README (#14)
- Converted to workspace layout; made `TestTimetable` public (#17)

## [0.1.1]

Initial release.
