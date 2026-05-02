# Property-Based Test Harness for `raptor-rs` — Design

Author: Stephan Hügel (with Claude)
Date: 2026-05-02
Companion: `docs/proptest.md` (handoff brief), `docs/roadmap/roadmap.md` (Phase 0),
`soundness.md` (issues A–H).

## Goal

Build a property-based test harness using Hegel that compares the algorithm's
output against a brute-force reference solver on randomly generated networks.
This is the single most valuable test in the repository — it must catch every
existing soundness issue (A, B, C, D, F, G; E is GTFS-only and out of scope)
and prevent regressions as Phase 0 fixes land.

A failing test on the unmodified v0.2.0 baseline is the deliverable. The
soundness fixes themselves are a separate piece of work that this harness will
guide and validate.

## Property under test

For any well-formed timetable `T`, source `pₛ`, target `pₜ`, departure time
`τ`, and max-trips bound `k`:

> The Pareto front of `(arrival, trips)` pairs returned by
> `T.raptor(k, τ, pₛ, pₜ)` equals the Pareto front computed by a time-expanded
> Dijkstra reference solver on the same inputs.

Equality is over Pareto fronts, not journeys. Two implementations may
legitimately return different journey witnesses for the same `(arrival, trips)`
point. The front is the invariant; the witnesses are not.

### Trip-count convention

`Timetable::raptor`'s `transfers` parameter is the trip count, not the
transfer count, despite its name. A journey "board R1 to B, board R2 to D"
has 2 trips and 1 transfer; the trait's `transfers=2` admits this journey.
Similarly, `Journey::plan.len()` is the trip count. Both this harness and the
reference solver use trip counts everywhere. The Pareto front is over
`(arrival, trip_count)`.

This is documented as a banner comment at the top of `proptest_support/mod.rs`.

## Module layout

```
raptor/src/proptest_support/
    mod.rs        # module root + #[hegel::test] functions for each layer
    spec.rs       # NetworkSpec types, per-layer generators, the renderer
    reference.rs  # time-expanded multi-criterion Dijkstra
    README.md     # convention notes, layer-to-issue mapping, repro guide
```

Wired into `raptor/src/lib.rs`:

```rust
#[cfg(test)]
mod proptest_support;
```

The module is internal — nothing escapes the `cfg(test)` boundary, the public
API is unchanged.

The harness lives in its own directory rather than inline alongside
`raptor/src/test.rs` because it has substantial non-test code (NetworkSpec,
generators, renderer, ~200-line reference solver) that would crowd out the
existing hand-written scenarios. Hegel's general advice about keeping property
tests next to existing tests is about not creating throw-away `test_hegel.rs`
files — here the harness is a coherent support module, so the directory is
justified.

## Dependency

Add to `raptor/Cargo.toml`:

```toml
[dev-dependencies]
hegel = { git = "https://github.com/hegeldev/hegel-rust" }
```

No `rand` feature — the algorithm under test is deterministic; the only
randomness is in test-case generation, which Hegel handles directly. Add
`.hegel/` to `.gitignore`.

## Data model

```rust
// raptor/src/proptest_support/spec.rs

pub struct NetworkSpec {
    pub n_stops: u8,                    // 2..=6
    pub routes: Vec<RouteSpec>,         // 1..=4
    pub footpaths: Vec<FootpathSpec>,   // 0..=6 sparse; renderer transitively closes
    pub query: QuerySpec,
}

pub struct RouteSpec {
    pub stop_sequence: Vec<u8>,         // 2..=4 distinct stops, all < n_stops
    pub trips: Vec<TripSpec>,           // 1..=3
}

pub struct TripSpec {
    pub first_dep: u16,                 // 0..=400
    pub leg_durations: Vec<u16>,        // length = stop_sequence.len() - 1
    pub dwell_times: Vec<u16>,          // length = stop_sequence.len()
}

pub struct FootpathSpec {
    pub from: u8,                       // < n_stops
    pub to: u8,                         // < n_stops, != from
    pub walk_time: u16,                 // 1..=300
}

pub struct QuerySpec {
    pub ps: u8,                         // < n_stops
    pub pt: u8,                         // < n_stops; may equal ps
    pub tau: u16,                       // 0..=500
    pub max_transfers: u8,              // 1..=5 (i.e. max trip count)
}
```

The use of `u8`/`u16` (rather than the `usize` types in `raptor/src/test.rs`
builders) keeps the input space small and shrinks tightly. The renderer casts
to `usize` at the `SimpleTimetable<u8, u8, u16>` boundary; `Tau = usize` in the
algorithm.

## Generator strategy: hybrid (generate-and-fixup + construct-valid)

Some invariants are enforced at generation time, some in the renderer. The
choice depends on which produces cleaner shrinking and avoids silently
dropping inputs.

### Enforced at generation time

- **Stops within a route are distinct:**
  `vecs(integers::<u8>().min_value(0).max_value(n_stops - 1)).unique().min_size(2).max_size(4)`.
- **Time monotonicity within a trip:** generate `leg_durations` and
  `dwell_times` as flat vectors of small non-negative integers; the renderer
  prefix-sums them into `(arr, dep)` pairs, so monotonicity is structural.
- **No overtaking across trips:** trips on the same route share
  `leg_durations` and `dwell_times`. Only `first_dep` varies per trip.
  Overtaking is structurally impossible. This is *stricter* than the paper
  (which allows differing leg durations as long as no trip overtakes another),
  but every spec the generator produces is a valid RAPTOR input — we sample
  from a smaller subset of the input space, not from outside the contract.
  The README documents the choice so a future contributor can loosen it.
- **Footpath endpoints differ and are < n_stops:** dependent generation —
  draw `from`, then draw `to` from the complement.

### Enforced at render time

- **Footpath transitive closure:** Floyd–Warshall on min-plus over the
  `n_stops × n_stops` cost matrix, seeded from the sparse `footpaths` list.
- **Same-route stop sequence padding:** none needed — the generator already
  guarantees `unique()` and `min_size(2)`.

### Renderer contract

`pub fn render(spec: &NetworkSpec) -> SimpleTimetable<u8, u8, u16>` is total
and panic-free on every spec the generators can produce, and deterministic
(same spec → byte-identical timetable). It does not silently drop or
normalize anything: if a generator produces an out-of-contract spec, the
renderer panics in debug, surfacing the generator bug rather than hiding it.

## Per-layer generators

Each layer is its own composite generator, called from a layer-specific
`#[hegel::test]`. Failures shrink to the smallest layer that exhibits the bug.

| Layer | Stops | Routes | Trips/route | Footpaths | Targets |
|-------|-------|--------|-------------|-----------|---------|
| 1     | 2..=4 | 1..=2  | 1..=2       | 0         | regression-only; should pass on v0.2.0 |
| 2     | 2..=5 | 1..=3  | 1..=2       | 1..=4     | issues A, B, C, D (F masked by harness filter — see "Comparison helper") |
| 3     | 2..=6 | 1..=4  | 1..=3       | 0..=6     | full-network catch-all |

Layer 1 is the regime the existing hand-written tests cover; it should be
green on the v0.2.0 baseline. Layers 2 and 3 should fail on baseline — that
failure is the deliverable.

## Reference solver

`raptor/src/proptest_support/reference.rs` — a time-expanded multi-criterion
Dijkstra. Optimise nothing; if we ever debug this, we've gone wrong somewhere.

```rust
pub fn reference_solve(
    spec: &NetworkSpec,
    ps: u8, pt: u8, tau: u16, max_trips: u8,
) -> BTreeSet<(u16, u8)>;  // Pareto front of (arrival, trips)
```

### Node set

Built explicitly, up front, as `BTreeMap<Stop, BTreeSet<u16>>`. Per stop, the
union of:

- every trip-arrival time at that stop,
- every trip-departure time at that stop,
- `tau` itself, if the stop is `ps`,
- every footpath-arrival time reachable from any of the above by walking once
  (fixed-point iteration; bounded because times are `u16`-discretised and the
  spec is small).

### Edges

- **Ride** `(stop_i, dep_i) → (stop_j, arr_j)`, cost `(arr_j, +0)` — for each
  consecutive pair on each trip.
- **Board** `(stop, t) → (stop, dep)` for `t ≤ dep` where some trip on a route
  through `stop` departs at `dep`, cost `(dep, +1)`.
- **Walk** `(a, t) → (b, t + walk_time)` for each footpath, cost
  `(t + walk_time, +0)`. Saturating add — drop the edge if it overflows `u16`.
- **Wait** `(stop, t) → (stop, t')` between *adjacent* timepoints at the same
  stop only, cost `(t', +0)`. The proptest brief explicitly calls out
  all-pairs wait edges as the most likely budget killer; adjacent-only is
  sufficient and asymptotically tight.

### Search

2D Dijkstra with `BinaryHeap<Reverse<(arrival, trips, node)>>`. Per node,
maintain a `Vec<(u16, u8)>` Pareto front. A candidate at a node is processed
iff not dominated by anything currently in that node's front; on insertion,
drop newly-dominated points.

### Edge cases

- `ps == pt` → return `{(tau, 0)}`.
- Network disconnected from the source → return `{}`.

### Output

Union of fronts at every `(pt, *)` node, filtered to `trips ≤ max_trips`,
returned as a `BTreeSet<(u16, u8)>` sorted by trips ascending.

The whole file should be ~200 lines using only `std`. It must be obviously
correct on inspection.

## Property tests

`raptor/src/proptest_support/mod.rs` holds three `#[hegel::test]` functions
plus a comparison helper.

### Comparison helper

```rust
fn raptor_front(journeys: &[Journey<u8, u8>]) -> BTreeSet<(u16, u8)> {
    let mut points: Vec<(u16, u8)> = journeys.iter()
        .map(|j| (j.arrival as u16, j.plan.len() as u8))
        .collect();
    points.sort_by_key(|&(_, k)| k);
    let mut best = u16::MAX;
    points.retain(|&(arr, _)| if arr < best { best = arr; true } else { false });
    points.into_iter().collect()
}
```

This applies the output-side Pareto filter that the algorithm itself ought to
be doing (issue F). Filtering on the harness side means the comparison still
passes once F is fixed; until then, the filter masks issue F so the front-
equality assertion isolates issues A/B/C/D from F. F gets a separate, more
targeted check (see "Optional follow-up" below).

### Test bodies

```rust
#[hegel::test]
fn layer1_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::layer1_spec());
    let timetable = spec::render(&spec);
    let ours = timetable.raptor(
        spec.query.max_transfers as usize,
        spec.query.tau as usize,
        spec.query.ps,
        spec.query.pt,
    );
    let theirs = reference::reference_solve(
        &spec, spec.query.ps, spec.query.pt,
        spec.query.tau, spec.query.max_transfers,
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

#[ignore = "expected to fail on v0.2.0; targets soundness issues A, B, C, D"]
#[hegel::test]
fn layer2_matches_reference(tc: hegel::TestCase) { /* same body, layer2_spec */ }

#[ignore = "expected to fail on v0.2.0; full-network coverage"]
#[hegel::test(test_cases = 500)]
fn layer3_matches_reference(tc: hegel::TestCase) { /* same body, layer3_spec */ }
```

`tc.note` only fires on the final shrunk counterexample, which is exactly the
right behaviour — clean output until the bug is found, full dump on failure.

### Why `#[ignore]`

The proptest brief is explicit that a failing test on v0.2.0 is the
deliverable. `#[ignore]` keeps `cargo nextest r` green for incremental commits
under the project's jj workflow, while preserving the failing-on-baseline
deliverable: running `cargo nextest r --run-ignored all` flips the harness
into validation mode. Removing the `#[ignore]` attributes is a natural first
commit of v0.3 once Phase 0 fixes land. This also matches the proptest brief's
discussion of "expected failure" as a clearer signal than just letting the
test go red.

## Verification protocol

After implementation:

1. `cargo nextest r -p raptor proptest_support` — must be green (only Layer 1
   runs).
2. `cargo nextest r --run-ignored all -p raptor proptest_support` — Layer 1
   green; Layer 2 must fail with a shrunk counterexample; Layer 3 must fail.
3. The Layer 2 counterexample should shrink to something readable — ideally a
   3-stop, 1-route, 1-footpath spec where the optimal journey requires walking
   from the source.
4. Total wall-clock for the full suite (including ignored) under 10 seconds on
   a developer laptop.

If Layer 2 *passes* on baseline, the harness has a bug — investigate before
declaring victory.

## Wall-clock budget

Spec target is 10 seconds for the full suite. Layer 1 (no footpaths) is
fastest; defaults to `test_cases = 100`. Layer 2 defaults to 100. Layer 3 uses
500 for fuller coverage. If 10s blows out, drop Layer 3 to 200 first; the
likely culprit is the reference solver's wait-edge generation, which the
adjacent-only cap should keep manageable.

## README content

`raptor/src/proptest_support/README.md` covers:

- Trip-vs-transfer convention (one paragraph, with a worked example).
- Layer table: which generator targets which soundness issue.
- How to reproduce a failure from Hegel's failure database / persisted seed.
- How to extend with a new generator layer (e.g., for McRAPTOR in Phase 2).
- The "trips share leg/dwell durations" simplification and how to relax it.

## Out of scope

Per the proptest brief:

- Multi-criterion labels (Phase 2 of the roadmap).
- Range queries (rRAPTOR).
- Realtime overlays.
- The GTFS adapter — soundness issue E gets its own harness eventually, but
  isolating algorithm bugs from adapter bugs is the priority for this work.
- Performance benchmarks.
- Issue G (saturating arithmetic) — can be triggered by extreme `tau` /
  `walk_time` combinations that the current generator ranges (`tau ≤ 500`,
  `walk_time ≤ 300`) won't reach. Worth a targeted unit test rather than
  property coverage.
- Issue H (footpath transitivity documentation) — the renderer transitively
  closes footpaths, so the harness can't directly observe non-closed inputs.
  Also out of scope.

## Optional follow-up (not part of this deliverable)

Once Phase 0 is in flight, add a fourth test that asserts RAPTOR's output is
already Pareto-filtered (i.e., calling `raptor_front` on the algorithm's raw
output should be a no-op). This isolates issue F from the front-equality
property. Cheap to add; can land in v0.3 alongside the issue F fix.

## Success criteria

When the work is done:

- `cargo nextest r --run-ignored all -p raptor proptest_support` runs the full
  property-test suite in under 10 seconds.
- Layer 1 passes on v0.2.0 baseline.
- Layers 2 and 3 fail on v0.2.0 baseline with shrunk, readable counterexamples
  — at minimum due to soundness issues A and B.
- The README is enough that someone unfamiliar with the project can run the
  harness, interpret a failure, and add a new generator layer.
- No `unwrap()` or `panic!()` in `reference.rs` on any spec the generators can
  produce.
- The public API of the `raptor` crate is unchanged — the harness is entirely
  internal under `#[cfg(test)]`.
