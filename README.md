# raptor-rs

Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.

Given a transit network, RAPTOR finds all pareto-optimal journeys between two
stops — trading off between fewer transfers and earlier arrival. It does this
really fast.

Based on the paper: [*Round-Based Public Transit Routing*](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) by Delling, Pajor, and Werneck.

## Usage

The core of the crate is the `Timetable` trait. Implement it for your transit
data and you get the `raptor` method for free.

```rust
use raptor::Timetable;

let journeys = my_timetable.raptor(
    3,      // max transfers
    28800,  // departure time: 08:00 in seconds
    source, // source stop
    target, // target stop
);

for journey in &journeys {
    println!("arrives at {} with {} step(s)", journey.arrival, journey.plan.len());
}
```

Each returned journey is Pareto-optimal: arrival strictly decreases as trip
count increases, and no two returned journeys weakly dominate each other.

## Reading a Journey

A `Journey` has a `plan` and an `arrival` time. The plan is a list of
(route, stop) pairs — each entry means "take this route, get off at this stop".
The source stop is implicit; it's not part of the plan.

For example, going from stop `"A"` to stop `"D"` with two transfers:

```rust
[("R1", "B"), ("R2", "C"), ("R3", "D")]
```

Read as: board `R1` at `A`, get off at `B`, board `R2` at `B`, get off at `C`,
board `R3` at `C`, get off at `D`.

The plan records transit boardings only. If the optimal journey ends with a
walk leg from the last boarded alight stop to the target, the plan still ends
at the boarded alight stop and `arrival` reflects the walk-derived arrival
time at the target. Compare `journey.plan.last()`'s stop with your target to
detect this case.

## Implementing `Timetable`

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

let mut cache: RaptorCache<MyRoute, MyStop> = RaptorCache::new();
for query in queries {
    let journeys = timetable.raptor_with_cache(
        &mut cache, query.transfers, query.tau, query.ps, query.pt,
    );
    // ...
}
```

The cache is reset at the start of each call but retains its heap allocations.
A single `RaptorCache` is not thread-safe; give each worker thread its own.

## GTFS Support

A ready-made implementation for GTFS feeds ships in the `gtfs` module:

```rust
use gtfs_structures::Gtfs;
use raptor::gtfs::{GtfsTimetable, RouteId};
use raptor::Timetable;

let gtfs = Gtfs::from_path("path/to/gtfs.zip").unwrap();
let timetable = GtfsTimetable::new(&gtfs).unwrap();

let journeys = timetable.raptor(10, 69300, "stop_a", "stop_b");

for journey in &journeys {
    for (route_id, stop) in &journey.plan {
        // route_id is a synthetic RouteId; recover the original GTFS route_id:
        let gtfs_route = timetable.route_name(*route_id);
        // ... look up further metadata on `gtfs.routes[gtfs_route]` ...
    }
}
```

`GtfsTimetable::new` splits each GTFS `route_id` into one or more synthetic
`RouteId`s — one per equivalence class of trips with identical, non-overtaking
stop sequences. This matches the paper's notion of a "route" and makes the
algorithm sound on real-world feeds (where one `route_id` routinely groups
short-turns, branches, and deadheads). The original `route_id` is recoverable
via `GtfsTimetable::route_name(RouteId)` for display.

There's also a runnable example:

```bash
cargo run --example gtfs-timetable path/to/gtfs.zip stop_a stop_b
```

## License

Apache-2.0
