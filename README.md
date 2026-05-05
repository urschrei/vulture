# Vulture

Rust implementation of [RAPTOR](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) (Delling, Pajor, Werneck): given a transit network, find all Pareto-optimal journeys between two stops, trading fewer transfers against earlier arrival.

## Quick start

The `gtfs` module wraps a parsed GTFS feed and implements the `Timetable` trait. The repo bundles `aux/dmrc_gtfs.zip` (Delhi Metro) so you can run the example without downloading anything:

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

Runnable as `cargo run --example gtfs-timetable -- aux/dmrc_gtfs.zip 2024-01-15 1 44`.

A returned `Journey` has `plan: Vec<(RouteIdx, StopIdx)>` ("take this route, get off at this stop"), an `origin` and `target` (relevant when the query supplies multiple of either), and a `label` carrying the criterion under optimisation. `journey.arrival()` reads it as a `SecondOfDay`. See [`Journey`](https://docs.rs/vulture/latest/vulture/struct.Journey.html) for the full contract; for trip IDs and per-leg depart/arrive times see [`Journey::with_timing`](https://docs.rs/vulture/latest/vulture/struct.Journey.html#method.with_timing).

## Worked examples

### Berlin VBB: station-level query

A station like Berlin Hbf has hundreds of platforms; you don't usually care which one. `GtfsTimetable::station_stops(parent_id)` expands a parent station to its child platforms and the multi-source/multi-target search picks the best combination:

```rust,ignore
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;

let journeys = tt
    .query()
    .from(tt.station_stops("de:11000:900003201"))   // Hbf – ~301 platforms
    .to(tt.station_stops("de:11000:900100003"))     // Alex – ~50 platforms
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();
```

Returns a direct S-Bahn boarding at 09:01, arriving Alex at 09:07 – across a feed of ~42k stops, ~71k active trips, in ~385 µs.

### Helsinki HSL: augmenting sparse footpaths

HSL ships an empty `transfers.txt`, so out of the box no walking transfers exist between physical stops. Build coordinate-derived walking edges before querying:

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

`with_walking_footpaths` uses an R-tree over an equirectangular projection (~0.5% accurate at city scale) and preserves any pre-existing `transfers.txt` entries.

### Paris IDFM: the expensive case

Châtelet → Versailles Rive Droite crosses the IDFM region: the expanded RER feed has ~54k stops, ~146k active trips per weekday, and the optimal journey involves the RER C from Saint-Michel-Notre-Dame:

```rust,ignore
let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?
    .assert_footpaths_closed();   // IDFM's transfers.txt is publisher-curated

let journeys = tt
    .query()
    .from(tt.stop_idx("IDFM:monomodalStopPlace:45102").unwrap())   // Châtelet
    .to(tt.stop_idx("IDFM:monomodalStopPlace:44602").unwrap())     // Versailles RD
    .max_transfers(10)
    .depart_at(SecondOfDay::hms(9, 0, 0))
    .run();
```

Returns in ~17 ms. Smaller cross-Paris queries (Châtelet → Gare du Nord, Châtelet → La Défense) come back in under 1 ms. See [`docs/cross-city-benchmarks.md`](docs/cross-city-benchmarks.md) for the full numbers and methodology.

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

Returns `Vec<RangeJourney>` – each entry is a `(depart, journey)` pair, Pareto-filtered on `(later depart, fewer transfers, earlier arrival)`. Serial uses [rRAPTOR](https://docs.rs/vulture/latest/vulture/trait.Timetable.html#method.query) (single reverse-chronological scan reusing labels across departures); `.run_par()` spreads the per-departure work across a Rayon thread pool.

### Server workload: many queries, multi-threaded

Allocate a `RaptorCachePool` once and have each thread check out a cache:

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

The pool's `checkout()` returns an RAII guard that returns the cache on drop; same pool serves any number of threads with no per-thread bookkeeping.

## Features

- **`Timetable` trait** – describes a transit network. Use the bundled [`GtfsTimetable`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html) for any GTFS feed, or [implement directly](https://docs.rs/vulture/latest/vulture/trait.Timetable.html) for non-GTFS data.
- **Typestate query builder** – `tt.query().from(...).to(...).max_transfers(...).depart_at(...).run()`. Compile-time enforced order; `.depart_at(...)` and `.depart_in_window(...)` switch the return type.
- **Multi-source / multi-target** – `.from(...)` and `.to(...)` accept a `StopIdx`, a slice, a `Vec`, or `(stop, walk_offset)` pairs. Pair with `station_stops(parent_id)` for "any platform of this station" queries.
- **Range queries** – `.depart_in_window(times)` returns a Pareto profile; serial path uses rRAPTOR, `.run_par()` and `.run_with_pool(&pool)` use a parallel naïve batch.
- **`Label` trait** – single-criterion [`ArrivalTime`](https://docs.rs/vulture/latest/vulture/struct.ArrivalTime.html) is the default. [`vulture::labels`](https://docs.rs/vulture/latest/vulture/labels/index.html) ships [`ArrivalAndWalk`](https://docs.rs/vulture/latest/vulture/labels/struct.ArrivalAndWalk.html) for trade-off queries (slower routes with less walking). Implement [`Label`](https://docs.rs/vulture/latest/vulture/trait.Label.html) for fares, accessibility, etc.
- **Walking footpaths from coordinates** – [`with_walking_footpaths(&gtfs, max_dist_m, speed_m_per_s)`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.with_walking_footpaths) builds bidirectional R-tree walking edges, preserving any existing `transfers.txt`.
- **Closed-footpath fast path** – if your `transfers.txt` is the entire intended relation, [`assert_footpaths_closed()`](https://docs.rs/vulture/latest/vulture/gtfs/struct.GtfsTimetable.html#method.assert_footpaths_closed) opts into a single-pass O(E) relaxation instead of Dijkstra.
- **`RaptorCache`** – reusable scratch allocations across queries against one timetable. **`RaptorCachePool`** is the `Sync` variant for thread pools.
- **Per-leg timing** – [`journey.with_timing(&tt, depart, origin_walk)`](https://docs.rs/vulture/latest/vulture/struct.Journey.html#method.with_timing) reconstructs trip IDs and per-leg depart/arrive times. `Journey.plan` alone is just topology.

## When To Use What

| Goal | API |
| --- | --- |
| Single query, default routing | `tt.query()...run()` |
| Many queries, one server | allocate a `RaptorCache`, finish each chain with `.run_with_cache(&mut cache)` |
| Same as above, multi-threaded | `RaptorCachePool::for_timetable(&tt)`, `pool.checkout()` per worker |
| "Leave between X and Y" | `.depart_in_window(times).run()` (serial rRAPTOR), or `.run_par()` (multi-core wins on wide windows) |
| "Slower route with less walking" | `tt.query_with_label::<ArrivalAndWalk>()...run()` |
| Any platform of a station | `tt.station_stops(parent_id)` into `.from(...)` / `.to(...)` |
| Sparse `transfers.txt` | `.with_walking_footpaths(&gtfs, max_dist_m, speed_m_per_s)` at construction |
| Trip IDs and per-leg times in output | `journey.with_timing(&tt, depart, origin_walk)` |

## Perf

Single-query latency, warm `RaptorCache`, M-series Apple Silicon, single thread. Full numbers (load times, multi-trip queries, methodology) in [`docs/cross-city-benchmarks.md`](docs/cross-city-benchmarks.md):

| Feed | Stops | Trips/day | Representative query | Latency |
| --- | ---: | ---: | --- | ---: |
| Delhi Metro | 262 | 5,400 | 2-trip cross-network | 22 µs |
| Helsinki HSL | 8,400 | 24,000 | Kamppi → Itäkeskus (with walking) | 5.9 ms |
| Berlin VBB | 42,000 | 71,000 | Hbf → Alex (station-to-station) | 385 µs |
| Paris IDFM | 54,000 | 146,000 | Châtelet → Versailles RD | 17 ms |

For range-query latencies (serial rRAPTOR vs parallel naïve batch), see the [bench source](vulture/benches/gtfs.rs) and the linked benchmark page.

## Soundness

For an issue-by-issue walk through the algorithmic correctness of the implementation against the paper – including the historical record of bugs found and fixed – see [`docs/soundness.md`](docs/soundness.md). The [`vulture-proptest`](vulture-proptest/) harness runs the algorithm against a brute-force reference solver on every test invocation as live validation.

## Cargo features

- `parallel` (default-on) – pulls in `rayon`, enables `Query::run_par` / `Query::run_with_pool`. Opt out with `default-features = false` for wasm or minimal builds; `RaptorCachePool` itself stays available.
- `gtfs-bench` – enables the `gtfs` criterion benchmark.
- `internal` – enables the `manual` criterion benchmark over `manual::SimpleTimetable`.

## License

Apache-2.0
