//! Range query: "leave between X and Y, what are my options?"
//!
//! `Query::depart_in_window(times)` runs RAPTOR across multiple
//! departure times and returns a 3-D Pareto profile filtered on
//! (later depart, fewer transfers, earlier arrival). The serial
//! path uses rRAPTOR (specialised for `ArrivalTime`); `.run_par()`
//! fans per-departure work across Rayon and is order-insensitive.
//!
//! This example builds a single A → B route with a trip every two
//! minutes, samples a 10-minute window of departures, and asserts
//! that:
//!
//! 1. Both serial and parallel paths return the same set.
//! 2. Each returned journey catches the next available trip after
//!    its declared `depart` time.
//!
//! Run with `cargo run --release --example range_query`.

use vulture::manual::SimpleTimetable;
use vulture::{SecondOfDay, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum S {
    A,
    B,
}

fn main() {
    // Five trips, each taking 60 seconds A → B, departing every
    // 120s starting at 09:00:00.
    let trips: Vec<(u16, Vec<(SecondOfDay, SecondOfDay)>)> = (0..5)
        .map(|i| {
            let dep = SecondOfDay::hms(9, 0, 0) + vulture::Duration(i * 120);
            let arr = dep + vulture::Duration(60);
            (i as u16, vec![(dep, dep), (arr, arr)])
        })
        .collect();

    // SimpleTimetable's `route(...)` wants `&[(T, &[(SecondOfDay, SecondOfDay)])]`,
    // so reshape to slice-of-pair-of-slices borrow.
    let trip_args: Vec<(u16, &[(SecondOfDay, SecondOfDay)])> = trips
        .iter()
        .map(|(t, sched)| (*t, sched.as_slice()))
        .collect();
    let tt = SimpleTimetable::new().route(0u8, &[S::A, S::B], &trip_args);

    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);

    // Sample three departure times spaced 60s apart starting at 08:59:30.
    // The middle one (09:00:30) and the latest (09:01:30) both catch
    // the 09:02 trip and arrive at 09:03; the Pareto filter keeps the
    // later-departing one (09:01:30) and drops 09:00:30, so the
    // profile has 2 entries rather than 3.
    let departures: Vec<SecondOfDay> = (0..3)
        .map(|i| SecondOfDay::hms(8, 59, 30) + vulture::Duration(i * 60))
        .collect();

    // ── Serial rRAPTOR ───────────────────────────────────────────────
    let serial = tt
        .query()
        .from(a)
        .to(b)
        .max_transfers(1)
        .depart_in_window(departures.iter().copied())
        .run();

    // ── Parallel naïve batch ─────────────────────────────────────────
    let parallel = tt
        .query()
        .from(a)
        .to(b)
        .max_transfers(1)
        .depart_in_window(departures.iter().copied())
        .run_par();

    println!("Serial rRAPTOR ({} entries):", serial.len());
    for rj in &serial {
        println!("  depart {}, arrive {}", rj.depart, rj.journey.arrival(),);
    }
    println!("Parallel naive ({} entries):", parallel.len());
    for rj in &parallel {
        println!("  depart {}, arrive {}", rj.depart, rj.journey.arrival(),);
    }

    // Both paths should return the same Pareto profile.
    let mut s_keys: Vec<(u32, u32)> = serial
        .iter()
        .map(|rj| (rj.depart.0, rj.journey.arrival().0))
        .collect();
    let mut p_keys: Vec<(u32, u32)> = parallel
        .iter()
        .map(|rj| (rj.depart.0, rj.journey.arrival().0))
        .collect();
    s_keys.sort();
    p_keys.sort();
    assert_eq!(
        s_keys, p_keys,
        "serial and parallel range queries should agree",
    );

    // Each entry should arrive 60s after the next trip departure
    // ≥ entry.depart. Trips depart at 09:00, 09:02, 09:04, …
    for rj in &serial {
        let dep = rj.depart.0;
        let next_trip_dep = (dep.div_ceil(120) * 120).max(SecondOfDay::hms(9, 0, 0).0);
        assert_eq!(
            rj.journey.arrival().0,
            next_trip_dep + 60,
            "expected to catch the next trip after depart {dep}",
        );
    }
}
