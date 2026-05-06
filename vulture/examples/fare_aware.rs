//! Fare-aware Pareto routing using the bundled
//! [`vulture::labels::ArrivalAndFare`] and a [`FareTable`] context.
//! Demonstrates the two-step recipe for any Ctx-bearing label:
//! build the lookup table once, supply it via
//! [`vulture::Query::with_context`], then run the query.
//!
//! Topology — same X/Y intermediate-stop shape as `custom_label.rs`,
//! but with the bundled fare label instead of a hand-rolled one:
//! ```text
//!   route R_express:  A --10s--> X    (premium fare 500)
//!   route R_local:    A --20s--> Y    (cheap fare 100)
//!   footpath X --5s--> T
//!   footpath Y --1s--> T
//! ```
//! Pareto front: `(arr 15, fare 500)` via the express + walk and
//! `(arr 21, fare 100)` via the local + walk. Run with
//! `cargo run --release --example fare_aware`.

use std::collections::HashMap;

use vulture::labels::{ArrivalAndFare, FareTable};
use vulture::manual::SimpleTimetable;
use vulture::{Duration, Journey, SecondOfDay, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum S {
    A,
    X,
    Y,
    T,
}

fn main() {
    let tt = SimpleTimetable::new()
        .route(
            "R_express",
            &[S::A, S::X],
            &[(
                "T_express",
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            "R_local",
            &[S::A, S::Y],
            &[(
                "T_local",
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(20), SecondOfDay(20)),
                ],
            )],
        )
        .footpath(S::X, S::T)
        .transfer_time(S::X, S::T, Duration(5))
        .footpath(S::Y, S::T)
        .transfer_time(S::Y, S::T, Duration(1));

    // Build the fare table from the route IDs you intern in the
    // timetable. In a real GTFS feed you'd populate this from
    // fare_attributes.txt / fare_rules.txt; here we hardwire two
    // routes for the example.
    let mut per_route = HashMap::new();
    per_route.insert(tt.route_idx_of(&"R_express"), 500u32);
    per_route.insert(tt.route_idx_of(&"R_local"), 100u32);
    let fares = FareTable { per_route };

    let journeys: Vec<Journey<ArrivalAndFare>> = tt
        .query_with_label::<ArrivalAndFare>()
        .with_context(fares)
        .from(tt.stop_idx_of(&S::A))
        .to(tt.stop_idx_of(&S::T))
        .max_transfers(2)
        .depart_at(SecondOfDay(0))
        .run();

    println!("Pareto front from A to T (arrival vs. fare):");
    for j in &journeys {
        println!(
            "  arrives {}, fare {} (plan: {} legs)",
            j.label.arrival,
            j.label.fare,
            j.plan.len(),
        );
    }

    assert_eq!(
        journeys.len(),
        2,
        "expected a 2-journey Pareto front, got {journeys:#?}",
    );
    assert!(
        journeys
            .iter()
            .any(|j| j.label.arrival.0 == 15 && j.label.fare == 500),
        "missing express journey",
    );
    assert!(
        journeys
            .iter()
            .any(|j| j.label.arrival.0 == 21 && j.label.fare == 100),
        "missing local journey",
    );
}
