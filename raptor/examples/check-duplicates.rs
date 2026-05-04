//! Count GTFS trips whose stop_sequence has duplicates (loops / revisits).
//! Routes with duplicate stops trigger a known correctness bug in our
//! adapter: `Vec::position()` returns only the FIRST occurrence, so
//! `get_arrival_time` and `get_stops_after` mis-resolve the second.

use gtfs_structures::Gtfs;
use std::collections::HashSet;

fn main() -> anyhow::Result<()> {
    for (label, path) in &[
        ("Delhi", "aux/dmrc_gtfs.zip"),
        ("Helsinki", "aux/external/helsinki.zip"),
        ("Berlin", "aux/external/berlin.zip"),
        ("Paris", "aux/external/paris.zip"),
    ] {
        let gtfs = match Gtfs::new(path) {
            Ok(g) => g,
            Err(_) => {
                println!("{label}: feed not present, skipping");
                continue;
            }
        };
        let total = gtfs.trips.len();
        let mut bad_trips = 0usize;
        let mut bad_route_ids: HashSet<&str> = HashSet::new();
        for (_trip_id, trip) in &gtfs.trips {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut had_dup = false;
            for st in &trip.stop_times {
                if !seen.insert(st.stop.id.as_str()) {
                    had_dup = true;
                    break;
                }
            }
            if had_dup {
                bad_trips += 1;
                bad_route_ids.insert(trip.route_id.as_str());
            }
        }
        println!(
            "{label}: {bad_trips}/{total} trips with duplicate stops in stop_sequence; {} distinct route_ids affected",
            bad_route_ids.len()
        );
    }
    Ok(())
}
