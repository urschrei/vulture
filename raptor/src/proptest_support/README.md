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

## ps == pt convention

When the source and target are the same stop, the algorithm's
`reconstruct_journey` filters out empty plans, so RAPTOR returns `[]` rather
than `[(tau, 0)]`. The reference solver matches this behaviour and also
returns the empty front when `ps == pt`. "Stay put at the source" is not
modelled as a journey.

## Generator layers

| Layer | Stops | Routes | Trips/route | Footpaths | Status on v0.2.0 |
|-------|-------|--------|-------------|-----------|-------------------|
| 1     | 2..=4 | 1..=2  | 1..=2       | 0         | passes            |
| 2     | 2..=5 | 1..=3  | 1..=2       | 1..=4     | fails (issues A, B, C, D) |
| 3     | 2..=6 | 1..=4  | 1..=3       | 0..=6     | fails             |

Layers 2 and 3 are `#[ignore]`-flagged so `cargo nextest r` stays green on
the v0.2.0 baseline. Run them with:

```
cargo nextest r -p raptor proptest_support --run-ignored all
```

When Phase 0 of `docs/roadmap/roadmap.md` lands, remove the `#[ignore]`
attributes — the deliverable for v0.3 includes "the previously-ignored
property tests now pass."

## Layer-to-soundness-issue map

See `soundness.md` at the repo root for the full issue catalogue.

| Soundness issue | Layer that detects it | Notes |
|-----------------|-----------------------|-------|
| A — labels not carried forward | 2 | Triggered by any journey reaching a stop in round k−1 then walking in round k. |
| B — no footpath relaxation from source | 2 | Triggered when optimal journey starts with a walk. The current shrunk counterexample on v0.2.0 isolates exactly this. |
| C — τ\* not updated in footpath stage | 2 (indirect) | Manifests as inflated arrivals via leaky pruning; most easily seen on multi-round journeys with footpaths. |
| D — no target pruning in footpath stage | 2 (indirect) | Wastes work but is correctness-neutral if F is also fixed; the harness may not directly demonstrate D in isolation. |
| E — GTFS adapter route-pattern conflation | (out of scope) | Needs a separate harness over `GtfsTimetable`. |
| F — output not Pareto-filtered | masked by `raptor_front` | Intentionally hidden in this harness so the front-equality property isolates A/B/C/D. A separate test once Phase 0 lands. |
| G — non-saturating Tau arithmetic | (out of scope) | Generator ranges keep `tau`, `walk_time` small enough to never trigger overflow; a targeted unit test is more appropriate. |
| H — footpath transitivity assumption | (out of scope) | Renderer transitively closes footpaths, so the harness can't observe non-closed inputs. |

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
