//! One-shot diagnostic for the Paris ARR<DEP issue surfaced by
//! `examples/cross-city-bench.rs`.
//!
//! Loads `aux/external/paris.zip`, runs the Châtelet → Gare du Nord
//! query at 09:00, and prints the full journey set — for each journey,
//! the plan as (route_id, stop_id, arrival, departure) tuples — so we
//! can trace back into the GTFS to find where the impossible arrival
//! time comes from.

use gtfs_structures::Gtfs;
use raptor::gtfs::GtfsTimetable;
use raptor::{Tau, Timetable};

const FEED: &str = "aux/external/paris.zip";
const ORIGIN: &str = "IDFM:monomodalStopPlace:45102"; // Châtelet
const TARGET: &str = "IDFM:monomodalStopPlace:462394"; // Gare du Nord
const DEPART: Tau = 9 * 3600;

fn main() -> anyhow::Result<()> {
    let gtfs = Gtfs::new(FEED)?;
    let tt = GtfsTimetable::new(&gtfs)?;

    let origin = tt.stop_idx(ORIGIN).expect("origin");
    let target = tt.stop_idx(TARGET).expect("target");

    println!("query: {} -> {} at {}", ORIGIN, TARGET, hms(DEPART));
    println!("origin idx: {origin}, target idx: {target}");

    // Run TWICE with a shared cache; compare to fresh-cache runs.
    use raptor::RaptorCache;
    let mut cache = RaptorCache::for_timetable(&tt);

    println!("\n=== run 1: fresh cache ===");
    let journeys = tt.raptor_with_cache(&mut cache, 10, DEPART, origin, target);
    println!("found {} journey(s)", journeys.len());
    for (i, j) in journeys.iter().enumerate() {
        println!("  [{i}] arrival={} ({})", j.arrival, hms(j.arrival));
    }

    println!("\n=== run 2: same cache, same query ===");
    let journeys2 = tt.raptor_with_cache(&mut cache, 10, DEPART, origin, target);
    println!("found {} journey(s)", journeys2.len());
    for (i, j) in journeys2.iter().enumerate() {
        println!("  [{i}] arrival={} ({})", j.arrival, hms(j.arrival));
    }

    // Now reproduce the BENCH sequence: run 3 different queries with the SAME cache.
    let queries = [
        ("Gare du Nord", "IDFM:monomodalStopPlace:462394"),
        ("La Défense", "IDFM:monomodalStopPlace:470549"),
        ("Versailles Rive Droite", "IDFM:monomodalStopPlace:44602"),
    ];
    println!("\n=== bench sequence: 3 queries with same cache ===");
    for (label, target_id) in &queries {
        let t = tt.stop_idx(target_id).expect("target");
        let js = tt.raptor_with_cache(&mut cache, 10, DEPART, origin, t);
        println!("{label}: {} journey(s)", js.len());
        for (i, j) in js.iter().enumerate() {
            let marker = if j.arrival < DEPART { " *** ARR<DEP ***" } else { "" };
            println!("  [{i}] arrival={} ({}){}", j.arrival, hms(j.arrival), marker);
        }
    }

    // Now zoom in on Versailles, where ARR<DEP showed.
    println!("\n=== inspect Versailles journeys ===");
    let versailles = tt.stop_idx("IDFM:monomodalStopPlace:44602").expect("versailles");
    let journeys_v = tt.raptor_with_cache(&mut cache, 10, DEPART, origin, versailles);
    println!("found {} journey(s) Châtelet -> Versailles", journeys_v.len());
    for (i, j) in journeys_v.iter().enumerate() {
        let bad = if j.arrival < DEPART { " *** ARR<DEP ***" } else { "" };
        println!(
            "  [{i}] arr={} ({}); plan.len()={}{}",
            j.arrival, hms(j.arrival), j.plan.len(), bad
        );
    }
    if let Some(bad) = journeys_v.iter().find(|j| j.arrival < DEPART) {
        println!(
            "\n--- step-by-step simulation of bad journey (plan.len()={}, arr={}) ---",
            bad.plan.len(),
            hms(bad.arrival)
        );
        let mut current_stop = origin;
        let mut current_time = DEPART;
        for (step_i, &(route, alight)) in bad.plan.iter().enumerate() {
            println!(
                "  step {step_i}: board at idx={} (gtfs:{}), current_time={} ({})",
                current_stop.get(),
                tt.stop_id(current_stop),
                current_time, hms(current_time)
            );
            // Confirm the route serves current_stop:
            let routes_here: Vec<_> = tt.get_routes_serving_stop(current_stop).iter().copied().collect();
            let route_present = routes_here.contains(&route);
            println!(
                "    route {} (gtfs:{}) serving current stop? {}",
                route.get(), tt.route_id(route), route_present
            );
            // What trip would the algorithm pick?
            match tt.get_earliest_trip(route, current_time, current_stop) {
                Some(t) => {
                    let dep = tt.get_departure_time(t, current_stop);
                    let arr = tt.get_arrival_time(t, alight);
                    println!(
                        "    earliest trip: idx={} (gtfs:{}), dep at board={} ({}), arr at alight={} ({})",
                        t.get(), tt.trip_id(t), dep, hms(dep), arr, hms(arr)
                    );
                    if dep < current_time {
                        println!("    !!! trip departs BEFORE current_time !!!");
                    }
                    current_time = arr;
                    current_stop = alight;
                }
                None => {
                    println!("    NO trip available — algorithm should not have included this leg");
                    break;
                }
            }
        }
        println!("  final reconstructed arrival: {} ({})", current_time, hms(current_time));
        if current_time != bad.arrival {
            println!(
                "  !!! MISMATCH: journey says arrival={} ({}) but step-by-step says {} ({})",
                bad.arrival, hms(bad.arrival), current_time, hms(current_time)
            );
        }
    }

    println!("\n=== full plan for worst (most-trips, earliest-arrival) journey ===");
    let journeys = tt.raptor_with_cache(&mut cache, 10, DEPART, origin, target);
    if let Some(worst) = journeys.iter().max_by_key(|j| j.plan.len()) {
        println!(
            "plan.len()={}, arrival={} ({})",
            worst.plan.len(),
            worst.arrival,
            hms(worst.arrival)
        );
        for (i, (r, s)) in worst.plan.iter().enumerate() {
            let raw_route = tt.route_id(*r);
            let raw_stop = tt.stop_id(*s);
            // Find a trip on this route departing within the journey window
            // and report its dep/arr at the alight stop.
            let dep_arr = (0..tt.n_routes())
                .find(|_| false) // placeholder
                .and(None::<()>);
            let _ = dep_arr;

            // Also: what's get_earliest_trip yield for (route, ?, alight_stop)?
            // We don't know the boarding stop without reconstruction — print what we have.
            println!(
                "  step {i}: synth_route_idx={} (gtfs:{}), alight_idx={} (gtfs:{})",
                r.get(),
                raw_route,
                s.get(),
                raw_stop,
            );
            // Look at the per-route stops + trips count
            let stops_in_route = tt.get_stops_after(*r, *s).len();
            // Find any trip serving the alight stop on this route
            let any_trip = tt.get_earliest_trip(*r, 0, *s);
            if let Some(t) = any_trip {
                let arr_here = tt.get_arrival_time(t, *s);
                println!(
                    "    earliest trip on this route at this stop: idx={} (gtfs:{}), arrival_time at this stop={} ({})",
                    t.get(),
                    tt.trip_id(t),
                    arr_here,
                    hms(arr_here),
                );
                println!("    stops remaining after alight: {stops_in_route}");
            }
        }
    }

    for (i, j) in journeys.iter().enumerate() {
        println!(
            "[journey {i}] arrival={} ({}); plan.len()={}",
            j.arrival,
            hms(j.arrival),
            j.plan.len()
        );
        if j.arrival < DEPART {
            println!(
                "  ^^ ARRIVAL BEFORE DEPARTURE: arrival={} < tau={}",
                hms(j.arrival),
                hms(DEPART)
            );
        }
        for (step, (route_idx, stop_idx)) in j.plan.iter().enumerate() {
            let route_id = tt.route_id(*route_idx);
            let stop_id = tt.stop_id(*stop_idx);
            // Look up which GTFS route this synthetic came from.
            println!(
                "  step {step}: route={} (gtfs:{}) -> alight stop={} (gtfs:{})",
                route_idx.get(),
                route_id,
                stop_idx.get(),
                stop_id
            );
        }
        // For the first journey only, dive deeper: print the route's
        // synthetic-route trips and times at the boarding/alight stops.
        if i == 0 {
            inspect_first_step(&tt, &gtfs, j, origin);
        }
        println!();
    }

    Ok(())
}

fn inspect_first_step(
    tt: &GtfsTimetable,
    gtfs: &Gtfs,
    j: &raptor::Journey,
    origin: raptor::StopIdx,
) {
    let Some(&(first_route, first_alight)) = j.plan.first() else { return };
    println!("\n  --- first-step inspection ---");
    println!(
        "  first synthetic route {} (gtfs:{})",
        first_route.get(),
        tt.route_id(first_route)
    );
    println!("  boarding at origin idx {} (gtfs:{})", origin.get(), tt.stop_id(origin));
    println!(
        "  alighting at idx {} (gtfs:{})",
        first_alight.get(),
        tt.stop_id(first_alight)
    );

    // Simulate get_earliest_trip's logic to see which trip we'd pick.
    let trip = tt.get_earliest_trip(first_route, DEPART, origin);
    match trip {
        Some(t) => {
            let dep = tt.get_departure_time(t, origin);
            let arr = tt.get_arrival_time(t, first_alight);
            println!(
                "  picked trip idx={} (gtfs:{}): dep at origin={} ({}), arr at alight={} ({})",
                t.get(),
                tt.trip_id(t),
                dep,
                hms(dep),
                arr,
                hms(arr),
            );
            if dep < DEPART {
                println!(
                    "  ^^ get_earliest_trip returned a trip departing BEFORE tau (dep={} < tau={})",
                    dep, DEPART
                );
            }
            if arr < dep {
                println!("  ^^ trip's arrival at alight is BEFORE its departure at origin");
            }
            // Look up the raw stop_times for this trip in the GTFS for sanity.
            println!("\n  raw stop_times entries for this trip (first 3):");
            let raw_trip = gtfs.get_trip(tt.trip_id(t)).unwrap();
            for (k, st) in raw_trip.stop_times.iter().take(5).enumerate() {
                println!(
                    "    {k}: stop={}, arr={:?}, dep={:?}",
                    st.stop.id, st.arrival_time, st.departure_time
                );
            }
        }
        None => {
            println!("  get_earliest_trip returned None (no trip departs origin >= tau)");
        }
    }
}

fn hms(t: Tau) -> String {
    let h = t / 3600;
    let m = (t % 3600) / 60;
    let s = t % 60;
    format!("{h:02}:{m:02}:{s:02}")
}
