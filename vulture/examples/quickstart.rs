//! Smallest viable RAPTOR query. Builds a four-stop linear route with a
//! single trip and asks for the journey from A to D, departing at 09:00.
//!
//! This is the recommended starting point for synthetic networks:
//! `SimpleTimetable` does the index bookkeeping so you can describe the
//! network in terms of meaningful keys (here, an enum) and read journeys
//! back using the same keys via `stop_idx_of` / `route_idx_of`.
//!
//! Run: `cargo run --example quickstart`
//!
//! For real GTFS feeds, see `gtfs-timetable.rs`.

use vulture::manual::SimpleTimetable;
use vulture::{Duration, SecondOfDay, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Stop {
    A,
    B,
    C,
    D,
}

fn main() {
    // A single trip on a single route, calling at A, B, C, D every few
    // minutes from 09:00. Each `(arr, dep)` pair uses zero dwell time.
    let nine = SecondOfDay::hms(9, 0, 0);
    let times: Vec<(SecondOfDay, SecondOfDay)> = [0, 5 * 60, 12 * 60, 20 * 60]
        .iter()
        .map(|&offset| {
            let t = nine + Duration(offset);
            (t, t)
        })
        .collect();

    let tt = SimpleTimetable::new().route(
        "L1",
        &[Stop::A, Stop::B, Stop::C, Stop::D],
        &[("L1-09:00", times.as_slice())],
    );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(1)
        .depart_at(nine)
        .run();

    for (i, journey) in journeys.iter().enumerate() {
        println!(
            "journey {}: depart {nine}, arrive {} ({} hops)",
            i + 1,
            journey.arrival(),
            journey.plan.len(),
        );
    }
}
