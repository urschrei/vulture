# Property-based test harness

This module exists to detect and prevent soundness regressions in
`Timetable::raptor` by comparing its output against a brute-force reference
solver on randomly generated networks. Powered by Hegel.

## Trip-count convention

`Timetable::raptor`'s `transfers` parameter is the trip count, not the
transfer count, despite its name. A journey "board R1 to B, board R2 to D"
has 2 trips and 1 transfer; the trait's `transfers=2` admits this journey.
Similarly, `Journey::plan.len()` is the trip count.

The Pareto front compared in property tests is over `(arrival, trip_count)`.

## Empty-plan convention (`k == 0`)

`reconstruct_journey` filters out empty plans (no boarding events), so
RAPTOR can never emit a `k == 0` journey. The reference solver matches
this in two cases:

- `ps == pt` ("stay put"): early-return empty front.
- `ps != pt` walk-only: the final filter drops journeys with `k == 0`.

Either way, "no transit boarding involved" is not modelled as a journey.
This isolates the harness from RAPTOR's API choice and lets the front-
equality property focus on issues that actually involve transit.

## Generator layers

| Layer | Stops | Routes | Trips/route | Footpaths | Status on v0.3 branch |
|-------|-------|--------|-------------|-----------|-----------------------|
| 1     | 2..=4 | 1..=2  | 1..=2       | 0         | passes                |
| 2     | 2..=5 | 1..=3  | 1..=2       | 1..=4     | fails (issue I)       |
| 3     | 2..=6 | 1..=4  | 1..=3       | 0..=6     | fails (issue I)       |

Layers 2 and 3 are `#[ignore]`-flagged so `cargo nextest r` stays green
while issue I is outstanding. Run them with:

```
cargo nextest r -p raptor proptest_support --run-ignored all
```

A–D were resolved in the Phase 0 work on the v0.3 branch. The remaining
failure mode is **issue I** (journey reconstruction cannot trace through
walk legs) — see `soundness.md` and `docs/roadmap/roadmap.md` step 0.5b.
Once 0.5b lands, remove the `#[ignore]` attributes; the v0.3 deliverable
includes "the previously-ignored property tests now pass."

## Layer-to-soundness-issue map

See `soundness.md` at the repo root for the full issue catalogue.

| Soundness issue | Layer that detects it | Notes |
|-----------------|-----------------------|-------|
| A — labels not carried forward | resolved in v0.3 | Was layer-2 territory pre-fix. |
| B — no footpath relaxation from source | resolved in v0.3 | Was layer-2 territory pre-fix. |
| C — τ\* not updated in footpath stage | resolved in v0.3 | Was layer-2 (indirect) pre-fix. |
| D — no target pruning in footpath stage | resolved in v0.3 | Was layer-2 (indirect) pre-fix. |
| E — GTFS adapter route-pattern conflation | (out of scope) | Needs a separate harness over `GtfsTimetable`. |
| F — output not Pareto-filtered | masked by `raptor_front` | Hidden so the front-equality property isolates other issues. A separate test once 0.5 lands. |
| G — non-saturating Tau arithmetic | (out of scope) | Generator ranges keep `tau`, `walk_time` small enough to never trigger overflow; a targeted unit test is more appropriate. |
| H — footpath transitivity assumption | (out of scope) | Renderer transitively closes footpaths, so the harness can't observe non-closed inputs. |
| I — journey reconstruction cannot trace through walk legs | 2 | Current shrunk counterexample on the post-0.4 branch is a 1-trip walk-then-board where the reference emits `(t, 1)` and RAPTOR returns `{}`. Layer-2 stays `#[ignore]` until roadmap step 0.5b lands. |

## Reproducing a failure

Hegel persists failing seeds to `.hegel/` (gitignored). Re-running
`cargo nextest r -p raptor <test_name> --run-ignored all` deterministically
reproduces the most recent shrunk counterexample. To reproduce a specific
failure on a different machine, pass the seed explicitly:

```rust
#[hegel::test(seed = Some(0xdeadbeef))]
fn layer2_matches_reference(tc: hegel::TestCase) { /* ... */ }
```

## Internal simplification: trips on a route share leg/dwell durations

The generator constrains trips on the same route to share `leg_durations`
and `dwell_times`. This makes overtaking *structurally impossible* — every
spec the generator produces is a valid RAPTOR input.

This is stricter than the paper requires: the paper allows differing
durations as long as no trip overtakes another. We sample from a smaller
subset of the input space, not from outside the contract. Loosening this
(generating arbitrary per-trip durations and using rejection or
construction to avoid overtaking) is a reasonable future enhancement —
expect roughly 20–30 lines of changes in `route_spec` and an explicit
"no overtaking" check in `render`.

## Adding a new generator layer

To add a Layer 4 (e.g., for McRAPTOR in roadmap Phase 2):

1. Add a `LayerBounds` constant via `layerN_bounds()` in `spec.rs`.
2. Add a `#[hegel::test]` function in `mod.rs` that draws from
   `spec::network_spec(spec::layerN_bounds())` and calls `run_property`.
3. If the new layer requires multi-criterion comparison, the property
   helper (`run_property`) and `raptor_front` will need to be generalised.
   That's a Phase 2 concern.

## Wall-clock budget

Target: full property-test suite under 10 seconds on a developer laptop at
default case counts. If the budget blows out, the most likely culprit is
over-generation in the reference solver's node set; the
adjacent-timepoint-only wait-edge model already caps this. Drop Layer 3's
`test_cases` first.
