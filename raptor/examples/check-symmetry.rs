//! Cross-check that `get_routes_serving_stop` and `get_stops_after`
//! agree: for every (route, stop) where `stop ∈ stops_for_route[route]`,
//! `route ∈ routes_for_stop[stop]`.
//!
//! Detects the asymmetry that surfaced in the Paris ARR<DEP diagnosis.

use gtfs_structures::Gtfs;
use raptor::gtfs::GtfsTimetable;
use raptor::{RouteIdx, StopIdx, Timetable};

const FEED: &str = "aux/external/paris.zip";

fn main() -> anyhow::Result<()> {
    let gtfs = Gtfs::new(FEED)?;
    let tt = GtfsTimetable::new(&gtfs)?;

    println!("n_stops={}, n_routes={}", tt.n_stops(), tt.n_routes());

    // For each stop, get_routes_serving_stop -> Set<RouteIdx>
    let mut routes_at: Vec<Vec<RouteIdx>> = Vec::with_capacity(tt.n_stops());
    for s in 0..tt.n_stops() {
        let stop = StopIdx::new(s as u32);
        routes_at.push(tt.get_routes_serving_stop(stop).to_vec());
    }

    // For each route, walk stops_for_route via get_stops_after starting at first stop.
    // We can't access the first stop directly, so probe via the routes_at table:
    // for each (route r, stop s) where r is in routes_at[s], we expect get_stops_after(r, s)
    // to include s. That's a weaker check. Stronger: walk every (s, routes_at[s]) and
    // also probe every (route, stop) returned by stops_after.
    //
    // Instead: for every stop, for every route reportedly serving it,
    // call get_stops_after(route, stop) and verify it doesn't panic. If it panics
    // (caught here? no, we'd just abort) then route doesn't list stop.
    //
    // Reverse direction: walk every (route, stop) pair appearing in get_stops_after,
    // verify routes_at[stop] contains route.

    let mut forward_violations = 0usize;
    let mut reverse_violations: Vec<(RouteIdx, StopIdx)> = Vec::new();

    // Forward: routes_at[stop] -> stops_after(route, stop) should include stop
    for (s_idx, routes) in routes_at.iter().enumerate() {
        let stop = StopIdx::new(s_idx as u32);
        for &route in routes {
            // get_stops_after panics if stop not in route.
            // Catch via std::panic::catch_unwind.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tt.get_stops_after(route, stop)
            }));
            if result.is_err() {
                forward_violations += 1;
                if forward_violations <= 5 {
                    eprintln!(
                        "FORWARD: route {} reportedly serves stop {} (gtfs:{}) but get_stops_after panics",
                        route.get(), stop.get(), tt.stop_id(stop)
                    );
                }
            }
        }
    }
    println!("forward (routes_at[stop] -> stops_after(route, stop)) violations: {forward_violations}");

    // Reverse: for each route, walk its stops via probing every stop the route MIGHT
    // serve. Without internal access, we use a brute approach: for each route r in 0..n_routes,
    // check every stop in 0..n_stops to see if get_stops_after(r, stop) succeeds, then
    // verify routes_at[stop].contains(&r).
    //
    // This is O(n_stops * n_routes) which is 54k * 14k = 750M for Paris. Skip for now;
    // sample the routes that appear in routes_at instead.
    //
    // Sample-based reverse: for each (stop, route) in routes_at, for each pi returned by
    // get_stops_after(route, stop), check routes_at[pi].contains(route).
    let mut sample = 0usize;
    for (s_idx, routes) in routes_at.iter().enumerate().take(10000) {
        let stop = StopIdx::new(s_idx as u32);
        for &route in routes.iter().take(3) {
            // Limit per-stop probes
            let stops_after = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tt.get_stops_after(route, stop).to_vec()
            }));
            let Ok(stops_after) = stops_after else {
                continue;
            };
            for pi in stops_after.iter().take(3) {
                sample += 1;
                if !routes_at[pi.get() as usize].contains(&route) {
                    reverse_violations.push((route, *pi));
                    if reverse_violations.len() <= 5 {
                        eprintln!(
                            "REVERSE: stops_after(route={}, ?) yields stop {} (gtfs:{}) but routes_at[{}] doesn't contain route {}",
                            route.get(),
                            pi.get(),
                            tt.stop_id(*pi),
                            pi.get(),
                            route.get(),
                        );
                    }
                }
            }
        }
    }
    println!(
        "reverse (sampled {} probes) violations: {}",
        sample,
        reverse_violations.len()
    );

    Ok(())
}
