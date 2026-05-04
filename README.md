# raptor-rs

Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.

Given a transit network, RAPTOR finds all Pareto-optimal journeys between two
stops — trading off between fewer transfers and earlier arrival.

Based on the paper: [*Round-Based Public Transit Routing*](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) by Delling, Pajor, and Werneck.

## Quick start: query a GTFS feed

The `gtfs` module wraps a parsed GTFS feed (via the
[`gtfs-structures`](https://crates.io/crates/gtfs-structures) crate) and
implements the `Timetable` trait for it. Most users start here.

```rust
use gtfs_structures::Gtfs;
use jiff::civil::date;
use raptor::Timetable;
use raptor::gtfs::GtfsTimetable;

let gtfs = Gtfs::new("path/to/gtfs.zip")?;
// GTFS feeds describe many service days; pin the timetable to one date.
// Trips whose service_id is not active on this day are filtered out.
let timetable = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;

// `raptor` takes dense u32 indices, not GTFS string IDs — resolve first.
let start = timetable.stop_idx("dilshad_garden").expect("unknown stop");
let target = timetable.stop_idx("vishwavidyalaya").expect("unknown stop");

// `raptor` is multi-source / multi-target: pass each origin and target as
// (stop, walk_time_offset). Single-stop queries use [(stop, 0)].
// 10 = max transfers; 32400 = depart at 09:00 (seconds since midnight).
let journeys = timetable.raptor(10, 32400, &[(start, 0)], &[(target, 0)]);

for journey in &journeys {
    print!("arrives {}s, plan: ", journey.arrival);
    for (route_idx, stop_idx) in &journey.plan {
        let route = timetable.route_id(*route_idx);   // original GTFS route_id
        let stop = timetable.stop_id(*stop_idx);      // original GTFS stop_id
        print!("[{route} -> {stop}] ");
    }
    println!();
}
```

A returned `Journey` has `plan: Vec<(RouteIdx, StopIdx)>` (each entry is
"take this route, get off at this stop"), plus `origin` and `target` fields
recording which user-supplied stops the algorithm picked, and `arrival`
in seconds since midnight (including the chosen target's walk-time offset).
Each returned journey is Pareto-optimal: arrival strictly decreases as trip
count increases, and no two returned journeys weakly dominate each other.

For station-level queries (where any platform of a parent station is an
acceptable origin or target), `GtfsTimetable::station_stops(parent_id)`
returns the children as a slice ready to pass to `raptor`:

```rust,ignore
let origins = timetable.station_stops("berlin_hbf");      // 301 platforms
let targets = timetable.station_stops("berlin_alex");     // 50 platforms
let journeys = timetable.raptor(10, 32400, origins, targets);
```

A runnable version of the above is in `examples/gtfs-timetable.rs`:

```bash
cargo run --example gtfs-timetable -- path/to/gtfs.zip start_id target_id
```

## Performance

Single-query latency on the bundled Delhi Metro feed
(`aux/dmrc_gtfs.zip` — 262 stops, 36 routes, 5,438 trips), measured with
[`criterion`](https://crates.io/crates/criterion) on Apple Silicon (M-series),
warm cache reused across iterations:

| Query                                   | Median latency |
|-----------------------------------------|----------------|
| Direct, 1 trip (Dilshad Garden→Shahdara)| 3.5 µs         |
| 2-trip with one interchange             | 43 µs          |
| 3-trip across three lines               | 94 µs          |

Construction (parsing the GTFS zip + building the indexed timetable)
dominates at ~98 ms — pay it once at startup, then queries are essentially
free. For server workloads doing many queries against the same timetable,
reuse a [`RaptorCache`](#reusing-a-raptorcache) to amortise scratch-buffer
allocation.

To reproduce these numbers on your own hardware:

```bash
cargo bench -p raptor --features gtfs-bench --bench gtfs
```

For numbers on larger feeds — Helsinki HSL (~8k stops), Berlin VBB
(~42k stops), Paris IDFM (~54k stops) — see
[`docs/cross-city-benchmarks.md`](docs/cross-city-benchmarks.md). That
page also documents three real-feed limitations the cross-city run
surfaced (parent-station handling, calendar filtering, transfer-graph
density), all of which are queued as follow-up work.

## Reading a Journey

A `Journey` has a `plan` and an `arrival` time. The plan is a
`Vec<(RouteIdx, StopIdx)>` — each entry means "take this route, get off at
this stop". The source stop is implicit; it is not part of the plan.

The plan records transit boardings only. If the optimal journey ends with a
walk leg from the last boarded alight stop to the target, the plan still
ends at the boarded alight stop and `arrival` reflects the walk-derived
arrival time at the target. Compare `journey.plan.last()`'s stop with your
target to detect this case.

To translate index newtypes back to your adapter's external IDs, the bundled
GTFS adapter exposes `GtfsTimetable::stop_id(stop_idx)` and
`GtfsTimetable::route_id(route_idx)`. The synthetic-route splitting (one GTFS
`route_id` may map to several `RouteIdx`s — one per equivalence class of
trips with identical, non-overtaking stop sequences) is documented on
`GtfsTimetable`; `routes_for_gtfs_id(&str)` enumerates the synthetics
derived from a single GTFS route.

## Reusing a `RaptorCache`

Calling `Timetable::raptor` allocates fresh scratch buffers on every query.
For server workloads doing many queries against the same timetable, prefer
`raptor_with_cache`:

```rust
use raptor::{RaptorCache, Timetable};

let mut cache = RaptorCache::for_timetable(&timetable);
for query in queries {
    let journeys = timetable.raptor_with_cache(
        &mut cache, query.transfers, query.tau, query.ps, query.pt,
    );
    // ...
}
```

A `RaptorCache` is sized for a specific timetable's `n_stops()`/`n_routes()`.
Calling `raptor_with_cache` with a cache whose dimensions differ from the
timetable in scope panics on entry — share a cache only across queries
against the same timetable. If you do not have the timetable in scope at
cache-construction time, `RaptorCache::with_capacity(n_stops, n_routes)` is
the count-only equivalent.

The cache is reset at the start of each call but retains its heap
allocations. A single `RaptorCache` is not thread-safe; give each worker
thread its own.

## Implementing `Timetable` for a custom backend

If you have transit data not in GTFS form, implement the `Timetable` trait.
It is non-generic. Identifiers are dense `u32` newtypes — `StopIdx`,
`RouteIdx`, `TripIdx` — and slice-returning accessors return plain `&[T]`
(no `Cow`). Implementors expose two count methods — `n_stops()` and
`n_routes()` — alongside the lookup methods the algorithm calls in its hot
loop. Adapters are responsible for interning external identifiers (e.g. GTFS
string IDs) to dense indices at construction.

Two contracts the algorithm relies on:

- **Footpaths must be transitively closed.** If `A → B` and `B → C` are both
  walkable, you must also report `A → C` from `get_footpaths_from(A)`. The
  algorithm relaxes footpaths once per round; it does not iterate to a fixed
  point. Most well-formed GTFS feeds satisfy this because `transfers.txt`
  consists of explicit pairs.
- **Trips on a route must share a stop sequence and not overtake.** The
  algorithm binary-searches by departure time at intermediate stops. If your
  data source groups trips with different stop patterns or overtaking pairs
  under one route, split them at construction. The bundled GTFS adapter does
  this automatically.

Both invariants are documented on the `Timetable` trait.

## License

Apache-2.0
