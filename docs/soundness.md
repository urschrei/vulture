# Soundness analysis

This document checks the implementation in `vulture/src/algorithm/`
against the original paper:

> Delling, D., Pajor, T., & Werneck, R. F. (2015). *Round-Based Public Transit
> Routing.* Transportation Science 49(3): 591–604.
> DOI: [10.1287/trsc.2014.0534](https://doi.org/10.1287/trsc.2014.0534).

An earlier conference version appeared as Delling, Pajor & Werneck (2012),
*Round-Based Public Transit Routing*, ALENEX. Algorithm line numbers below
refer to Algorithm 1 in the journal version.

The text reflects the current state of the implementation. All
catalogued soundness issues have been fixed and are described in the
*Resolved issues* section at the end as historical record. Live
validation runs through the [`vulture-proptest`](../vulture-proptest/)
property-test harness, which compares the algorithm against a
brute-force reference solver on randomly generated networks across
three difficulty layers (transit-only, transit + footpaths,
transit + closed footpaths) on every test run.

---

## RAPTOR Algorithm Overview

RAPTOR computes all Pareto-optimal journeys jointly minimizing **arrival
time** and **number of transfers** in a public transit network.

### Core Data Structures

- **Timetable**: (Π, S, T, R, F) where:
  - Π = period of operation (e.g., seconds of a day)
  - S = set of stops
  - T = set of trips
  - R = set of routes (equivalence classes of trips with identical stop
    sequences)
  - F = set of footpaths (walking transfers)

- **Multilabel**: Each stop p has labels (τ₀(p), τ₁(p), ..., τₖ(p)) where
  τᵢ(p) is the earliest known arrival at p using at most i trips.
  The implementation generalises this to a *bag* (Pareto front) of
  labels per (round, stop), so multi-criterion `Label` impls produce
  real Pareto fronts at the targets. The default single-criterion
  `ArrivalTime` collapses each bag to size 1 and behaves identically
  to the paper's per-cell scalar.

### Algorithm Structure

**Initialization:**
- Set all τᵢ(p) = ∞
- Set τ₀(pₛ) = τ (departure time from source)
- Mark source stop pₛ
- Relax footpaths from pₛ at round 0

**Per Round k (computing journeys with k trips / k−1 transfers):**

1. **Stage 1**: Set τₖ(p) ← τₖ₋₁(p) for all stops (upper bound from
   previous round).

2. **Stage 2**: Traverse routes
   - Collect routes Q serving marked stops, picking the earliest marked
     stop on each.
   - For each (route r, boarding stop p):
     - Walk stops of r starting at p; on each pᵢ, hop to an earlier trip
       if τₖ₋₁(pᵢ) < τ_dep(t, pᵢ).
     - Update τₖ(pⱼ) = τ_arr(t, pⱼ) when this improves on τ\*(pⱼ) and
       τ\*(pₜ).

3. **Stage 3**: Process footpaths
   - For each (pᵢ, pⱼ) ∈ F: τₖ(pⱼ) ← min{τₖ(pⱼ), τₖ(pᵢ) + ℓ(pᵢ, pⱼ)}.

**Optimizations:**
- **Marking**: Only routes containing stops improved in the previous round
  are scanned.
- **Local pruning**: Track τ\*(pᵢ) – earliest arrival at pᵢ across all
  rounds.
- **Target pruning**: Don't update or mark stops whose arrival exceeds
  τ\*(pₜ).

---

## Outstanding soundness issues

None known. The closure-of-footpaths concern that earlier versions of
this document carried is now addressed in two complementary ways
(*transitive closure* of the footpath relation means every walk
reachable through a chain of direct edges is already present as a
single direct edge – `A → B` and `B → C` implies `A → C` with the
combined walk time; see the [`Timetable` trait docs][tt-fp] and
[Wikipedia][tc]):

- The `Timetable` trait documents the closure requirement explicitly,
  and the algorithm uses multi-source Dijkstra inside each round's
  footpath relaxation to chain non-closed walks `A → B → C` correctly.
  See [`vulture/src/algorithm/footpaths.rs`](../vulture/src/algorithm/footpaths.rs).
- For adapters whose `transfers.txt` *is* publisher-curated and
  transitively closed (Berlin VBB, Paris IDFM), the
  `Timetable::footpaths_are_transitively_closed` opt-in switches to
  the cheaper single-pass `O(E)` relaxation.

[tt-fp]: https://docs.rs/vulture/latest/vulture/trait.Timetable.html#footpaths
[tc]: https://en.wikipedia.org/wiki/Transitive_closure

The earlier "wish list" item – an opt-in transitive-closure pass
during `GtfsTimetable::new` – has effectively been replaced by
[`GtfsTimetable::with_walking_footpaths`], which builds a coordinate-
derived walking graph from stop locations using an R-tree. This
augments rather than closes the existing relation, but it solves the
same real problem (sparse / empty `transfers.txt` in feeds like
Helsinki HSL).

[`GtfsTimetable::with_walking_footpaths`]: ../vulture/src/gtfs.rs

---

## Summary

All catalogued soundness issues are resolved. See *Resolved issues*
below for per-issue write-ups, ordered roughly by when each was
identified and fixed:

| ID | Topic | Fixed in |
|----|-------|----------|
| R1–R5 | Trait signature + miscellaneous algorithm gaps | v0.2.0 |
| A–D | Round-label carry-forward, footpath stage from source, τ\* updates, target pruning | v0.3 |
| E | GTFS adapter conflated route_id with RAPTOR route | v0.3 |
| F | Output not Pareto-filtered | v0.3 |
| G | Saturating arithmetic on `SecondOfDay` | v0.3 |
| H | Footpath-transitivity assumption undocumented | v0.3 |
| I | Journey reconstruction couldn't trace through walk legs | v0.3 |
| J | Calendar / service-day filtering | v0.6 |
| K | Loop routes (trips revisiting a stop) | v0.5 (Phase 0.11) |

The [`vulture-proptest`](../vulture-proptest/) harness validates
correctness against a brute-force reference solver on every commit;
500+ randomly generated layer-3 specs (transit + closed footpaths) per
run.

---

## Resolved issues

The following issues were identified in earlier revisions of this
document and have since been fixed. R1–R5 were resolved by v0.2.0;
A–I were resolved on the v0.3 development branch; J and K landed
later (Phase 0.10 and Phase 0.11 respectively, against bugs surfaced
by the cross-city benchmarks on real feeds).

### A: Round labels are not carried forward – **Fixed (v0.3)**

**Was**: labels lived in `BTreeMap<(K, Stop), SecondOfDay>` and entries were
only inserted on improvement, so footpath relaxation in round k could
not see arrivals from round k−1 that had not been re-improved.

**Now**: labels live in `Vec<Vec<LabelBag<L>>>` indexed by `(round,
stop)`. At the top of each round k, the algorithm carries forward
the bag at every stop reached so far in any previous round (sparse
carry-forward via the `ever_reached` bitset, so the cost is `O(reached)`
rather than `O(n_stops)`). Footpath origins reached in earlier rounds
remain visible. The Phase 1 data-representation rewrite replaced the
original v0.3 `Vec<BTreeMap<Stop, SecondOfDay>>` shape; Phase 2's
multi-criterion work then generalised the inner cell from
`SecondOfDay` to `LabelBag<L>`.

### B: Footpath relaxation from the source missing in round 1 – **Fixed (v0.3)**

**Was**: only `(0, ps) = tau` was set at init; the footpath stage ran
inside the round loop and never relaxed walks from `ps` itself before
round 1's route scanning.

**Now**: the footpath logic is extracted into `relax_footpaths_round`
and called once at init for round 0. Combined with A, walk-reachable
neighbours of `ps` carry forward into round 1 and are usable as
boarding points for the first route scan.

### C: τ\* not updated in the footpath stage – **Fixed (v0.3)**

**Was**: walk-derived label updates were written to
`best_arrival_per_k` only; `best_arrival` (τ\*) remained stale, leaving
local and target pruning unaware of walk-reached arrivals.

**Now**: `relax_footpaths_round` updates both `labels[k]` and
`best_arrival` whenever a walk strictly improves an arrival.

### D: No target pruning in the footpath stage – **Fixed (v0.3)**

**Was**: every walk-reachable stop was unconditionally pushed into the
marked set, including stops whose arrival exceeded τ\*(pₜ).

**Now**: `relax_footpaths_round` only marks `p_dash` if the walk-derived
arrival strictly improves on `best_arrival[pt]`.

### E: GTFS adapter conflated route_id with RAPTOR route – **Fixed (v0.3)**

**Was**: `cache_trips_for_routes` indexed trips by GTFS `route_id` and
sorted them by first-stop departure. `get_earliest_trip` then binary-
searched at an *intermediate* stop, which is unsound when (a) trips on
the same `route_id` have different stop sequences (short-turns,
branching), or (b) two trips overtake each other on the same pattern.
Both modes silently returned wrong answers.

**Now**: `GtfsTimetable::new` groups trips by `(route_id,
stop_sequence)`, sorts each group by first-stop departure, and
greedily splits each group into non-overtaking sub-groups
(`split_non_overtaking`). Each sub-group becomes a synthetic
`RouteId` (a newtype around `u32`); the algorithm operates entirely on
`RouteId`s. The original GTFS `route_id` is recoverable for display via
`GtfsTimetable::route_name`.

This is a public API change – `Journey`'s `Route` type for this
adapter is now `RouteId` rather than `&str`. The binary search in
`get_earliest_trip` is now sound because every synthetic route's trips
share a stop sequence and pairwise do not overtake.

### F: Output is not Pareto-filtered – **Fixed (v0.3)**

**Was**: `Query::run` (dispatching through the algorithm free fn `run_per_call_query`) collected one journey per
`k ∈ 1..=transfers` from `reconstruct_journey` without an explicit
output-side Pareto filter. With local/target pruning leaky (Issue C),
dominated journeys could leak into the output.

**Now**: after collecting the journeys, `Query::run` (dispatching through the algorithm free fn `run_per_call_query`) sorts them
by trip count ascending and retains only those whose arrival is
strictly less than the best seen so far. The trait doc documents the
contract: arrival strictly decreases as trip count increases. Output
is deterministic and independent of pruning correctness.

### I: Journey reconstruction cannot trace through walk legs – **Fixed (v0.3)**

**Was**: the boarding tree recorded only route-scan boardings, so
reconstruction broke whenever the optimal path involved a walk leg
(walk-then-board, board-walk-board, or board-then-walk-to-pt). RAPTOR
computed correct arrival times after A–D landed but could not emit the
corresponding journeys.

**Now**: the boarding tree is keyed by
`(K, Stop) -> Step<Route, Stop>` where

```rust
enum Step<Route, Stop> {
    Boarded { from: Stop, route: Route },
    Walked { from: Stop },
}
```

`relax_footpaths_round` inserts `Walked` entries whenever a walk
strictly improves a label. `reconstruct_journey` chains through walk
entries without decrementing the round index – walks happen within a
round and do not consume a trip. The hegel proptest layers 2 and 3
(footpath-bearing networks) turned green on this fix.

The public `Journey::plan` remains route-only for API stability; a
journey ending in a walk is emitted with its last *boarded* alight stop
in the plan and the walk-derived arrival time. Surfacing walk steps in
the public API is a future enhancement.

### G: Saturating arithmetic on `SecondOfDay` everywhere – **Fixed (v0.3)**

**Was**: the v0.2.0 footpath stage added `transfer_time` without
`saturating_add`, allowing wrap on misconfigured input.

**Now**: `relax_footpaths_round` is the only site in the algorithm
that combines `SecondOfDay` values arithmetically, and it uses `saturating_add`.
A 0.8 audit confirmed there is no other `SecondOfDay` arithmetic in
`Query::run` (dispatching through the algorithm free fn `run_per_call_query`), in the simple adapter, or in the GTFS adapter
(beyond reading values out of the underlying timetable, which the
algorithm only compares, never combines).

### H: Footpath transitivity is undocumented – **Fixed (v0.3)**

**Was**: the assumption that the footpath relation is transitively
closed was implicit. Feeds whose `transfers.txt` derives from
coordinate-radius rules can violate it without warning, causing missed
journeys.

**Now**: the `Timetable` trait documents the closure requirement at
the trait level and on `get_footpaths_from`. The algorithm itself no
longer *requires* closure – Phase 0.7 rewrote `relax_footpaths_round`
to use multi-source Dijkstra inside each round, chaining direct walks
to a fixed point. Adapters whose `transfers.txt` *is* closed can opt
into the cheaper single-pass relaxation via
`Timetable::footpaths_are_transitively_closed`. `GtfsTimetable::new`
defaults to `false`; opt in with
`GtfsTimetable::assert_footpaths_closed()`.

### J: GTFS adapter ignored calendar / service-day filtering – **Fixed (v0.6, Phase 0.10)**

**Was**: the GTFS adapter loaded every trip in the feed regardless of
whether its `service_id` was active on the user's chosen date. For
real feeds (Berlin VBB, Helsinki HSL, Paris IDFM) only ~5–32% of
trips run on any given Monday; including the rest produced wrong
"earliest trip" answers from `get_earliest_trip` whenever an
inactive trip happened to be earlier than the active one.

**Now**: `GtfsTimetable::new(&gtfs, date)` takes a service date and
filters trips against `calendar.txt` and `calendar_dates.txt` per the
GTFS spec (`calendar_dates` exception trumps `calendar` day-of-week
flag, constrained by the service's `start_date`/`end_date`). Date
arithmetic uses [`jiff`](https://crates.io/crates/jiff); the
`gtfs-structures` chrono dates convert at the boundary. See
[`vulture/src/gtfs.rs`](../vulture/src/gtfs.rs) `is_service_active`.

### K: Routes that revisit a stop (loop routes) – **Fixed (v0.5, Phase 0.11)**

**Was**: GTFS allows a trip's `stop_sequence` to revisit the same
`stop_id` (bus loops, shuttles that turn around, terminus loops). The
v0.4 adapter collapsed each trip's stop sequence to `Vec<StopIdx>`
and used `Vec::position()` to find a stop's index within that
sequence – `position()` returns the *first* occurrence. So when the
algorithm asked "where on this route does stop X sit?", the answer
silently picked the first visit even when the journey actually
boarded or alighted at the second. On Paris IDFM this produced
ARR<DEP results in the cross-city benchmarks.

**Now**: position is part of the trait surface throughout. The
algorithm operates on `(route, position)` pairs rather than
`(route, stop)`; `Timetable::get_routes_serving_stop` returns
`&[(RouteIdx, u32)]` (the *earliest* position of the stop on each
route, with each route appearing once); `Timetable::get_stops_after`,
`get_earliest_trip`, `get_arrival_time`, and `get_departure_time` all
take an explicit `pos: u32` argument. Loop-route ambiguity is
resolved at the trait boundary; the algorithm is unaffected. The
hegel proptest harness's layer-3 generator emits loop routes; both
the per-call algorithm and the rRAPTOR scan stay sound.

### R1: `get_earliest_trip` missing time parameter – **Fixed**

**Was**: the `Timetable::get_earliest_trip` trait method took only
`(route, stop)`, with no way to specify the minimum departure time
required to catch the trip. The route-scan stage therefore could not
correctly identify catchable trips.

**Now**: the trait signature is

```rust
fn get_earliest_trip(
    &self,
    route: Self::Route,
    at: SecondOfDay,
    stop: Self::Stop,
) -> Option<Self::Trip>;
```

(`vulture/src/timetable.rs`), and the call site at line 240 passes the
correct `t_prev_pi = τₖ₋₁(pᵢ)` value. The `simple` and `gtfs` adapters
implement this correctly.

### R2: Footpath transfer time not added to walking arrivals – **Fixed**

**Was**: the footpath stage took the minimum of the destination's
existing arrival and the origin's arrival, without adding the walking
duration. Footpaths were effectively teleportation.

**Now**: `relax_footpaths_round` adds
`self.get_transfer_time(stop, p_dash)` via `saturating_add`.

### R3: `get_stops_after` semantics ambiguous – **Fixed**

**Was**: it was unclear whether `get_stops_after(route, stop)` should
include `stop` itself. The paper's "for each stop pᵢ of r beginning with
p" requires inclusive semantics, but the trait did not document this.

**Now**: both the `simple` adapter (`vulture/src/manual/mod.rs`, returns
`stops[pos..]`) and the GTFS adapter (`vulture/src/gtfs.rs`, same pattern)
use inclusive semantics. The trait documentation at `vulture/src/timetable.rs`
confirms: "Returns all stops on a route from the given stop onwards
(inclusive)".

### R4: Loop range excluded the final round – **Fixed**

**Was**: `for k in 1..transfers` (exclusive upper bound), so a query
with `transfers=3` ran only rounds 1 and 2.

**Now**: `for k in 1..=transfers` at `vulture/src/algorithm/per_call.rs` (the rounds loop in `run_raptor_rounds`). Inclusive,
matching the paper.

### R5: Trip update logic depended on broken `get_earliest_trip` – **Fixed**

**Was**: a downstream consequence of R1 – the trip-hop logic at the
inner loop could not correctly determine when to switch trips because
the underlying accessor was unsound.

**Now**: the logic at `vulture/src/algorithm/per_call.rs` (the route-scan inner loop) correctly uses
`t_prev_pi` as the lower bound when calling `get_earliest_trip`, and the
trait method now respects it. The remaining concern in this area is
covered by Issue E (the GTFS adapter's binary-search-over-route-trips is
still unsound for a different reason – multiple stop patterns per
`route_id`).
