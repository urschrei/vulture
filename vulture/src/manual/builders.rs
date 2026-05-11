//! Synthetic-network builders for the criterion bench harness (and the
//! `vulture-dotgraph` CLI). Each `build_*` produces a fully-formed
//! [`SimpleTimetable`] with `usize` keys for stops, routes, and trips, so the
//! IDs in the rendered output match the indices used in the source.

use super::SimpleTimetable;
use crate::{Duration, SecondOfDay};

type Net = SimpleTimetable<usize, usize, usize>;

/// Per-stop `(arrival, departure)` schedule that starts at `base` seconds,
/// advances `step` seconds between consecutive stops, and dwells `dwell`
/// seconds at each stop.
fn schedule(base: u32, step: u32, dwell: u32, stops: usize) -> Vec<(SecondOfDay, SecondOfDay)> {
    (0..stops as u32)
        .map(|i| {
            let arr = base + i * step;
            (SecondOfDay(arr), SecondOfDay(arr + dwell))
        })
        .collect()
}

/// Single route serving stops `0..stops`, with `trips` back-to-back trips.
/// Trip `t` occupies the time window `[t * stops * 2, (t + 1) * stops * 2)`.
pub fn build_linear(stops: usize, trips: usize) -> Net {
    let stop_keys: Vec<usize> = (0..stops).collect();
    let trip_times: Vec<Vec<(SecondOfDay, SecondOfDay)>> = (0..trips)
        .map(|t| schedule((t * stops * 2) as u32, 2, 1, stops))
        .collect();
    let trip_specs: Vec<(usize, &[(SecondOfDay, SecondOfDay)])> = trip_times
        .iter()
        .enumerate()
        .map(|(t, times)| (t, times.as_slice()))
        .collect();
    SimpleTimetable::new().route(0, &stop_keys, &trip_specs)
}

/// `routes` horizontal routes (each `stops_per_route` long), joined by
/// `connectors` vertical connector routes that link the corresponding column
/// of stops across every horizontal route.
pub fn build_grid(routes: usize, stops_per_route: usize, connectors: usize) -> Net {
    let mut tt = SimpleTimetable::new();
    let mut id = 0usize;

    for r in 0..routes {
        let stops: Vec<usize> = (r * stops_per_route..(r + 1) * stops_per_route).collect();
        let times = schedule(0, 10, 5, stops_per_route);
        tt = tt.route(id, &stops, &[(id, times.as_slice())]);
        id += 1;
    }

    let col_step = stops_per_route.max(1) / connectors.max(1);
    for c in 0..connectors {
        let col = c * col_step;
        let stops: Vec<usize> = (0..routes).map(|r| r * stops_per_route + col).collect();
        let times: Vec<(SecondOfDay, SecondOfDay)> = (0..routes as u32)
            .map(|r| {
                let t = (col * 10) as u32 + r * 3;
                (SecondOfDay(t), SecondOfDay(t + 1))
            })
            .collect();
        tt = tt.route(id, &stops, &[(id, times.as_slice())]);
        id += 1;
    }

    tt
}

/// `hubs` central hub stops, each fed by `routes_per_hub` routes that radiate
/// out to `spokes` spoke stops. Every distinct pair of hubs is connected by a
/// 2-second footpath in both directions.
pub fn build_hub_spoke(hubs: usize, routes_per_hub: usize, spokes: usize) -> Net {
    let mut tt = SimpleTimetable::new();
    let mut id = 0usize;

    // Hub keys are 0..hubs; spoke keys are allocated sequentially after that.
    let mut next_spoke = hubs;
    for h in 0..hubs {
        for r in 0..routes_per_hub {
            let mut stops = Vec::with_capacity(spokes + 1);
            stops.push(h);
            for _ in 0..spokes {
                stops.push(next_spoke);
                next_spoke += 1;
            }
            let base = ((h * routes_per_hub + r) * (spokes + 1) * 10) as u32;
            let times = schedule(base, 10, 5, spokes + 1);
            tt = tt.route(id, &stops, &[(id, times.as_slice())]);
            id += 1;
        }
    }

    for i in 0..hubs {
        for j in 0..hubs {
            if i == j {
                continue;
            }
            tt = tt.footpath(i, j).transfer_time(i, j, Duration(2));
        }
    }

    tt
}

/// Chain of single-leg routes `0 -> 1 -> 2 -> ...`. Each segment requires its
/// own transfer, so journeys spanning the chain exercise the multi-round
/// scanning path.
pub fn build_chain(segments: usize) -> Net {
    let mut tt = SimpleTimetable::new();
    for seg in 0..segments {
        let base = (seg * 20) as u32;
        let times = [
            (SecondOfDay(base), SecondOfDay(base + 5)),
            (SecondOfDay(base + 10), SecondOfDay(base + 15)),
        ];
        tt = tt.route(seg, &[seg, seg + 1], &[(seg, &times)]);
    }
    tt
}

/// `path_count` parallel paths from stop `0` (source) to stop `1` (target).
/// Path `p` has `((p % max_legs) + 1).min(max_legs)` legs and a total journey
/// time of `1000 - p * 50` seconds, so later paths are slightly faster but
/// require more transfers.
pub fn build_parallel_paths(path_count: usize, max_legs: usize) -> Net {
    const SOURCE: usize = 0;
    const TARGET: usize = 1;

    let mut tt = SimpleTimetable::new();
    let mut id = 0usize;
    let mut next_intermediate = 2usize;

    for p in 0..path_count {
        let legs = ((p % max_legs) + 1).min(max_legs);
        let leg_time = (1000 - p * 50) / legs;

        let mut prev = SOURCE;
        for leg in 0..legs {
            let is_last = leg + 1 == legs;
            let curr = if is_last {
                TARGET
            } else {
                let s = next_intermediate;
                next_intermediate += 1;
                s
            };
            let depart = (leg * leg_time) as u32;
            let arrive = depart + leg_time as u32 - 5;
            let times = [
                (SecondOfDay(depart), SecondOfDay(depart + 1)),
                (SecondOfDay(arrive), SecondOfDay(arrive + 1)),
            ];
            tt = tt.route(id, &[prev, curr], &[(id, &times)]);
            id += 1;
            prev = curr;
        }
    }

    tt
}
