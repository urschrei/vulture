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
| 2     | 2..=5 | 1..=3  | 1..=2       | 1..=4     | passes                |
| 3     | 2..=6 | 1..=4  | 1..=3       | 0..=6     | passes                |

All three layers are part of the default `cargo nextest r` run. They
were `#[ignore]`-flagged on v0.2.0 because issues A–D and I made the
front-equality property fail; resolving Phase 0 (0.1–0.4 + 0.5b) on
the v0.3 branch turned them green and the flags came off.

## Layer-to-soundness-issue map

See `soundness.md` at the repo root for the full issue catalogue.

| Soundness issue | Layer that detects it | Notes |
|-----------------|-----------------------|-------|
| A — labels not carried forward | resolved in v0.3 | Was layer-2 territory pre-fix. |
| B — no footpath relaxation from source | resolved in v0.3 | Was layer-2 territory pre-fix. |
| C — τ\* not updated in footpath stage | resolved in v0.3 | Was layer-2 (indirect) pre-fix. |
| D — no target pruning in footpath stage | resolved in v0.3 | Was layer-2 (indirect) pre-fix. |
| E — GTFS adapter route-pattern conflation | (out of scope) | Needs a separate harness over `GtfsTimetable`. |
| F — output not Pareto-filtered | resolved in v0.3 | `Timetable::raptor` now sorts and Pareto-filters the output. `raptor_front` is now redundant on the algorithm side and could be inlined to a `.iter().map(...).collect::<BTreeSet<_>>()`; left in place as belt-and-braces. |
| G — non-saturating Tau arithmetic | (out of scope) | Generator ranges keep `tau`, `walk_time` small enough to never trigger overflow; a targeted unit test is more appropriate. |
| H — footpath transitivity assumption | (out of scope) | Renderer transitively closes footpaths, so the harness can't observe non-closed inputs. |
| I — journey reconstruction cannot trace through walk legs | resolved in v0.3 | Was the post-0.4 layer-2 blocker. Boarding tree now records walk legs alongside route boardings; reconstruction chains through walks within a round. |

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
2. Add a `#[hegel::test]` function in `src/lib.rs` that draws from
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
