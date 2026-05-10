# Vulture

Rust implementation of [RAPTOR](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) (Delling, Pajor, Werneck): given a public transit network, find all Pareto-optimal journeys between two stops, trading fewer transfers against earlier arrival.

**Browser demo:** <https://urschrei.github.io/vulture/>. WASM bindings (~290 KB gzipped) in [`vulture-wasm/`](vulture-wasm/), on npm as [`vulture-wasm`](https://www.npmjs.com/package/vulture-wasm).

## Quick start

The `gtfs` adapter implements `Timetable` over a parsed GTFS feed. `aux/dmrc_gtfs.zip` (Delhi Metro) is bundled.

```rust
use gtfs_structures::Gtfs;
use jiff::civil::date;
use vulture::{SecondOfDay, Timetable};
use vulture::gtfs::GtfsTimetable;

let gtfs = Gtfs::new("aux/dmrc_gtfs.zip")?;
let tt = GtfsTimetable::new(&gtfs, date(2024, 1, 15))?;

let start = tt.stop_idx("1").expect("Dilshad Garden");
let target = tt.stop_idx("44").expect("Vishwavidyalaya");

let journeys = tt
    .query()
    .from(start)
    .to(target)
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();

for j in &journeys {
    println!("{} trip(s), arrives {}", j.plan.len(), j.arrival());
}
```

Runnable: `cargo run --example gtfs-timetable -- aux/dmrc_gtfs.zip 2024-01-15 1 44`.

Each `Journey` carries `plan: Vec<(RouteIdx, StopIdx)>`, an `origin`, a `target`, and a `label`. `journey.arrival()` returns the `SecondOfDay`. For trip IDs and per-leg times, see [`Journey::with_timing`](https://docs.rs/vulture/latest/vulture/struct.Journey.html#method.with_timing).

`Gtfs::new(path)` reads from disk. `gtfs-structures` also exposes [`Gtfs::from_url`](https://docs.rs/gtfs-structures/latest/gtfs_structures/structures/gtfs/struct.Gtfs.html#method.from_url), [`Gtfs::from_url_async`](https://docs.rs/gtfs-structures/latest/gtfs_structures/structures/gtfs/struct.Gtfs.html#method.from_url_async), and [`Gtfs::from_reader`](https://docs.rs/gtfs-structures/latest/gtfs_structures/structures/gtfs/struct.Gtfs.html#method.from_reader); all yield a `Gtfs` that `GtfsTimetable::new` accepts.

```rust,ignore
let gtfs = Gtfs::from_url("https://example.org/feed/gtfs.zip")?;
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;
```

> `from_url` requires the `read-url` feature on `gtfs-structures`. vulture pulls it with `default-features = false`; for the URL paths, depend on `gtfs-structures` directly: `gtfs-structures = { version = "0.47", features = ["read-url"] }`.

## Worked examples

### Berlin VBB: station-level query

`GtfsTimetable::station_stops(parent_id)` expands a parent station into its child platforms. Passing the result to `.from(...)` / `.to(...)` runs a multi-source / multi-target search.

```rust,ignore
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;

let journeys = tt
    .query()
    .from(tt.station_stops("de:11000:900003201"))   // Hbf, ~301 platforms
    .to(tt.station_stops("de:11000:900100003"))     // Alex, ~50 platforms
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();
```

Berlin VBB (~42k stops, ~71k active trips): 385 µs.

### Helsinki HSL: augmenting sparse footpaths

HSL ships an empty `transfers.txt`. Build coordinate-derived walking edges before querying:

```rust,ignore
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?
    .with_walking_footpaths(&gtfs, 500.0, 1.4);   // 500 m at 5 km/h

let journeys = tt
    .query()
    .from(tt.stop_idx("1040601").unwrap())   // Kamppi metro
    .to(tt.stop_idx("1453601").unwrap())     // Itäkeskus metro
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();
```

`with_walking_footpaths` builds bidirectional R-tree edges and preserves existing `transfers.txt` entries.

### Paris IDFM: expensive case

Paris IDFM is the largest of the bundled benchmark feeds (~54k stops, ~146k active trips per weekday). `assert_footpaths_closed()` opts into the single-pass `O(E)` footpath relaxation when `transfers.txt` is publisher-curated.

```rust,ignore
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?
    .assert_footpaths_closed();

let journeys = tt
    .query()
    .from(tt.stop_idx("IDFM:monomodalStopPlace:45102").unwrap())   // Châtelet
    .to(tt.stop_idx("IDFM:monomodalStopPlace:44602").unwrap())     // Versailles RD
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();
```

Châtelet → Versailles RD: ~17 ms. Shorter cross-Paris queries (Châtelet → Gare du Nord, Châtelet → La Défense): under 1 ms. See [`docs/cross-city-benchmarks.md`](docs/cross-city-benchmarks.md) for the full table and methodology.

### Range queries: "leave between X and Y"

```rust,ignore
let profile = tt
    .query()
    .from(start)
    .to(target)
    .max_transfers(10)
    .depart_in_window(SecondOfDay::every(
        SecondOfDay::hms(17, 0, 0),
        SecondOfDay::hms(18, 0, 0),
        300,   // every 5 minutes
    ))
    .run();
```

Returns `Vec<RangeJourney>`: `(depart, journey)` pairs Pareto-filtered on `(later depart, fewer transfers, earlier arrival)`. Serial `.run()` is [rRAPTOR](https://docs.rs/vulture/latest/vulture/trait.Timetable.html#method.query); `.run_par()` fans the per-departure work across a Rayon thread pool.

### Server workload: many queries, multi-threaded

Allocate a `RaptorCachePool` once; each worker checks out a cache for the scope of one query.

```rust,ignore
use rayon::prelude::*;
use vulture::RaptorCachePool;

let pool = RaptorCachePool::for_timetable(&tt);

let results: Vec<_> = queries.par_iter().map(|q| {
    let mut cache = pool.checkout();
    tt.query()
        .from(q.from)
        .to(q.to)
        .max_transfers(10)
        .depart_at(q.depart)
        .run_with_cache(&mut cache)
}).collect();
```

`pool.checkout()` returns a `PooledCache` handle; the cache returns to the pool on drop.

### Runnable examples

Synthetic-network examples in `vulture/examples/`:

- `cargo run --release --example custom_label` – custom `Label` with its own `Ctx`, route-preference scoring.
- `cargo run --release --example fare_aware` – fare-aware Pareto routing via `ArrivalAndFare` and a `FareTable` context.
- `cargo run --release --example range_query` – `depart_in_window(...)`; serial rRAPTOR vs parallel naïve batch, with an equality assertion.
- `cargo run --release --example cache_reuse` – `RaptorCache` reuse with an equality assertion against one-shot results.

## Features

- **`Timetable` trait** – the network abstraction. [`GtfsTimetable`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html) covers GTFS; [implement directly](https://docs.rs/vulture/latest/vulture/trait.Timetable.html) for non-GTFS data.
- **Typestate query builder** – `tt.query().from(...).to(...).max_transfers(...).depart_at(...).run()`. Order is compile-time enforced; `.depart_at(...)` / `.depart_in_window(...)` switch the return type.
- **Multi-source / multi-target** – `.from(...)` / `.to(...)` accept a `StopIdx`, a slice, a `Vec`, or `(stop, walk_offset)` pairs. Use `station_stops(parent_id)` for any-platform queries.
- **Range queries** – `.depart_in_window(times)` returns a Pareto profile. Serial path is rRAPTOR; `.run_par()` / `.run_with_pool(&pool)` use a parallel naïve batch.
- **`Label` trait** – default [`ArrivalTime`](https://docs.rs/vulture/latest/vulture/struct.ArrivalTime.html) for single-criterion. [`vulture::labels`](https://docs.rs/vulture/latest/vulture/labels/index.html) ships [`ArrivalAndWalk`](https://docs.rs/vulture/latest/vulture/labels/struct.ArrivalAndWalk.html) (arrival vs walking time) and [`ArrivalAndFare`](https://docs.rs/vulture/latest/vulture/labels/struct.ArrivalAndFare.html) (arrival vs accumulated fare). Custom impls plug in via [`Query::with_context`](https://docs.rs/vulture/latest/vulture/struct.Query.html#method.with_context).
- **Walking footpaths from coordinates** – [`with_walking_footpaths(&gtfs, max_dist_m, speed_m_per_s)`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.with_walking_footpaths) builds bidirectional R-tree edges, preserving existing `transfers.txt`.
- **Closed-footpath fast path** – [`assert_footpaths_closed()`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.assert_footpaths_closed) opts into single-pass `O(E)` relaxation instead of Dijkstra.
- **`RaptorCache`** – reusable scratch allocations per timetable. **`RaptorCachePool`** is the `Sync` variant.
- **Per-leg timing** – [`journey.with_timing(&tt, depart, origin_walk)`](https://docs.rs/vulture/latest/vulture/struct.Journey.html#method.with_timing) attaches trip IDs and per-leg depart / arrive times to a `Journey`.
- **Wheelchair-accessibility filter** – [`require_wheelchair_accessible()`](https://docs.rs/vulture/latest/vulture/struct.Query.html#method.require_wheelchair_accessible) skips trips and stops flagged `NotAvailable` in GTFS.
- **Multi-day search** – [`GtfsTimetable::new_with_overnight_days(&gtfs, date, OvernightDays(n))`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.new_with_overnight_days) loads `n` additional service days, shifting day-`d` trips by `d × 86 400` seconds onto one time axis.
- **Per-leg shape access** – [`GtfsTimetable::shape_for_leg(&gtfs, trip_id, board, alight)`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.shape_for_leg) returns the polyline points for one leg, sliced via `shape_dist_traveled` when present.
- **Feed introspection** – [`GtfsTimetable::features()`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.features) returns a [`FeedFeatures`](https://docs.rs/vulture/latest/vulture/gtfs/struct.FeedFeatures.html) snapshot. [`FeedFeatures::suggestions()`](https://docs.rs/vulture/latest/vulture/gtfs/struct.FeedFeatures.html#method.suggestions) returns an advisory string list mapping the snapshot to relevant vulture knobs.

## When To Use What

| Goal | API |
| --- | --- |
| Single query, default routing | `tt.query()...run()` |
| Many queries, one server | allocate a `RaptorCache`, finish each chain with `.run_with_cache(&mut cache)` |
| Same as above, multi-threaded | `RaptorCachePool::for_timetable(&tt)`, `pool.checkout()` per worker |
| "Leave between X and Y" | `.depart_in_window(times).run()` (serial rRAPTOR), or `.run_par()` (multi-core wins on wide windows) |
| "Slower route with less walking" | `tt.query_with_label::<ArrivalAndWalk>()...run()` |
| "Cheapest vs. fastest journey" | `tt.query_with_label::<ArrivalAndFare>().with_context(fares)...run()` |
| Wheelchair-only routing | `tt.query()....require_wheelchair_accessible()....run()` |
| "Last train home" / overnight queries | `GtfsTimetable::new_with_overnight_days(&gtfs, date, OvernightDays(1))?` |
| Any platform of a station | `tt.station_stops(parent_id)` into `.from(...)` / `.to(...)` |
| Sparse `transfers.txt` | `.with_walking_footpaths(&gtfs, max_dist_m, speed_m_per_s)` at construction |
| Trip IDs and per-leg times in output | `journey.with_timing(&tt, depart, origin_walk)` |
| Polyline geometry per leg (for map drawing) | `tt.shape_for_leg(&gtfs, trip_id, board, alight)` |

## Perf

Single-query latency. Warm `RaptorCache`; M-series Apple Silicon; single thread. Full table and methodology in [`docs/cross-city-benchmarks.md`](docs/cross-city-benchmarks.md).

| Feed | Stops | Trips/day | Representative query | Latency |
| --- | ---: | ---: | --- | ---: |
| Delhi Metro | 262 | 5,400 | 2-trip cross-network | 22 µs |
| Helsinki HSL | 8,400 | 24,000 | Kamppi → Itäkeskus (with walking) | 1.2 ms |
| Berlin VBB | 42,000 | 71,000 | Hbf → Alex (station-to-station) | 576 µs |
| Paris IDFM | 54,000 | 146,000 | Châtelet → Versailles RD | 19 ms |

Range-query numbers (serial rRAPTOR vs parallel naïve batch): see the [bench source](vulture/benches/gtfs.rs) and the linked benchmarks page. Head-to-head against the TypeScript [`raptor-journey-planner`](https://github.com/planarnetwork/raptor): [`docs/cross-impl-comparison.md`](docs/cross-impl-comparison.md).

### Consumer build profile

The numbers above are measured with `lto = true` and `codegen-units = 1` in this workspace's `[profile.release]`. Cargo does not propagate workspace profile settings to downstream crates; a vanilla `cargo add vulture` build inherits Cargo's defaults (`lto = false`, `codegen-units = 16`, `opt-level = 3`). To match these numbers from a downstream crate, set:

```toml
[profile.release]
lto = true
codegen-units = 1
```

## Soundness

[`docs/soundness.md`](docs/soundness.md) audits the implementation issue-by-issue against the paper. The [`vulture-proptest`](vulture-proptest/) harness runs the algorithm against a brute-force reference solver on every test invocation.

## Testing

### Correctness

Three layers, all run by `cargo nextest r` on every PR:

- **Unit tests** in `vulture/src/test.rs` cover the algorithm (single-source, multi-source, range, footpath relaxation, wheelchair filtering, overnight) against hand-built `SimpleTimetable` fixtures and the bundled Delhi Metro feed.
- **Doctests** on every public type with a non-trivial contract (`Label`, `Query`, `Journey`, `RaptorCache`, `GtfsTimetable::station_stops`, etc.).
- **Property-based tests** in [`vulture-proptest`](vulture-proptest/) generate random transit networks and check the algorithm against a brute-force reference solver, via [Hegel](https://github.com/hegeldev/hegel-rust).

The proptest harness has three generator layers (Layer 1: 1–2 routes, 2–4 stops, no footpaths. Layer 2: adds 1–4 footpaths. Layer 3: 1–4 routes, up to 6 stops, optional footpaths, multi-source / multi-target queries, per-route fares). Properties:

| Property | What it checks |
| --- | --- |
| `layer{1,2,3}_matches_reference` | Algorithm output equals the reference Pareto front of `(arrival, trip_count)` at each layer. |
| `parallel_naive_matches_serial_rrap` | Serial rRAPTOR range path and parallel naïve batch return the same Pareto profile. |
| `range_query_matches_reference` | Range-query output equals a per-`τ` reference solve plus the same 3-D Pareto filter `filter_range_pareto_front` uses. |
| `fare_label_matches_per_leg_sum` | `ArrivalAndFare`'s accumulated fare equals the per-leg fare sum on every returned journey. |

Hegel persists shrunk failing seeds to `.hegel/`. See [`vulture-proptest/README.md`](vulture-proptest/README.md) for the layer / issue map.

### Perf regressions

Run `scripts/perf-smoke.sh` before finalising any feature or `Cargo.toml` profile change. It runs the Delhi `gtfs_query` group from [`vulture/benches/gtfs.rs`](vulture/benches/gtfs.rs) in criterion `--quick` mode against the last saved baseline in `target/criterion/`. Wall time: ~2 s with no source change, ~24 s after a source edit (the release profile's LTO link dominates). Criterion prints "Performance has regressed." when the median moves past noise.

The first run on a fresh checkout has no prior baseline; run twice, or pass `--save-baseline <name>` through to criterion (extra arguments are forwarded). Skip only when the change cannot affect runtime. The full criterion suite (`cargo bench --features gtfs-bench`, ~5 min) is the right thing to run before tagging a release.

For diagnosis after a regression: [`vulture/examples/profile-delhi.rs`](vulture/examples/profile-delhi.rs) loops the same three queries with no criterion-harness overhead, sized for `samply` / `cargo-flamegraph`. Build with `cargo build --profile profiling --example profile-delhi`, then `samply record -- target/profiling/examples/profile-delhi`.

## Cargo features

- `parallel` (default-on) – enables `Query::run_par` / `Query::run_with_pool` via `rayon`. `default-features = false` drops the dependency; `RaptorCachePool` stays available.
- `gtfs-bench` – the `gtfs` criterion benchmark.
- `internal` – the `manual` criterion benchmark over `manual::SimpleTimetable`.

## License

Apache-2.0
