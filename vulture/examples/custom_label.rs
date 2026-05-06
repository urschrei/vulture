//! End-to-end custom [`Label`]: define your own multi-criterion
//! label with a sidecar [`Label::Ctx`], build the lookup table, run
//! a query, and inspect the Pareto front.
//!
//! The label tracks `(arrival_time, worst_route_score_along_journey)`.
//! Each route has a "badness" score (lower = better — more reliable,
//! less crowded, more scenic, whatever the operator's domain calls
//! for); the algorithm returns a Pareto front of the fastest journey
//! (which may sit through the worst route) and any slower journeys
//! that avoid worse routes.
//!
//! Topology — two parallel routes from `A` and a footpath stage to
//! the target:
//! ```text
//!   route R_fast: A --10s--> X         (score 5, "unpleasant")
//!   route R_slow: A --20s--> Y         (score 1, "pleasant")
//!   footpath:    X --5s walk--> T
//!   footpath:    Y --1s walk--> T
//! ```
//! Two Pareto-non-dominated journeys reach `T`:
//! `(arr 15, worst 5)` via `R_fast + walk`, and
//! `(arr 21, worst 1)` via `R_slow + walk`. Run with
//! `cargo run --release --example custom_label`.

use std::collections::HashMap;

use vulture::manual::SimpleTimetable;
use vulture::{Duration, Label, RouteIdx, SecondOfDay, StopIdx, Timetable, TripContext};

/// Per-route preference scores. Default = empty map (every route
/// scores zero). Borrowed immutably by every label callback.
#[derive(Default, Debug, Clone)]
struct RouteScores(HashMap<RouteIdx, u32>);

/// Two-criterion label: arrival time + worst score encountered.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct ArrivalAndWorstScore {
    arrival: SecondOfDay,
    worst: u32,
}

impl Label for ArrivalAndWorstScore {
    type Ctx = RouteScores;
    const UNREACHED: Self = Self {
        arrival: SecondOfDay::MAX,
        worst: 0,
    };

    fn from_departure(_ctx: &Self::Ctx, at: SecondOfDay) -> Self {
        Self {
            arrival: at,
            worst: 0,
        }
    }

    fn extend_by_trip(self, ctx: &Self::Ctx, leg: TripContext) -> Self {
        let score = ctx.0.get(&leg.route).copied().unwrap_or(0);
        Self {
            arrival: leg.arrival,
            worst: self.worst.max(score),
        }
    }

    fn extend_by_footpath(
        self,
        _ctx: &Self::Ctx,
        _from_stop: StopIdx,
        _to_stop: StopIdx,
        walk: Duration,
    ) -> Self {
        Self {
            arrival: self.arrival + walk,
            worst: self.worst,
        }
    }

    fn dominates(&self, other: &Self) -> bool {
        self.arrival <= other.arrival && self.worst <= other.worst
    }

    fn arrival(&self) -> SecondOfDay {
        self.arrival
    }
}

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
            "R_fast",
            &[S::A, S::X],
            &[(
                "T_fast",
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            "R_slow",
            &[S::A, S::Y],
            &[(
                "T_slow",
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

    // Build the per-route score table once. Borrowed by every label
    // callback for the duration of the query.
    let mut scores = HashMap::new();
    scores.insert(tt.route_idx_of(&"R_fast"), 5);
    scores.insert(tt.route_idx_of(&"R_slow"), 1);

    let journeys = tt
        .query_with_label::<ArrivalAndWorstScore>()
        .with_context(RouteScores(scores))
        .from(tt.stop_idx_of(&S::A))
        .to(tt.stop_idx_of(&S::T))
        .max_transfers(2)
        .depart_at(SecondOfDay(0))
        .run();

    println!("Pareto front from A to T:");
    for j in &journeys {
        println!(
            "  arrives {}, worst route score {} (plan: {} legs)",
            j.label.arrival,
            j.label.worst,
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
            .any(|j| j.label.arrival.0 == 15 && j.label.worst == 5),
        "missing fast/unpleasant journey",
    );
    assert!(
        journeys
            .iter()
            .any(|j| j.label.arrival.0 == 21 && j.label.worst == 1),
        "missing slow/pleasant journey",
    );
}
