//! Reboarding: when a faster feeder route reaches a stop mid-way along a
//! shared route, the algorithm records the boarding stop as the stop it
//! was first reached at — not the shared route's earliest call.
//!
//! Network:
//!
//! ```text
//!     R1  S ─────────────── A
//!     R2  S ──── B
//!     R3          A ─── B ─── C ─── D    (two trips: early, late)
//! ```
//!
//! Two ways from S to D:
//!
//! * via R1 to A (arrive 100), catch R3-late from A (departs 105) → D @ 130.
//! * via R2 to B (arrive 30), catch R3-early from B (departs 30) → D @ 50.
//!
//! Option 2 wins. The point is that the boarding stop recorded for the R3
//! leg is B, not A — even though R3's earliest call is A.
//!
//! Run: `cargo run --example reboarding`

use vulture::manual::SimpleTimetable;
use vulture::{Duration, RouteIdx, SecondOfDay, StopIdx, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Stop {
    S,
    A,
    B,
    C,
    D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Route {
    R1,
    R2,
    R3,
}

type Tt = SimpleTimetable<Stop, Route, &'static str>;

fn main() {
    use Route::*;
    use Stop::*;

    let t = |secs: u32| (SecondOfDay(secs), SecondOfDay(secs));

    let tt: Tt = SimpleTimetable::new()
        .route(R1, &[S, A], &[("R1T1", &[t(0), t(100)])])
        .route(R2, &[S, B], &[("R2T1", &[t(0), t(30)])])
        .route(
            R3,
            &[A, B, C, D],
            &[
                ("R3early", &[t(25), t(30), t(40), t(50)]),
                ("R3late", &[t(105), t(110), t(120), t(130)]),
            ],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&S), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();

    println!("S → D, departing 00:00:00 — {} journey(s):", journeys.len());
    for j in &journeys {
        println!("  arrives {} ({} legs)", j.arrival(), j.plan.len());
        let mut board = S; // origin
        for (route_idx, alight_idx) in &j.plan {
            let route = route_for(&tt, *route_idx);
            let alight = stop_for(&tt, *alight_idx);
            println!("    {route:?}: board at {board:?}, alight at {alight:?}");
            board = alight;
        }
    }
}

fn stop_for(tt: &Tt, idx: StopIdx) -> Stop {
    [Stop::S, Stop::A, Stop::B, Stop::C, Stop::D]
        .into_iter()
        .find(|s| tt.stop_idx_of(s) == idx)
        .expect("stop index belongs to this network")
}

fn route_for(tt: &Tt, idx: RouteIdx) -> Route {
    [Route::R1, Route::R2, Route::R3]
        .into_iter()
        .find(|r| tt.route_idx_of(r) == idx)
        .expect("route index belongs to this network")
}
