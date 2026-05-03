# raptor-rs: Roadmap to Production

A working document for turning this codebase into a
production-grade RAPTOR implementation. Organised by priority: correctness
first (because wrong answers fast is worse than right answers slow), then
performance, then API/feature work.

References:

- Delling, D., Pajor, T., & Werneck, R. F. (2015). *Round-Based Public Transit
  Routing.* Transportation Science 49(3): 591–604. DOI:
  [10.1287/trsc.2014.0534](https://doi.org/10.1287/trsc.2014.0534).
- Delling, D., Pajor, T., & Werneck, R. F. (2012). *Round-Based Public Transit
  Routing.* ALENEX. (Earlier conference version of the above.)
- Wang, S., et al. (2015). *Public Transit Labeling.* SEA. (For HL-style
  preprocessing if we ever go there.)

Throughout this document I refer to the journal version's algorithm
numbering. The trait being modified is `raptor::Timetable` in
`raptor/src/lib.rs`.

---

## Phase 0 — Stop the bleeding (correctness)

These are the bugs that produce wrong answers on real feeds. Fix these
before doing anything else, and add a property-based test using hegel that compares
output against a brute-force solver before declaring victory.

### 0.1 Carry forward round labels (τₖ(p) ← τₖ₋₁(p))

**Status:** landed on v0.3 branch. Labels now live in
`Vec<BTreeMap<Stop, Tau>>` indexed by round; each round k starts with
`labels[k] = labels[k - 1].clone()`. Once stops are interned to `u32`
indices (Phase 1), this can become `Vec<Vec<Tau>>` for branch-free
carry-forward — that's a Phase 1 follow-up, not a soundness change.

### 0.2 Footpath relaxation from the source in round 1

**Status:** landed on v0.3 branch. The footpath stage is extracted into
`relax_footpaths_round` and called once at init for round 0, then once
per round inside the loop. Combined with 0.1's carry-forward, walk-
reachable neighbours of `ps` propagate into round 1 as boarding origins.

### 0.3 Update τ\* in the footpath stage

**Status:** landed on v0.3 branch. `relax_footpaths_round` now updates
both `labels[k]` and `best_arrival` whenever a walk strictly improves
an arrival.

### 0.4 Target pruning in the footpath stage

**Status:** landed on v0.3 branch. `relax_footpaths_round` only marks
`p_dash` when the walk-derived arrival strictly improves on
`best_arrival[pt]`.

### 0.5 Pareto-filter the output

**Status:** broken. `reconstruct_journey` emits one plan per
`k ∈ 1..=transfers` and `Timetable::raptor` collects all of them. A
2-transfer journey with the same arrival as a 1-transfer journey is not
dropped. Local/target pruning *should* prevent dominated plans from
being recorded in the first place; with 0.3 landed this is now more
reliable, but the explicit filter is cheap insurance.

**Fix:** sort journeys by transfer count ascending, then drop any
journey whose arrival is not strictly less than the best arrival seen
so far. Document that "Pareto-optimal" means "no other returned journey
has both ≤ transfers and ≤ arrival, with at least one strict".

### 0.5b Journey reconstruction must trace through walk legs

**Status:** landed on v0.3 branch. The boarding tree is now keyed by
`(K, Stop) -> Step<Route, Stop>` where `Step` is either `Boarded { from,
route }` or `Walked { from }`. `relax_footpaths_round` inserts `Walked`
entries when a walk strictly improves a label; `reconstruct_journey`
chains through walks within a round (no `inner_k` decrement on a walk
step) so walk-then-board, board-walk-board, and board-then-walk-to-pt
journeys all survive reconstruction.

The public `Journey::plan` remains route-only — a journey ending in a
walk is emitted with the last *boarded* alight stop in the plan and
the walk-derived arrival time. Surfacing walk steps in the public API
is a future enhancement; the current shape preserves the existing
contract.

The hegel proptest harness layers 2 and 3 turned green on this fix and
their `#[ignore]` attributes have been removed. See `soundness.md`
Issue I (resolved).

### 0.6 Route-pattern splitting in the GTFS adapter

**Status:** broken. `cache_trips_for_routes` sorts trips by *first-stop*
departure time and uses `partition_point` to binary-search at an
*arbitrary intermediate* stop. This is unsound when:

- Two trips on the same `route_id` have different stop sequences (very
  common in GTFS — short-turns, branching, deadheads).
- Two trips share a stop sequence but overtake each other (rarer; some
  feeds have express + local on the same route_id).

The TODO at line 98 acknowledges the first case. The current code
silently returns wrong answers for both.

**Fix:** at construction time, split each GTFS `route_id` into one or
more *RAPTOR routes* — equivalence classes of trips with identical stop
sequences. Concretely:

1. For each trip, compute its stop sequence as a `Vec<&str>`.
2. Use `(route_id, stop_sequence)` as the key for the synthetic route
   map. This gives one synthetic route per pattern.
3. Within each synthetic route, sort trips by departure at the *first
   stop*. Because all trips share a stop pattern, this implies sorted
   by departure at every stop — *unless* trips overtake.
4. Detect overtaking at construction time: for each consecutive pair of
   trips, check that arrival/departure times are monotonic at every
   stop. If overtaking exists, either split further (into non-overtaking
   sub-groups) or document the assumption and refuse to construct.

This needs a new internal type — `RouteId(u32)` or similar — distinct
from the user-facing `&str` route IDs. Expose the original route IDs
through a `route_name(RouteId) -> &str` accessor on the timetable for
display purposes (e.g., "take the 39A").

### 0.7 Footpath transitivity assumption

**Status:** undocumented. The paper assumes the footpath relation is
transitively closed — i.e., if A↔B and B↔C are footpaths, then A↔C is
either also a footpath or A→C is reachable in one stage. Without this,
a single round of footpath relaxation isn't sufficient.

**Fix (short-term):** document the requirement on the `Timetable` trait
and on `GtfsTimetable::new`. Most well-formed GTFS feeds satisfy it
already (transfers.txt entries tend to be explicit pairs).

**Fix (medium-term):** in `GtfsTimetable::new`, optionally compute the
transitive closure of the footpath graph at construction with a maximum
walking radius. This is what the OpenTripPlanner ecosystem does. Gate
behind a feature flag because it's expensive on large feeds.

### 0.8 Saturating arithmetic on Tau

**Status:** partially landed on v0.3 branch — `relax_footpaths_round`
uses `saturating_add` for the walk-arrival computation. The trip
arithmetic in route scanning still trusts the underlying timetable.
Outstanding: audit any remaining `Tau` arithmetic and switch to
`saturating_add`/`saturating_sub`.

### 0.9 Property-based correctness test

**Status:** landed (Hegel-based). Lives in `raptor/src/proptest_support/`.
See the module's `README.md` for the trip-count convention, the layer-to-
soundness-issue map, and the wall-clock budget.

The harness has three generator layers, all green on the v0.3 branch:

- **Layer 1** (no footpaths) covers the regime the hand-written tests
  cover, protecting against regressions in known-good territory.
- **Layers 2 and 3** generate footpath-bearing networks and were
  `#[ignore]`-flagged on v0.2.0 because they exposed issues A–D and I.
  Phase 0 (0.1–0.4 + 0.5b) resolved those, the flags came off, and they
  now run as part of `cargo nextest r`.

The reference solver is a time-expanded multi-criterion Dijkstra in
`reference.rs` (~200 lines, std-only). It uses atomic board+ride-segment
edges to avoid the "free re-ride" trap where separate board/ride/wait
edges admit unboarded trips.

Two implementation choices made during the build that diverge from the
original handoff brief and are documented in the module's README:

- The brief specified "ps == pt → {(tau, 0)}". The algorithm's
  `reconstruct_journey` filters out empty plans, so RAPTOR returns `[]`.
  The reference solver matches that convention, and also drops walk-
  only journeys for `ps != pt` (`k == 0`) for the same reason.
- Trips on the same generated route share `leg_durations` and
  `dwell_times`, making overtaking structurally impossible. This is
  stricter than the paper but every spec produced is a valid RAPTOR
  input. Loosening this is a reasonable future enhancement.

---

## Phase 1 — Make it fast (performance)

Once correctness is locked down, the big perf wins are about data
representation. The current `BTreeMap<(K, Stop), Tau>` design is
~10–100× off optimal on realistic networks.

### 1.1 Intern stops, routes, trips to dense `u32` indices

**Why:** every BTreeMap lookup is `O(log(K · |S|))` with a generic
comparison. Dense indexing turns lookups into array indexing.

**How:** in `GtfsTimetable::new`, build:

- `Vec<&str>` for each entity type (stops, routes, trips) — index → ID.
- `HashMap<&str, u32>` for the reverse direction at construction time
  only (drop after construction unless we add a "look up by string ID"
  method to the public API).

Internally, the algorithm operates entirely on `u32` indices.
Externally, the `Timetable` trait stays generic — but for `GtfsTimetable`
specifically, `type Stop = u32`. Users who want `&str` lookups go through
a thin display layer.

Decision point: do we keep the generic trait? Yes, but add a
sub-trait `IndexedTimetable: Timetable<Stop = u32, Route = u32, Trip = u32>`
that the algorithm specialises on. The generic version stays for tests
and for users with weird custom backends.

### 1.2 `Vec<Vec<Tau>>` labels instead of `BTreeMap<(K, Stop), Tau>`

Once stops are dense indices, labels become:

```rust
// labels[k][stop_idx] = earliest arrival at stop_idx with at most k transfers
let mut labels: Vec<Vec<Tau>> = vec![vec![Tau::MAX; n_stops]; transfers + 1];
labels[0][ps_idx] = tau;
```

Carry-forward is `labels[k] = labels[k-1].clone()` — one allocation per
round, ~`n_stops * 8` bytes. For 50k stops × 10 rounds, that's 4MB —
trivial.

The route-scan inner loop now does no map lookups at all, just array
indexing. Branch predictors love this.

### 1.3 Stop-pattern indexing for `get_earliest_trip`

The current `get_earliest_trip` does a binary search over a route's
trips, then a `find_stop_time` linear scan inside the chosen trip to
look up departure time at the queried stop. That linear scan is
`O(|stops_in_route|)` per call, called once per stop per route per
round.

**Fix:** at construction time, build per-(route, stop) sorted arrays of
`(departure_time, trip_idx)`. Lookup becomes one binary search, no
linear scan. Memory cost: one `(u32, u32)` per (trip, stop) — a few
hundred MB for the largest national feeds, fine for in-memory routing
servers, possibly too much for embedded use cases. Gate the
representation choice behind a config option if needed.

After the route-pattern splitting from 0.6, this is even cleaner: each
synthetic route has a fixed stop sequence, so we can flatten to
`departures[route_idx][stop_pos][trip_pos] -> Tau`.

### 1.4 Marked-stops as a bitset

`BTreeSet<Stop>` for marked stops costs `O(log n)` insert and ordered
iteration we don't need. With dense indices, swap in a `FixedBitSet`
(or a `Vec<bool>` if we want to avoid the dep). Marking is a single
bit write; iteration walks set bits. The `bit-set` or `fixedbitset`
crates are mature.

We do still need to iterate marked stops in some order for
deterministic output, but bitset iteration is naturally ordered by
index, which is good enough.

### 1.5 Re-use allocations across calls

`Q`, `marked_stops`, `more_marked_stops`, the labels arrays — all
currently allocated per call. For server use cases doing thousands of
queries against the same timetable, this dominates.

**Fix:** introduce a `RaptorCache` struct holding the scratch buffers,
and a `Timetable::raptor_with_cache(&self, cache: &mut RaptorCache, …)`
method. The non-cached version delegates with a fresh cache. Document
that callers doing many queries should reuse a cache.

### 1.6 Parallel queries

Different `RaptorCache` instances are independent. A web server can hold
a `&'static GtfsTimetable` and a per-request cache, and run as many
queries in parallel as it has cores. Document this and add an
integration test using `rayon::par_iter`.

For *single-query* parallelism (e.g., bidirectional search, or
multi-source Pareto), see the McRAPTOR section below — that's a much
bigger structural change and probably out of scope until v0.4 or v0.5.

### 1.7 Benchmark harness on realistic data

The current benchmarks run on synthetic grids and hub-and-spoke
networks. Useful for spotting regressions, but the absolute numbers
don't tell you whether the implementation is fast on real GTFS.

**Fix:** add a benchmark group that runs against a known public feed
(NYC MTA, BART, TfL, or a smaller one for CI speed — the Dublin Bus
feed in `aux/dmrc_gtfs.zip` is a good baseline). Track:

- 95th-percentile query latency
- Allocation count per query (use `dhat-rs` or `tracking-allocator`)
- Memory footprint of the loaded timetable

Compare against published RAPTOR numbers — the original paper reports
sub-millisecond queries on London and Madrid. If we're 10× slower than
that, we know there's still work to do.

---

## Phase 2 — McRAPTOR-ready label representation

Even if we don't ship multi-criteria routing in v0.3, we should
restructure labels *now* so that adding it later isn't a rewrite.

### 2.1 The `Label` trait

```rust
pub trait Label: Copy + Ord + Debug {
    /// The "departure-time" component of the label; this is what gets
    /// initialized at the source.
    fn from_departure(tau: Tau) -> Self;

    /// Combine a label with the cost of riding a trip to a new stop.
    fn extend_by_trip(self, arrival_at_new_stop: Tau) -> Self;

    /// Combine a label with the cost of walking a footpath.
    fn extend_by_footpath(self, transfer_time: Tau) -> Self;

    /// Returns true if `self` weakly dominates `other`: every component
    /// of self is ≤ the corresponding component of other.
    fn dominates(&self, other: &Self) -> bool;
}
```

Single-criterion RAPTOR is then:

```rust
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ArrivalTime(pub Tau);

impl Label for ArrivalTime {
    fn from_departure(tau: Tau) -> Self { ArrivalTime(tau) }
    fn extend_by_trip(self, arrival: Tau) -> Self { ArrivalTime(arrival) }
    fn extend_by_footpath(self, dt: Tau) -> Self {
        ArrivalTime(self.0.saturating_add(dt))
    }
    fn dominates(&self, other: &Self) -> bool { self.0 <= other.0 }
}
```

### 2.2 Multi-criterion labels

For McRAPTOR, a label becomes a `Vec<Self>` (a Pareto front) per stop
per round, not a single value. The route-scan stage updates the front by
inserting candidate labels and removing dominated ones. This is the
shape laid out in §6 of the journal paper.

Concrete criteria worth supporting (in order of usefulness):

- **Number of transfers** — already implicit in the round structure, but
  exposing it as a label component makes the algorithm work for queries
  like "minimize walking time, but don't exceed 3 transfers".
- **Total walking time** — sum of all footpath traversals. Useful for
  accessibility-aware routing.
- **Reliability / buffer time** — minimum transfer slack across the
  journey. The TypeScript `cata-dev` impl supports this. Useful for
  routes with unreliable schedules.
- **Fare zones crossed** — for systems with zonal fares (TfL, RATP).
  Trickier because fares aren't linear; might be better handled as a
  post-processing filter.

### 2.3 The bag-of-labels representation

For each `(round, stop)` we now hold a *bag* — a set of mutually
non-dominated labels. Operations:

- `insert(label)` — add label, remove anything it dominates, do nothing
  if it's dominated by an existing label.
- `extend_by_trip(arrival_at_stop)` — apply to every label in the bag.
- `merge(other)` — Pareto union.

For 1–3 criteria the bag is small (typically ≤ 10 labels) and a
`SmallVec<[Label; 8]>` outperforms anything fancier. Document the
expected bag size and gate at e.g. 64 labels per stop per round to
prevent pathological growth — if we ever hit that limit on a real feed,
something is wrong.

### 2.4 Migration strategy

1. Define the `Label` trait and `ArrivalTime` impl in v0.3 alongside the
   existing single-criterion code path. Don't switch internal code over
   yet.
2. In v0.4, refactor the algorithm to be generic over `L: Label`, with
   the bag-of-labels representation. Single-criterion users see no
   behaviour change; the bag is always size 1.
3. In v0.5, ship multi-criterion labels for the use cases above and a
   `McRaptor` API.

This way each release is independently shippable and testable.

---

## Phase 3 — API and feature surface

Ordered roughly by user-visible value.

### 3.1 Range queries (rRAPTOR)

Most users don't actually want "best journey departing at exactly
08:00:00" — they want "best journeys departing in the 08:00–09:00
window". rRAPTOR (described in §4 of the paper) does this efficiently
by sharing work across departure times.

This needs a separate algorithm path; can't be retrofitted onto plain
RAPTOR. Worth doing because every routing UI eventually wants it.

### 3.2 GTFS-RT (real-time updates)

Real-time delays, cancellations, and added trips. The standard pattern
is to keep the static timetable immutable and overlay a delta structure
that's queried alongside it. Adapter pattern: `RealtimeTimetable<T:
Timetable>` wraps a static timetable and an `arc-swap`-protected delta.

The note at `gtfs.rs:47` ("can use docs.rs/arc-swap's cache for
realtime support") suggests the author has thought about this. Worth
designing the trait now so the data flow is clear, even if we don't
implement the GTFS-RT parser until v0.6.

### 3.3 Calendar / service-day handling

Currently, the GTFS adapter ignores `calendar.txt` and `calendar_dates.txt`
entirely — every trip is assumed to run on every day. This is wrong on
real feeds, where trips have service patterns (weekdays only, Sundays
only, holidays excluded).

**Fix:** at construction time, accept a service date and filter trips
to those active on that date. Or store per-trip service IDs and resolve
at query time. The former is simpler; the latter supports multi-day
queries. Start with the former.

### 3.4 First-class footpath construction from stop coordinates

GTFS feeds vary in transfer.txt completeness. Many feeds have nothing,
expecting routers to compute walkable transfers from stop coordinates +
a max distance. Add a helper:

```rust
impl<'a> GtfsTimetable<'a> {
    pub fn with_walking_footpaths(
        self,
        max_distance_m: f64,
        walking_speed_m_per_s: f64,
    ) -> Self { ... }
}
```

Use a spatial index (kd-tree or R*-tree — `rstar` is the obvious
choice given the existing geo ecosystem context) to find nearby stop
pairs in `O(n log n)` rather than `O(n²)`. Compute great-circle
distance, divide by walking speed, store as transfer time.

### 3.5 Multi-source / multi-target queries

Real geocoders give you a *set* of source stops (with walking times
from the user's GPS to each) and a set of target stops. Plain RAPTOR
takes single source/target. The standard fix:

- For multi-source: initialize round 0 labels at every source stop with
  the corresponding walking time. The algorithm doesn't need to change.
- For multi-target: at the end of each round, take the Pareto front
  across all target stops + their walking times. Slightly more
  invasive — the target-pruning condition has to consider all targets.

Add `RaptorQuery` builder with `.from_stops(&[(stop, walk_time)])` and
`.to_stops(&[(stop, walk_time)])` methods. Single-stop case becomes
sugar over this.

### 3.6 Journey reconstruction with full timing

Currently, `Journey.plan` is `Vec<(Route, Stop)>` — just the topology.
Users often want timing too: "board R1 at A at 08:03, arrive at B at
08:17". Add a `with_timing` method on `Journey` that walks the plan
against the timetable and produces `Vec<JourneyLeg>` with arrival,
departure, and trip ID per leg.

### 3.7 Accessibility flags

GTFS has `wheelchair_boarding` on stops and `wheelchair_accessible` on
trips. Add a query flag that filters trips/stops accordingly. Should be
a single boolean parameter on the query, plus a few more for things like
"avoid stairs" if the feed supports it.

### 3.8 Better error handling

Currently, the `simple` and `gtfs` adapters panic in several places
(e.g., `get_arrival_time` panics if the stop isn't on the trip's
sequence). For a library, panics are usually wrong — return `Result` or
`Option`.

The trait itself is fine to keep panic-free in its required methods
(implementors decide what to do with bad inputs), but the algorithm
should be resilient to None returns from accessors where it makes sense.

---

## Phase 4 — Engineering hygiene

Ongoing rather than sequential.

### 4.1 Documentation pass

The README is fine for an introduction. Missing:

- A complete worked example with timing reconstruction.
- A "porting from OpenTripPlanner / r5" guide for people coming from
  other ecosystems.
- Documented assumptions: footpath transitivity, no overtaking within a
  pattern, time-since-midnight semantics, what happens at day
  boundaries.
- A perf guide: "if you're running many queries, do this. If you have a
  big feed, do this."

### 4.2 CI on real feeds

Current CI runs unit tests. Add:

- A weekly job that loads a real public feed (Dublin Bus, since it's
  already in the repo at `aux/dmrc_gtfs.zip`) and runs a fixed set of
  queries. Compares results against a golden file. Flags regressions.
- Allocation tracking. `dhat-rs` or `tracking-allocator` integrated
  into a benchmark target. Alert on regressions.
- `cargo-mutants` run periodically. Mutation testing on the algorithm
  is a strong forcing function for test coverage — it'll find every
  branch we haven't covered.

### 4.3 Fuzzing

`cargo-fuzz` target that constructs random `Timetable` impls and runs
RAPTOR. Asserts no panics, no inf-loops, output well-formed. Cheap
insurance against the panic paths in the GTFS adapter and the various
`unwrap()`s in the algorithm.

### 4.4 MSRV policy

Pin a minimum supported Rust version. The `rust-toolchain.toml` pinning
to 1.93.1 is fine for development but the published crate should
support more. 1.75 (stable async traits) is a reasonable lower bound.

### 4.5 `no_std`?

Probably not worth it. RAPTOR uses heap allocation throughout, and the
target audience (transit routing servers) all have `std`. Skip unless
someone files an issue.

### 4.6 Workspace structure

Currently: `raptor`, `dotgraph` as workspace members. The `gtfs` module
lives inside `raptor` and is gated behind nothing. A future split:

- `raptor-core` — the algorithm and `Timetable` trait, no
  format-specific dependencies.
- `raptor-gtfs` — the GTFS adapter.
- `raptor-gtfsrt` — real-time overlay (when it lands).
- `raptor-cli` — a CLI that wraps the above for ad-hoc queries.
- `raptor` — facade crate that re-exports the others, gated behind
  feature flags. Keeps `cargo add raptor` working.

Don't do this until at least one of the format-specific crates is
substantial. Premature splitting just adds friction.

---

## Suggested release plan

Each release is independently usable, and the roadmap is structured so
that earlier releases don't constrain the design of later ones.

- **v0.3 (Correctness):** Phase 0 in full (0.1–0.4 + 0.5b are landed;
  0.5, 0.6, 0.7, 0.8 outstanding), plus 1.5 (allocation reuse) and 4.1
  (docs pass) because they're cheap. The property-based test against
  the reference solver is green across all three generator layers.
  Announcement post: "raptor-rs now produces correct results."

- **v0.4 (Performance):** Phase 1 in full. Benchmark numbers within
  3× of the published RAPTOR figures. Announcement post: "raptor-rs is
  now fast enough for production servers."

- **v0.5 (McRAPTOR readiness):** Phase 2 in full. The single-criterion
  API is stable; the multi-criterion API is alpha. No announcement —
  internal restructuring.

- **v0.6 (Multi-criterion + range):** McRAPTOR shipped, rRAPTOR shipped
  (3.1). Announcement post: "raptor-rs supports multi-criterion and
  range queries."

- **v0.7 (Real-time):** GTFS-RT (3.2), service days (3.3), accessibility
  (3.7).

- **v1.0:** all of the above stable, two real-world deployments,
  property-based + fuzzing CI green for 3 consecutive months. Lock the
  API.

Skip whatever isn't pulling its weight; this is a rough sequence, not a
contract. The two non-negotiables are (a) Phase 0 lands before anyone
uses this in anger, and (b) the property-based test against a reference
solver lives in the repo from v0.3 onwards.

---

## Out of scope (explicitly)

To stop scope creep, things this roadmap does *not* try to do:

- **Contraction Hierarchies / Transit Node Routing.** Different
  algorithm class entirely. RAPTOR is competitive enough for transit
  alone; CH/TNR territory is "transit + walking + cycling integrated"
  and that's a different library.
- **Public Transit Labeling (PTL).** Would beat RAPTOR on raw query
  speed but the preprocessing cost and memory footprint are
  significantly higher. Worth a feasibility study at v1.0+, not now.
- **Routing on combined transit + road networks.** Same reason.
- **GTFS-Pathways (in-station walking graphs).** A natural extension of
  the footpath model but a substantial spec to support. Defer.
- **Web service / HTTP API.** Out of scope for the library itself.
  Someone can build it on top.
  