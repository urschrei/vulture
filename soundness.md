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

### Issue E: GTFS `get_earliest_trip` assumes incorrectly that route_id implies a single stop pattern

**Severity**: Critical (for real GTFS feeds)

**Location**: `raptor/src/gtfs.rs:215–245`, supported by
`cache_trips_for_routes` at lines 133–156.

**Paper**: a "route" in RAPTOR is an equivalence class of trips with
*identical* stop sequences. The construction `R` of routes from raw
trip-pattern data is described in §3.1 of the journal version. A GTFS
`route_id` is *not* a RAPTOR route — it routinely groups trips with
different stop patterns (short-turns, branching, deadheads, express vs.
local).

**Current code**: `cache_trips_for_routes` (line 144) sorts trips on a
`route_id` by their *first-stop* departure time. `get_earliest_trip`
(line 236) then `partition_point`s over this list to find trips
departing at-or-after `at` from an *intermediate* stop:

```rust
let idx = trips.partition_point(|&trip| {
    departure_at_stop(trip).map(|dep| dep < at).unwrap_or(true)
});
```

**Problem**: when trips on a `route_id` have different stop sequences,
"sorted by first-stop departure" does not imply "sorted by intermediate-
stop departure" — different trips visit different first stops, or skip
the queried stop entirely. The fallback `.find` at line 241 only handles
trips that don't serve the stop; it does not recover a trip that *does*
serve the stop but was sorted "after" the partition point because of a
different first stop.

Even when all trips share a stop pattern, *trip overtaking* (e.g., an
express trip that overtakes a local) breaks the binary-search ordering.
RAPTOR assumes no overtaking within a route, but this assumption is only
sound after route-pattern splitting has been performed.

The TODO at `gtfs.rs:98` ("handle case where multiple trips run on a
route but with different patterns which require merging stops in a
meaningful way") acknowledges the first failure mode; the second is not
flagged anywhere.

**Fix**: at construction time, split each GTFS `route_id` into one or
more synthetic *RAPTOR routes*, each defined as the equivalence class of
trips sharing an identical stop sequence:

1. For each trip, compute its stop sequence as a tuple/Vec of stop IDs.
2. Key synthetic routes by `(route_id, stop_sequence)`.
3. Within each synthetic route, sort trips by first-stop departure. By
   construction, all trips share a stop pattern, so first-stop ordering
   implies ordering at every other stop — *unless* trips overtake.
4. At construction, verify monotonicity: for each consecutive trip pair
   on a synthetic route, check that arrival/departure times are
   monotonic at every stop. On detection of overtaking, either split
   further into non-overtaking sub-groups or surface a construction-
   time warning.

The user-facing route ID can be preserved via a side map for display
purposes; the algorithm operates on synthetic route indices.

---

### Issue G: Saturating arithmetic on `Tau` everywhere

**Severity**: Low

**Location**: `raptor/src/lib.rs`.

**Current code**: the footpath helper `relax_footpaths_round` uses
`saturating_add` for the `stop_arrival + transfer_time` computation. The
remaining `Tau` arithmetic in the algorithm — notably the trip
arrival/departure lookups — relies on the underlying timetable to return
sane values.

**Problem**: `Tau = usize`. A misconfigured trip schedule or a custom
`Timetable` impl returning `Tau::MAX` for a missing trip can still wrap
on downstream arithmetic outside the helper.

**Fix**: audit the algorithm for any non-saturating `Tau` arithmetic and
switch to `saturating_add` / `saturating_sub`. The footpath helper is
already covered.

---

### Issue H: Footpath transitivity is undocumented

**Severity**: Low

**Location**: `Timetable` trait documentation, `GtfsTimetable::new`.

**Paper (§3.1)**: the footpath relation F is assumed to be transitively
closed — i.e., F* = F — so that a single round of footpath relaxation
suffices. Without this, walking via A→B→C is missed when only A→B and
B→C are listed as direct footpaths.

**Current code**: nothing documents this assumption. Most well-formed
GTFS feeds happen to satisfy it because `transfers.txt` entries are
typically explicit pairs, but feeds that derive transfers from coordinate-
based proximity often do not.

**Fix (short-term)**: document the assumption on the `Timetable` trait
and on `GtfsTimetable::new`. Note the failure mode explicitly so users
know what to check for.

**Fix (medium-term)**: optionally compute the transitive closure during
`GtfsTimetable::new`, gated behind a feature flag and a maximum-walking-
distance parameter. This is what OpenTripPlanner does. The closure is
expensive on large feeds, so it must be opt-in.

---

---

## Summary Table

| Issue | Severity | Location | Description |
|-------|----------|----------|-------------|
| E | **Critical** | gtfs.rs:215–245 | GTFS adapter conflates route_id with RAPTOR route; binary search unsound |
| G | Low | lib.rs trip arithmetic | Non-saturating Tau arithmetic outside footpath helper |
| H | Low | trait docs | Footpath transitivity assumption undocumented |

A–D, F, and I are now resolved on the v0.3 branch. See *Resolved
Issues* below.

---

## Impact Assessment

**Correctness on real GTFS feeds**: with A–D and I resolved, the
remaining critical issue is E (GTFS route-pattern conflation), which
breaks routing on most non-trivial agency feeds. The hegel-based
property test in `raptor/src/proptest_support/` is green across all
three generator layers (footpaths included) and protects against
regressions on the synthetic side. E itself needs a separate harness
over `GtfsTimetable` to be exercised under property tests.

**Defence-in-depth**: Issue G is individually low-impact but worth
fixing because it makes the algorithm robust to bugs elsewhere. F was
in the same category and has now landed; it makes the output
independent of pruning correctness and simplifies testing and
debugging.

---

## Resolved Issues

The following issues were identified in earlier revisions of this
document and have since been fixed. R1–R5 were resolved by v0.2.0; A–D
were resolved on the v0.3 development branch.

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
`self.get_transfer_time(stop, p_dash)` via `saturating_add`. Wider
saturation in the trip arithmetic is still pending — see Issue G.

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
