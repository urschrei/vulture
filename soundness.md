# RAPTOR Implementation Soundness Analysis

This document analyzes the soundness of the RAPTOR implementation in
`raptor/src/lib.rs` against the original paper:

> Delling, D., Pajor, T., & Werneck, R. F. (2015). *Round-Based Public Transit
> Routing.* Transportation Science 49(3): 591–604.
> DOI: [10.1287/trsc.2014.0534](https://doi.org/10.1287/trsc.2014.0534).

An earlier conference version appeared as Delling, Pajor & Werneck (2012),
*Round-Based Public Transit Routing*, ALENEX. Algorithm line numbers below
refer to Algorithm 1 in the journal version.

This revision reflects the state of the code on the v0.3 development
branch. Prior versions of this document described several issues that
have since been fixed; those are summarized in the *Resolved Issues*
section at the end for historical reference.

The footpath/round-label issues A–D were resolved by the Phase 0 work
on this branch and now live in *Resolved Issues* as well.

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
- **Local pruning**: Track τ\*(pᵢ) — earliest arrival at pᵢ across all
  rounds.
- **Target pruning**: Don't update or mark stops whose arrival exceeds
  τ\*(pₜ).

---

## Outstanding Soundness Issues

(none on the v0.3 branch — Phase 0 is complete.)

A possible future hardening: an opt-in transitive-closure pass during
`GtfsTimetable::new` (gated behind a feature flag and a max-walking-
distance parameter) for feeds whose `transfers.txt` is not transitively
closed, mirroring what OpenTripPlanner does. This is a feature
enhancement, not a soundness fix — the current contract documents the
closure requirement explicitly and well-formed GTFS feeds satisfy it.

---

## Summary Table

All catalogued soundness issues (A–I) are resolved on the v0.3 branch.
See *Resolved Issues* below for the per-issue write-ups.

---

## Impact Assessment

**Correctness on real GTFS feeds**: A–I are all resolved on v0.3. No
critical soundness issues remain in the algorithm or the GTFS adapter.
The hegel-based property test in the `raptor-proptest` workspace crate is
green across all three generator layers (footpaths included). A
separate property-test harness over `GtfsTimetable` is on the wish list
(roadmap step 4.2: CI on real feeds with golden files).

---

## Resolved Issues

The following issues were identified in earlier revisions of this
document and have since been fixed. R1–R5 were resolved by v0.2.0;
A–I were resolved on the v0.3 development branch.

### A: Round labels are not carried forward — **Fixed (v0.3)**

**Was**: labels lived in `BTreeMap<(K, Stop), Tau>` and entries were
only inserted on improvement, so footpath relaxation in round k could
not see arrivals from round k−1 that had not been re-improved.

**Now**: labels are stored in `Vec<BTreeMap<Stop, Tau>>` indexed by
round. At the top of each round, `labels[k] = labels[k-1].clone()`
seeds round k with the previous round's values. Footpath origins
reached in earlier rounds remain visible.

### B: Footpath relaxation from the source missing in round 1 — **Fixed (v0.3)**

**Was**: only `(0, ps) = tau` was set at init; the footpath stage ran
inside the round loop and never relaxed walks from `ps` itself before
round 1's route scanning.

**Now**: the footpath logic is extracted into `relax_footpaths_round`
and called once at init for round 0. Combined with A, walk-reachable
neighbours of `ps` carry forward into round 1 and are usable as
boarding points for the first route scan.

### C: τ\* not updated in the footpath stage — **Fixed (v0.3)**

**Was**: walk-derived label updates were written to
`best_arrival_per_k` only; `best_arrival` (τ\*) remained stale, leaving
local and target pruning unaware of walk-reached arrivals.

**Now**: `relax_footpaths_round` updates both `labels[k]` and
`best_arrival` whenever a walk strictly improves an arrival.

### D: No target pruning in the footpath stage — **Fixed (v0.3)**

**Was**: every walk-reachable stop was unconditionally pushed into the
marked set, including stops whose arrival exceeded τ\*(pₜ).

**Now**: `relax_footpaths_round` only marks `p_dash` if the walk-derived
arrival strictly improves on `best_arrival[pt]`.

### E: GTFS adapter conflated route_id with RAPTOR route — **Fixed (v0.3)**

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

This is a public API change — `Journey`'s `Route` type for this
adapter is now `RouteId` rather than `&str`. The binary search in
`get_earliest_trip` is now sound because every synthetic route's trips
share a stop sequence and pairwise do not overtake.

### F: Output is not Pareto-filtered — **Fixed (v0.3)**

**Was**: `Timetable::raptor` collected one journey per
`k ∈ 1..=transfers` from `reconstruct_journey` without an explicit
output-side Pareto filter. With local/target pruning leaky (Issue C),
dominated journeys could leak into the output.

**Now**: after collecting the journeys, `Timetable::raptor` sorts them
by trip count ascending and retains only those whose arrival is
strictly less than the best seen so far. The trait doc documents the
contract: arrival strictly decreases as trip count increases. Output
is deterministic and independent of pruning correctness.

### I: Journey reconstruction cannot trace through walk legs — **Fixed (v0.3)**

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
entries without decrementing the round index — walks happen within a
round and do not consume a trip. The hegel proptest layers 2 and 3
(footpath-bearing networks) turned green on this fix.

The public `Journey::plan` remains route-only for API stability; a
journey ending in a walk is emitted with its last *boarded* alight stop
in the plan and the walk-derived arrival time. Surfacing walk steps in
the public API is a future enhancement.

### G: Saturating arithmetic on `Tau` everywhere — **Fixed (v0.3)**

**Was**: the v0.2.0 footpath stage added `transfer_time` without
`saturating_add`, allowing wrap on misconfigured input.

**Now**: `relax_footpaths_round` is the only site in the algorithm
that combines `Tau` values arithmetically, and it uses `saturating_add`.
A 0.8 audit confirmed there is no other `Tau` arithmetic in
`Timetable::raptor`, in the simple adapter, or in the GTFS adapter
(beyond reading values out of the underlying timetable, which the
algorithm only compares, never combines).

### H: Footpath transitivity is undocumented — **Fixed (v0.3)**

**Was**: the assumption that the footpath relation is transitively
closed was implicit. Feeds whose `transfers.txt` derives from
coordinate-radius rules can violate it without warning, causing missed
journeys.

**Now**: the `Timetable` trait documents the closure requirement at
the trait level and on `get_footpaths_from`. `GtfsTimetable::new`
documents that it passes `transfers.txt` through unmodified, leaving
closure to the caller (or to a future opt-in feature).

### R1: `get_earliest_trip` missing time parameter — **Fixed**

**Was**: the `Timetable::get_earliest_trip` trait method took only
`(route, stop)`, with no way to specify the minimum departure time
required to catch the trip. The route-scan stage therefore could not
correctly identify catchable trips.

**Now**: the trait signature is

```rust
fn get_earliest_trip(
    &self,
    route: Self::Route,
    at: Tau,
    stop: Self::Stop,
) -> Option<Self::Trip>;
```

(`raptor/src/lib.rs:151–156`), and the call site at line 240 passes the
correct `t_prev_pi = τₖ₋₁(pᵢ)` value. The `simple` and `gtfs` adapters
implement this correctly.

### R2: Footpath transfer time not added to walking arrivals — **Fixed**

**Was**: the footpath stage took the minimum of the destination's
existing arrival and the origin's arrival, without adding the walking
duration. Footpaths were effectively teleportation.

**Now**: `relax_footpaths_round` adds
`self.get_transfer_time(stop, p_dash)` via `saturating_add`.

### R3: `get_stops_after` semantics ambiguous — **Fixed**

**Was**: it was unclear whether `get_stops_after(route, stop)` should
include `stop` itself. The paper's "for each stop pᵢ of r beginning with
p" requires inclusive semantics, but the trait did not document this.

**Now**: both the `simple` adapter (`mod.rs:142–146`, returns
`stops[pos..]`) and the GTFS adapter (`gtfs.rs:201–213`, same pattern)
use inclusive semantics. The trait documentation at `lib.rs:145–146`
confirms: "Returns all stops on a route from the given stop onwards
(inclusive)".

### R4: Loop range excluded the final round — **Fixed**

**Was**: `for k in 1..transfers` (exclusive upper bound), so a query
with `transfers=3` ran only rounds 1 and 2.

**Now**: `for k in 1..=transfers` at `raptor/src/lib.rs:202`. Inclusive,
matching the paper.

### R5: Trip update logic depended on broken `get_earliest_trip` — **Fixed**

**Was**: a downstream consequence of R1 — the trip-hop logic at the
inner loop could not correctly determine when to switch trips because
the underlying accessor was unsound.

**Now**: the logic at `raptor/src/lib.rs:234–242` correctly uses
`t_prev_pi` as the lower bound when calling `get_earliest_trip`, and the
trait method now respects it. The remaining concern in this area is
covered by Issue E (the GTFS adapter's binary-search-over-route-trips is
still unsound for a different reason — multiple stop patterns per
`route_id`).
