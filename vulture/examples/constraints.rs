//! GTFS-style boarding constraints: wheelchair accessibility, pickup-only
//! stops, drop-off-only stops. These flags are usually carried in real GTFS
//! feeds (`trips.wheelchair_accessible`, `stop_times.pickup_type`,
//! `stop_times.drop_off_type`); the `SimpleTimetable` builder lets you set
//! the equivalent flags by hand for tests and demos.
//!
//! Three scenarios:
//!
//! 1. **Wheelchair**: one accessible trip and one inaccessible trip on the
//!    same route. The default query takes the faster (inaccessible) trip;
//!    `.require_wheelchair_accessible()` reroutes to the accessible one.
//!
//! 2. **No pickup**: a stop on a route can't be boarded from. A query
//!    whose source is that stop returns no journey.
//!
//! 3. **No drop-off**: a stop on a route can't be alighted at. A query
//!    whose target is that stop returns no journey.
//!
//! Run: `cargo run --example constraints`

use vulture::manual::SimpleTimetable;
use vulture::{Duration, SecondOfDay, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Stop {
    A,
    B,
    C,
}

fn main() {
    wheelchair();
    no_pickup();
    no_drop_off();
}

fn wheelchair() {
    let t = |secs: u32| (SecondOfDay(secs), SecondOfDay(secs));
    let nine = SecondOfDay::hms(9, 0, 0);
    let nine_30 = nine + Duration(30 * 60);
    let nine_05 = nine + Duration(5 * 60);
    let nine_35 = nine_30 + Duration(5 * 60);

    let tt = SimpleTimetable::new()
        .route(
            "L1",
            &[Stop::A, Stop::B],
            &[
                ("L1-inaccessible", &[t(nine.0), t(nine_05.0)]),
                ("L1-accessible", &[t(nine_30.0), t(nine_35.0)]),
            ],
        )
        .no_wheelchair_on_trip("L1-inaccessible");

    let a = tt.stop_idx_of(&Stop::A);
    let b = tt.stop_idx_of(&Stop::B);

    let plain = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(nine)
        .run();
    let accessible_only = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(1)
        .require_wheelchair_accessible()
        .depart_at(nine)
        .run();

    println!("=== wheelchair ===");
    println!("default query:               arrive {}", plain[0].arrival());
    println!(
        "require_wheelchair query:    arrive {}\n",
        accessible_only[0].arrival()
    );
}

fn no_pickup() {
    let t = |secs: u32| (SecondOfDay(secs), SecondOfDay(secs));

    // Express route A → B → C with pickup at B disabled (position 1).
    let tt = SimpleTimetable::new()
        .route(
            "L1",
            &[Stop::A, Stop::B, Stop::C],
            &[("T1", &[t(0), t(60), t(120)])],
        )
        .no_pickup_at("T1", 1);

    let a = tt.stop_idx_of(&Stop::A);
    let b = tt.stop_idx_of(&Stop::B);
    let c = tt.stop_idx_of(&Stop::C);

    let from_a = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();
    let from_b = tt
        .query()
        .from(&[(b, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();

    println!("=== no pickup at B ===");
    println!("A → C: {} journey(s)", from_a.len());
    println!(
        "B → C: {} journey(s) (boarding at B is forbidden)\n",
        from_b.len()
    );
}

fn no_drop_off() {
    let t = |secs: u32| (SecondOfDay(secs), SecondOfDay(secs));

    // Same route, this time with drop-off at B disabled.
    let tt = SimpleTimetable::new()
        .route(
            "L1",
            &[Stop::A, Stop::B, Stop::C],
            &[("T1", &[t(0), t(60), t(120)])],
        )
        .no_drop_off_at("T1", 1);

    let a = tt.stop_idx_of(&Stop::A);
    let b = tt.stop_idx_of(&Stop::B);
    let c = tt.stop_idx_of(&Stop::C);

    let to_c = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();
    let to_b = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();

    println!("=== no drop-off at B ===");
    println!("A → C: {} journey(s)", to_c.len());
    println!(
        "A → B: {} journey(s) (alighting at B is forbidden)",
        to_b.len()
    );
}
