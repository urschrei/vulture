# raptor-rs

Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.

Given a transit network, RAPTOR finds all pareto-optimal journeys between two
stops — trading off between fewer transfers and earlier arrival.

Based on the paper: [*Round-Based Public Transit Routing*](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) by Delling, Pajor, and Werneck.

## Usage

The core of the crate is the `Timetable` trait. Implement it for your transit
data and you get the `raptor` method for free.

```rust
use raptor::Timetable;

// `source` and `target` are `StopIdx` values — dense `u32` indices into the
// timetable's stop table. Adapters intern from external IDs (e.g. GTFS
// string IDs) at construction; see the GTFS section below.
let journeys = my_timetable.raptor(
    3,      // max transfers
    28800,  // departure time: 08:00 in seconds
    source, // source StopIdx
    target, // target StopIdx
);

for journey in &journeys {
    println!("arrives at {} with {} step(s)", journey.arrival, journey.plan.len());
}
```

Each returned journey is Pareto-optimal: arrival strictly decreases as trip
count increases, and no two returned journeys weakly dominate each other.

## Reading a Journey

A `Journey` has a `plan` and an `arrival` time. The plan is a
`Vec<(RouteIdx, StopIdx)>` — each entry means "take this route, get off at
this stop". The source stop is implicit; it's not part of the plan.

For example, a journey from stop `A` to stop `D` with two transfers has a
plan of three entries:

```rust
// (route, alight_stop) pairs, all index newtypes around u32
[(r1, b_idx), (r2, c_idx), (r3, d_idx)]
```

Read as: board `r1` at `A`, get off at `b_idx`, board `r2` there, get off at
`c_idx`, board `r3` there, get off at `d_idx`. To translate the indices back
to your adapter's external IDs, use the adapter's lookup methods — for the
bundled GTFS adapter, `GtfsTimetable::stop_id(stop_idx)` and
`GtfsTimetable::route_id(route_idx)` return the original GTFS string IDs.

The plan records transit boardings only. If the optimal journey ends with a
walk leg from the last boarded alight stop to the target, the plan still ends
at the boarded alight stop and `arrival` reflects the walk-derived arrival
time at the target. Compare `journey.plan.last()`'s stop with your target to
detect this case.

## Implementing `Timetable`

The trait is non-generic. Identifiers are dense `u32` newtypes —
`StopIdx`, `RouteIdx`, `TripIdx` — and slice-returning accessors return
plain `&[T]` (no `Cow`). Implementors expose two count methods —
`n_stops()` and `n_routes()` — alongside the lookup methods the algorithm
calls in its hot loop. Adapters are responsible for interning external
identifiers (e.g. GTFS string IDs) to dense indices at construction.

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
  this automatically (see below).

Both invariants are documented on the `Timetable` trait.

## Performance: reusing a `RaptorCache`

Calling `Timetable::raptor` allocates fresh scratch buffers on every query.
For server use cases running many queries against the same timetable, prefer
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

## GTFS Support

A ready-made implementation for GTFS feeds ships in the `gtfs` module:

```rust
use gtfs_structures::Gtfs;
use raptor::gtfs::GtfsTimetable;
use raptor::Timetable;

let gtfs = Gtfs::new("path/to/gtfs.zip").unwrap();
let timetable = GtfsTimetable::new(&gtfs).unwrap();

// `GtfsTimetable::raptor` takes `StopIdx` arguments; resolve string IDs first.
let start = timetable.stop_idx("stop_a").expect("unknown stop");
let target = timetable.stop_idx("stop_b").expect("unknown stop");
let journeys = timetable.raptor(10, 69300, start, target);

for journey in &journeys {
    for (route_idx, stop_idx) in &journey.plan {
        // Recover the original GTFS string IDs for display:
        let gtfs_route = timetable.route_id(*route_idx);
        let gtfs_stop = timetable.stop_id(*stop_idx);
        // ... look up further metadata on `gtfs.routes[gtfs_route]` ...
    }
}
```

`GtfsTimetable::new` splits each GTFS `route_id` into one or more synthetic
`RouteIdx`s — one per equivalence class of trips with identical,
non-overtaking stop sequences. This matches the paper's notion of a "route"
and makes the algorithm sound on real-world feeds (where one `route_id`
routinely groups short-turns, branches, and deadheads). The original
`route_id` is recoverable via `GtfsTimetable::route_id(RouteIdx)` for
display, and `GtfsTimetable::routes_for_gtfs_id(&str)` enumerates every
synthetic `RouteIdx` derived from a given GTFS `route_id`.

There's also a runnable example:

```bash
cargo run --example gtfs-timetable path/to/gtfs.zip stop_a stop_b
```

## License

Apache-2.0
