# RAPTOR Implementation Soundness Analysis

This document analyzes the soundness of the RAPTOR implementation in
`raptor/src/lib.rs` against the original paper:

> Delling, D., Pajor, T., & Werneck, R. F. (2015). *Round-Based Public Transit
> Routing.* Transportation Science 49(3): 591–604.
> DOI: [10.1287/trsc.2014.0534](https://doi.org/10.1287/trsc.2014.0534).

An earlier conference version appeared as Delling, Pajor & Werneck (2012),
*Round-Based Public Transit Routing*, ALENEX. Algorithm line numbers below
refer to Algorithm 1 in the journal version.

This revision reflects the state of the code as of **v0.2.0**. Prior versions
of this document described several issues that have since been fixed; those
are summarized in the *Resolved Issues* section at the end for historical
reference.

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

### Issue A: Round labels are not carried forward (Stage 1 missing)

**Severity**: Critical

**Location**: `raptor/src/lib.rs`, throughout the round loop (lines
202–271).

**Paper (Algorithm 1, line 5)**:

> for each stop p, set τₖ(p) ← τₖ₋₁(p)

**Current code**: labels are stored in
`best_arrival_per_k: BTreeMap<(K, Stop), Tau>` and entries are only ever
inserted when a stop is improved in the current round. There is no
explicit copy-forward of round k−1 labels into round k.

**Problem**: when round k reads `best_arrival_per_k.get(&(k, stop))`
during footpath relaxation (line 256), the lookup returns `None` for any
stop that was reached in an earlier round but not re-improved this round.
The walking-arrival value defaults to `Tau::MAX`, so footpaths cannot
relax from stops touched only by previous rounds.

**Concrete failure mode**: a journey whose final leg is "walk from a stop
reached in round k via routing to a target reached in round k via
walking" is found, but a journey whose final leg is "walk from a stop
reached in round k−1 via routing" is not, because round k never sees the
τₖ₋₁ value at the walking origin.

**Fix**: at the start of each round, seed round k's labels from round
k−1. With the current sparse representation:

```rust
let prev: Vec<_> = best_arrival_per_k
    .range((k - 1, Self::Stop::MIN)..(k, Self::Stop::MIN))
    .map(|(&(_, s), &t)| (s, t))
    .collect();
for (s, t) in prev {
    best_arrival_per_k.entry((k, s)).or_insert(t);
}
```

A cleaner fix is to switch the underlying representation to
`Vec<HashMap<Stop, Tau>>` indexed by k, making carry-forward a single
`labels[k] = labels[k-1].clone()`.

---

### Issue B: Footpath relaxation from the source is missing in round 1

**Severity**: Critical

**Location**: `raptor/src/lib.rs:193–202`.

**Paper**: round 0 sets τ₀(pₛ) = τ and additionally relaxes footpaths
from pₛ so that walk-reachable neighbours of pₛ have finite τ₀ before
round 1 begins.

**Current code**: only `best_arrival_per_k.insert((0, ps), tau)` is
performed at initialization. The footpath stage (lines 246–264) runs
inside the per-round loop and reads `best_arrival_per_k.get(&(k, stop))`
for `stop ∈ marked_stops`. At the start of round 1, `marked_stops`
contains only `ps`, but the lookup is for round k = 1 and `(1, ps)` is
not yet set.

**Problem**: a journey whose first leg is "walk from pₛ to a neighbour,
then board" is not discovered. This affects any feed where the optimal
journey requires a short walk to a higher-frequency stop.

**Fix**: extract the footpath stage into a helper, run it once during
initialization seeded from round 0, and continue running it once per
round. After fixing Issue A, the carry-forward will propagate the
round-0 footpath labels into round 1 automatically.

```rust
fn relax_footpaths<...>(&self, k: K, marked: &mut BTreeSet<Self::Stop>, ...) { ... }

// In raptor():
best_arrival_per_k.insert((0, ps), tau);
self.relax_footpaths(0, &mut marked_stops, ...);

for k in 1..=transfers {
    // existing route scan + footpath relaxation
}
```

---

### Issue C: τ\* (`best_arrival`) not updated in the footpath stage

**Severity**: Medium

**Location**: `raptor/src/lib.rs:246–264`.

**Paper**: τ\*(p) tracks the earliest known arrival at p across all
rounds and is the basis for both local pruning ("don't improve a label
that doesn't beat τ\*(p)") and target pruning ("don't bother if it
doesn't beat τ\*(pₜ)"). Updates to τₖ from any stage — including
footpaths — should be reflected in τ\*.

**Current code**: the route-scanning stage updates both
`best_arrival_per_k` and `best_arrival` (the τ\* table) at lines 227–229.
The footpath stage at line 261 updates only `best_arrival_per_k`.

**Problem**: in subsequent rounds, local and target pruning compare
candidate arrivals against a stale τ\* that ignores walk-reached stops.
Pruning is therefore weaker than it should be — the algorithm does
redundant work and admits dominated journeys into the output that should
have been pruned at recording time. Combined with the missing output-
side Pareto filter (Issue F), this means dominated journeys can leak to
the user.

**Fix**: after the footpath update, also update τ\*:

```rust
best_arrival_per_k.insert((k, p_dash), tau);
best_arrival
    .entry(p_dash)
    .and_modify(|v| *v = (*v).min(tau))
    .or_insert(tau);
```

---

### Issue D: No target pruning in the footpath stage

**Severity**: Medium

**Location**: `raptor/src/lib.rs:262`.

**Paper (Algorithm 1, lines 18–21)**: stops are only marked when their
new arrival improves on both τ\*(pᵢ) and τ\*(pₜ). This applies to both
route-scanning and footpath stages.

**Current code**: the footpath stage marks every walk-reachable stop
unconditionally:

```rust
more_marked_stops.push(p_dash);
```

**Problem**: stops with arrival times worse than τ\*(pₜ) are still marked
and will cause unnecessary route scans in the next round. Correctness is
not directly affected — those scans cannot improve τ\*(pₜ) and won't
emit dominated journeys *if* Issues C and F are also fixed — but the
algorithm wastes work proportional to the size of the footpath graph
beyond the target.

**Fix**:

```rust
if tau < *best_arrival.get(&pt).unwrap_or(&Tau::MAX) {
    more_marked_stops.push(p_dash);
}
```

---

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

### Issue F: Output is not Pareto-filtered

**Severity**: Medium

**Location**: `raptor/src/lib.rs:273–283`, `reconstruct_journey` at
lines 73–120.

**Paper**: the output is the Pareto front over (arrival, transfers).

**Current code**: `reconstruct_journey` produces one plan per
`k ∈ 1..=transfers` and returns them all. `Timetable::raptor` wraps each
plan with the corresponding arrival time and emits the lot.

**Problem**: a 2-transfer journey arriving at the same time as a
1-transfer journey is dominated and should be dropped. Local and target
pruning *should* prevent dominated plans from being recorded — but with
Issue C unfixed, pruning is leaky and dominated plans do reach the
output. Even with C fixed, an explicit output-side filter is cheap and
provides a defence in depth.

**Fix**: after collecting the journeys, sort by transfer count ascending
and drop any whose arrival is not strictly less than the best arrival
seen so far:

```rust
let mut journeys: Vec<_> = plans.into_iter().map(...).collect();
journeys.sort_by_key(|j| j.plan.len());
let mut best = Tau::MAX;
journeys.retain(|j| {
    if j.arrival < best { best = j.arrival; true } else { false }
});
journeys
```

This also makes the output ordering deterministic and easier to test.

---

### Issue G: Saturating arithmetic on `Tau`

**Severity**: Low

**Location**: `raptor/src/lib.rs:259`.

**Current code**:

```rust
+ self.get_transfer_time(stop, p_dash),
```

**Problem**: `Tau = usize`. With a misconfigured transfer time near
`Tau::MAX` (or, more realistically, a footpath-via-footpath chain after
Issue B is fixed), this addition can wrap.

**Fix**: use `saturating_add` everywhere `Tau` arithmetic happens. Cheap
and removes a class of bugs that would otherwise have to be debugged
from a stack trace.

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

## Summary Table

| Issue | Severity | Location | Description |
|-------|----------|----------|-------------|
| A | **Critical** | lib.rs round loop | Round labels not carried forward (τₖ ← τₖ₋₁) |
| B | **Critical** | lib.rs:193–202 | Footpath relaxation from source missing in round 1 |
| C | Medium | lib.rs:246–264 | τ\* not updated in footpath stage |
| D | Medium | lib.rs:262 | No target pruning in footpath stage |
| E | **Critical** | gtfs.rs:215–245 | GTFS adapter conflates route_id with RAPTOR route; binary search unsound |
| F | Medium | lib.rs:273–283 | Output not Pareto-filtered |
| G | Low | lib.rs:259 | Non-saturating Tau arithmetic |
| H | Low | trait docs | Footpath transitivity assumption undocumented |

---

## Impact Assessment

**Correctness on real GTFS feeds**: the combination of Issues A, B, and E
means the algorithm produces wrong answers on realistic inputs. Issue E
alone is sufficient to break routing on most non-trivial agency feeds —
practically any feed where one `route_id` covers more than one service
pattern, which is the norm.

**Synthetic test networks** (the kind covered by `raptor/src/test.rs`)
sidestep all three. Each test constructs a `SimpleTimetable` whose
routes have a single stop sequence and whose footpaths are trivial,
hiding A, B, and E. The existing test suite is therefore green despite
the implementation being unsound on real inputs — a strong argument for
adding a property-based test that compares output against a brute-force
reference solver on randomly-generated networks.

**Performance**: Issues C and D weaken pruning. The algorithm still
terminates and produces *a* result, but does redundant work in
proportion to the size of the footpath graph and the breadth of the
network. Not a correctness problem in isolation, but combined with F it
allows dominated journeys into the output.

**Defence-in-depth**: Issues F and G are individually low-impact but
worth fixing because they make the algorithm robust to bugs in the rest
of the implementation. F is especially valuable because it makes the
output independent of pruning correctness, simplifying both testing and
debugging.

---

## Resolved Issues

The following issues were identified in earlier revisions of this
document and have since been fixed (as of v0.2.0). They are retained
here for historical reference.

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

**Now**: the footpath update at `raptor/src/lib.rs:259` adds
`self.get_transfer_time(stop, p_dash)`. (Saturation is still missing —
see Issue G.)

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
