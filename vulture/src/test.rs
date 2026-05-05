use crate::Duration;
use crate::RaptorCache;
use crate::RaptorCachePool;
use crate::SecondOfDay;
use crate::Timetable;
use crate::manual::SimpleTimetable;

macro_rules! plan {
    ($tt:expr; $(($route:expr, $stop:expr)),* $(,)?) => {
        vec![$(($tt.route_idx_of(&$route), $tt.stop_idx_of(&$stop))),*]
    };
}

/// When a faster route reaches a mid-route stop, the algorithm must record
/// that stop as the boarding stop – not the earlier stop where the route
/// scan began. See examples/reboarding.rs for the full network.
#[test]
fn reboarding_picks_correct_boarding_stop() {
    use Route::*;
    use Stop::*;
    use Trip::*;

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

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        R1T1,
        R2T1,
        R3Late,
        R3Early,
    }

    let tt = SimpleTimetable::new()
        .route(
            R1,
            &[S, A],
            &[(
                R1T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(100), SecondOfDay(100)),
                ],
            )],
        )
        .route(
            R2,
            &[S, B],
            &[(
                R2T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        )
        .route(
            R3,
            &[A, B, C, D],
            &[
                (
                    R3Late,
                    &[
                        (SecondOfDay(105), SecondOfDay(105)),
                        (SecondOfDay(110), SecondOfDay(110)),
                        (SecondOfDay(120), SecondOfDay(120)),
                        (SecondOfDay(130), SecondOfDay(130)),
                    ],
                ),
                (
                    R3Early,
                    &[
                        (SecondOfDay(25), SecondOfDay(25)),
                        (SecondOfDay(30), SecondOfDay(30)),
                        (SecondOfDay(40), SecondOfDay(40)),
                        (SecondOfDay(50), SecondOfDay(50)),
                    ],
                ),
            ],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&S), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();

    // The optimal journey: S->B via R2, then B->D via R3/Early, arriving at t=50
    assert!(!journeys.is_empty(), "should find at least one journey");

    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), SecondOfDay(50));
    assert_eq!(best.plan, plan!(tt; (R2, B), (R3, D)));
}

// ── Edge case tests ─────────────────────────────────────────────────

#[test]
fn no_journey_disconnected_graph() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(
        journeys.is_empty(),
        "disconnected graph should yield no journeys"
    );
}

#[test]
fn no_journey_missed_connection() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(50), SecondOfDay(50)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(30)),
                    (SecondOfDay(40), SecondOfDay(40)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::C), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(
        journeys.is_empty(),
        "missed connection should yield no journeys"
    );
}

#[test]
fn no_journey_late_departure() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B],
        &[(
            Trip::T1,
            &[
                (SecondOfDay(0), SecondOfDay(10)),
                (SecondOfDay(20), SecondOfDay(20)),
            ],
        )],
    );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(100))
        .run();
    assert!(
        journeys.is_empty(),
        "late departure should yield no journeys"
    );
}

#[test]
fn no_journey_transfers_zero() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B],
        &[(
            Trip::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
            ],
        )],
    );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(0)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(journeys.is_empty(), "transfers=0 should yield no journeys");
}

#[test]
fn source_equals_target() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B],
        &[(
            Trip::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
            ],
        )],
    );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(
        journeys.is_empty(),
        "source == target should yield no journeys"
    );
}

#[test]
fn direct_journey_single_route() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B, Stop::C],
        &[(
            Trip::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
                (SecondOfDay(20), SecondOfDay(20)),
            ],
        )],
    );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::C), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival(), SecondOfDay(20));
    assert_eq!(journeys[0].plan, plan!(tt; (Route::R1, Stop::C)));
}

#[test]
fn direct_journey_picks_fastest_route() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(100), SecondOfDay(100)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(50), SecondOfDay(50)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), SecondOfDay(50));
}

#[test]
fn exact_time_connection() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(20), SecondOfDay(20)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::C), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(!journeys.is_empty(), "exact-time connection should work");
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), SecondOfDay(30));
    assert_eq!(
        best.plan,
        plan!(tt; (Route::R1, Stop::B), (Route::R2, Stop::C))
    );
}

#[test]
fn multi_trip_picks_earliest_catchable() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
        T3,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B],
        &[
            (
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(5)),
                    (SecondOfDay(15), SecondOfDay(15)),
                ],
            ),
            (
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(15)),
                    (SecondOfDay(25), SecondOfDay(25)),
                ],
            ),
            (
                Trip::T3,
                &[
                    (SecondOfDay(0), SecondOfDay(25)),
                    (SecondOfDay(35), SecondOfDay(35)),
                ],
            ),
        ],
    );

    // Query at tau=12: T1 departs A@5 (too early), T2 departs A@15 (catchable)
    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(12))
        .run();
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival(), SecondOfDay(25)); // T2 arrives B@25
}

#[test]
fn two_transfer_journey() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
        T3,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(10)),
                    (SecondOfDay(20), SecondOfDay(20)),
                ],
            )],
        )
        .route(
            Route::R3,
            &[Stop::C, Stop::D],
            &[(
                Trip::T3,
                &[
                    (SecondOfDay(0), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(!journeys.is_empty());
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), SecondOfDay(30));
    assert_eq!(
        best.plan,
        plan!(tt;
            (Route::R1, Stop::B),
            (Route::R2, Stop::C),
            (Route::R3, Stop::D),
        )
    );
}

#[test]
fn pareto_optimal_fewer_transfers_vs_faster() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
        R3,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
        T3,
    }

    let tt = SimpleTimetable::new()
        // Direct slow route A→D
        .route(
            Route::R1,
            &[Stop::A, Stop::D],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(200), SecondOfDay(200)),
                ],
            )],
        )
        // Fast 2-leg: A→B via R2, B→D via R3
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(40), SecondOfDay(40)),
                ],
            )],
        )
        .route(
            Route::R3,
            &[Stop::B, Stop::D],
            &[(
                Trip::T3,
                &[
                    (SecondOfDay(0), SecondOfDay(40)),
                    (SecondOfDay(100), SecondOfDay(100)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 2, "should have 2 pareto-optimal journeys");

    let mut sorted = journeys.clone();
    sorted.sort_by_key(|j| j.arrival());
    // Faster journey (2 legs)
    assert_eq!(sorted[0].arrival(), SecondOfDay(100));
    assert_eq!(sorted[0].plan.len(), 2);
    // Slower direct journey (1 leg)
    assert_eq!(sorted[1].arrival(), SecondOfDay(200));
    assert_eq!(sorted[1].plan.len(), 1);
}

#[test]
fn footpath_enables_connection() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        )
        .footpath(Stop::B, Stop::C)
        .transfer_time(Stop::B, Stop::C, Duration(5));

    // Optimal: board R1 at A, alight at B (arr 10), walk B→C (arr 15),
    // board R2 at C (dep 20), alight at D (arr 30). Two boardings, with a
    // walk leg between them. Reconstruction traces back through the walk.
    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1, "expected one journey, got {journeys:?}");
    assert_eq!(journeys[0].arrival(), SecondOfDay(30));
    assert_eq!(
        journeys[0].plan,
        plan!(tt; (Route::R1, Stop::B), (Route::R2, Stop::D))
    );
}

#[test]
fn footpath_transfer_time_causes_miss() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(50), SecondOfDay(50)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(52)),
                    (SecondOfDay(60), SecondOfDay(60)),
                ],
            )],
        )
        .footpath(Stop::B, Stop::C)
        .transfer_time(Stop::B, Stop::C, Duration(5)); // 50+5=55 > 52

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::D), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(
        journeys.is_empty(),
        "footpath transfer time should cause miss"
    );
}

#[test]
fn early_termination_no_improvement() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        Route::R1,
        &[Stop::A, Stop::B],
        &[(
            Trip::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
            ],
        )],
    );

    let j1 = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();
    let j100 = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(100)
        .depart_at(SecondOfDay(0))
        .run();

    assert_eq!(j1.len(), j100.len());
    assert_eq!(
        j1.iter().min_by_key(|j| j.arrival()).unwrap().arrival(),
        j100.iter().min_by_key(|j| j.arrival()).unwrap().arrival(),
    );
}

#[test]
fn dominance_prunes_slower_arrival() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(50), SecondOfDay(50)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(100), SecondOfDay(100)),
                ],
            )],
        );

    let journeys = tt
        .query()
        .from(&[(tt.stop_idx_of(&Stop::A), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&Stop::B), Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    // Both routes are discovered in round 1, so the slower one is dominated
    assert_eq!(journeys.len(), 1, "dominated journey should be pruned");
    assert_eq!(journeys[0].arrival(), SecondOfDay(50));
}

#[test]
fn raptor_with_cache_matches_fresh_run() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::B],
            &[(
                Trip::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(
                Trip::T2,
                &[
                    (SecondOfDay(15), SecondOfDay(15)),
                    (SecondOfDay(25), SecondOfDay(25)),
                ],
            )],
        );

    // Run several queries with varying parameters through one cache and
    // confirm each result matches a fresh-run baseline.
    let queries = [
        (3, SecondOfDay(0), Stop::A, Stop::C),
        (1, SecondOfDay(0), Stop::A, Stop::B),
        (5, SecondOfDay(5), Stop::A, Stop::C),
        (2, SecondOfDay(0), Stop::B, Stop::C),
    ];

    let mut cache: RaptorCache = RaptorCache::for_timetable(&tt);
    for &(transfers, depart, ps, pt) in &queries {
        let ps_idx = tt.stop_idx_of(&ps);
        let pt_idx = tt.stop_idx_of(&pt);
        let baseline = tt
            .query()
            .from(&[(ps_idx, Duration::ZERO)])
            .to(&[(pt_idx, Duration::ZERO)])
            .max_transfers(transfers as u8)
            .depart_at(depart)
            .run();
        let cached = tt
            .query()
            .from(&[(ps_idx, Duration::ZERO)])
            .to(&[(pt_idx, Duration::ZERO)])
            .max_transfers(transfers as u8)
            .depart_at(depart)
            .run_with_cache(&mut cache);
        assert_eq!(
            cached.len(),
            baseline.len(),
            "journey count differs at query {ps:?}->{pt:?}"
        );
        for (b, c) in baseline.iter().zip(cached.iter()) {
            assert_eq!(b.arrival(), c.arrival());
            assert_eq!(b.plan, c.plan);
        }
    }
}

#[test]
fn multi_source_picks_best_origin() {
    // Two parallel single-trip routes. R_fast goes A->C in 10. R_slow goes
    // B->C in 30 (departs same time but the trip itself takes longer).
    // Query "from {A, B} to C" should pick the journey via A (faster).
    use Route::*;
    use Stop::*;
    use Trip::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        RFast,
        RSlow,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        TFast,
        TSlow,
    }

    let tt = SimpleTimetable::new()
        .route(
            RFast,
            &[A, C],
            &[(
                TFast,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            RSlow,
            &[B, C],
            &[(
                TSlow,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        );

    let a = tt.stop_idx_of(&A);
    let b = tt.stop_idx_of(&B);
    let c = tt.stop_idx_of(&C);

    let journeys = tt
        .query()
        .from(&[(a, Duration::ZERO), (b, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1, "one Pareto-optimal journey expected");
    assert_eq!(journeys[0].arrival(), SecondOfDay(10));
    assert_eq!(
        journeys[0].origin, a,
        "should have started at A (the faster route)"
    );
    assert_eq!(journeys[0].target, c);
    assert_eq!(journeys[0].plan, plan!(tt; (RFast, C)));
}

#[test]
fn multi_source_walk_offset_changes_best_origin() {
    // Same routes as above, but the user has a 30s walk to A and 0s to B.
    // The fast trip via A now effectively takes 30 + 10 = 40s of departure
    // delay + travel, while the slow trip via B takes 0 + 30 = 30s. So B
    // wins.
    use Route::*;
    use Stop::*;
    use Trip::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        RFast,
        RSlow,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        TFast,
        TSlow,
    }

    let tt = SimpleTimetable::new()
        // The slow trip needs to depart at 30 (so it leaves later than the
        // user is ready) for the algorithm to actually pick it cleanly.
        .route(
            RFast,
            &[A, C],
            &[(
                TFast,
                &[
                    (SecondOfDay(30), SecondOfDay(30)),
                    (SecondOfDay(40), SecondOfDay(40)),
                ],
            )],
        )
        .route(
            RSlow,
            &[B, C],
            &[(
                TSlow,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        );

    let a = tt.stop_idx_of(&A);
    let b = tt.stop_idx_of(&B);
    let c = tt.stop_idx_of(&C);

    // Walk 30s to A means the user reaches A at tau=30. By then TFast is
    // about to depart; arrival at C = 40.
    // Walk 0s to B means the user reaches B at tau=0. TSlow boards at 0,
    // arrives C at 30.
    let journeys = tt
        .query()
        .from(&[(a, Duration(30)), (b, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), SecondOfDay(30));
    assert_eq!(
        best.origin, b,
        "should have started at B (closer + slow trip wins)"
    );
}

#[test]
fn multi_target_walk_offset_picks_best_target() {
    // Two targets, T1 reachable at 10 with walk 30 (effective 40), T2
    // reachable at 25 with walk 0 (effective 25). T2 should win.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Stop {
        A,
        T1,
        T2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Route {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Trip {
        Tr1,
        Tr2,
    }

    let tt = SimpleTimetable::new()
        .route(
            Route::R1,
            &[Stop::A, Stop::T1],
            &[(
                Trip::Tr1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::T2],
            &[(
                Trip::Tr2,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(25), SecondOfDay(25)),
                ],
            )],
        );

    let a = tt.stop_idx_of(&Stop::A);
    let t1 = tt.stop_idx_of(&Stop::T1);
    let t2 = tt.stop_idx_of(&Stop::T2);

    let journeys = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(t1, Duration(30)), (t2, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    // best raw arrival at T1 = 10, +30 walk = 40 effective
    // best raw arrival at T2 = 25, +0 walk = 25 effective → wins
    assert_eq!(best.arrival(), SecondOfDay(25));
    assert_eq!(best.target, t2);
}

#[test]
fn query_builder_single_departure() {
    use crate::labels::ArrivalAndWalk;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        R::R1,
        &[S::A, S::B, S::C],
        &[(
            Tr::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
                (SecondOfDay(20), SecondOfDay(20)),
            ],
        )],
    );
    let a = tt.stop_idx_of(&S::A);
    let c = tt.stop_idx_of(&S::C);

    // 1. Bare typestate flow with all defaults except from/to/depart_at.
    let journeys = tt.query().from(a).to(c).depart_at(SecondOfDay(0)).run();
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival(), SecondOfDay(20));

    // 2. With max_transfers and a u8 literal (Into<Transfers>).
    let journeys = tt
        .query()
        .from(a)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1);

    // 3. With explicit cache (run_with_cache).
    let mut cache = crate::RaptorCache::for_timetable(&tt);
    let journeys = tt
        .query()
        .from(a)
        .to(c)
        .depart_at(SecondOfDay(0))
        .run_with_cache(&mut cache);
    assert_eq!(journeys.len(), 1);

    // 4. Custom label via query_with_label.
    let journeys: Vec<crate::Journey<ArrivalAndWalk>> = tt
        .query_with_label::<ArrivalAndWalk>()
        .from(a)
        .to(c)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].label.walk_time, Duration::ZERO);
}

#[test]
fn query_builder_range_departure() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
        T3,
    }

    let tt = SimpleTimetable::new().route(
        R::R1,
        &[S::A, S::B],
        &[
            (
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            ),
            (
                Tr::T2,
                &[
                    (SecondOfDay(10), SecondOfDay(10)),
                    (SecondOfDay(20), SecondOfDay(20)),
                ],
            ),
            (
                Tr::T3,
                &[
                    (SecondOfDay(20), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            ),
        ],
    );
    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);

    let profile = tt
        .query()
        .from(a)
        .to(b)
        .depart_in_window([
            SecondOfDay(0),
            SecondOfDay(5),
            SecondOfDay(10),
            SecondOfDay(15),
            SecondOfDay(20),
        ])
        .run();

    let mut points: Vec<(SecondOfDay, SecondOfDay)> = profile
        .iter()
        .map(|p| (p.depart, p.journey.arrival()))
        .collect();
    points.sort();
    assert_eq!(
        points,
        vec![
            (SecondOfDay(0), SecondOfDay(10)),
            (SecondOfDay(10), SecondOfDay(20)),
            (SecondOfDay(20), SecondOfDay(30))
        ]
    );
}

#[test]
fn into_endpoints_accepts_natural_input_shapes() {
    // The single-stop call is the headline ergonomics win – `start` and
    // `end` go straight in, no slice-of-tuples wrapping.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
    }

    let tt = SimpleTimetable::new().route(
        R::R1,
        &[S::A, S::B, S::C],
        &[(
            Tr::T1,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(10), SecondOfDay(10)),
                (SecondOfDay(20), SecondOfDay(20)),
            ],
        )],
    );
    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);
    let c = tt.stop_idx_of(&S::C);

    // 1. Bare StopIdx – the trivial case.
    let j = tt
        .query()
        .from(a)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0].arrival(), SecondOfDay(20));

    // 2. (StopIdx, Duration) pair as a single endpoint with explicit walk.
    //    Walk-time offset of 0 should match the bare-stop behaviour above.
    let j = tt
        .query()
        .from((a, Duration::ZERO))
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j[0].arrival(), SecondOfDay(20));

    // 3. Slice of stops (multi-source / multi-target with walk = 0).
    let stops = [a, b];
    let j = tt
        .query()
        .from(&stops[..])
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j[0].arrival(), SecondOfDay(20));

    // 4. Slice of (stop, duration) pairs – original v0.13 shape.
    let pairs = [(a, Duration::ZERO)];
    let j = tt
        .query()
        .from(&pairs[..])
        .to(&[(c, Duration::ZERO)][..])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j[0].arrival(), SecondOfDay(20));

    // 5. Owned Vec of (stop, duration) pairs.
    let owned = vec![(a, Duration::ZERO)];
    let j = tt
        .query()
        .from(&owned)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j[0].arrival(), SecondOfDay(20));

    // 6. Pre-built Endpoints.
    let mut ep = crate::Endpoints::new();
    ep.push(a, Duration::ZERO);
    let j = tt
        .query()
        .from(ep)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(j[0].arrival(), SecondOfDay(20));
}

#[test]
fn closed_path_dispatch_matches_dijkstra() {
    use crate::{RouteIdx, SecondOfDay, StopIdx, TripIdx};

    // Newtype wrapper that delegates everything to its inner timetable
    // but reports the footpath relation as transitively closed. Run the
    // same scenario through both paths and assert identical journeys.
    struct ClosedAssert<T: Timetable>(T);

    impl<T: Timetable> Timetable for ClosedAssert<T> {
        fn n_stops(&self) -> usize {
            self.0.n_stops()
        }
        fn n_routes(&self) -> usize {
            self.0.n_routes()
        }
        fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)] {
            self.0.get_routes_serving_stop(stop)
        }
        fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx] {
            self.0.get_stops_after(route, pos)
        }
        fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx {
            self.0.stop_at(route, pos)
        }
        fn get_earliest_trip(&self, route: RouteIdx, at: SecondOfDay, pos: u32) -> Option<TripIdx> {
            self.0.get_earliest_trip(route, at, pos)
        }
        fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay {
            self.0.get_arrival_time(trip, pos)
        }
        fn get_departure_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay {
            self.0.get_departure_time(trip, pos)
        }
        fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
            self.0.get_footpaths_from(stop)
        }
        fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Duration {
            self.0.get_transfer_time(from, to)
        }
        fn footpaths_are_transitively_closed(&self) -> bool {
            true
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
    }

    // A→B by R1, walk B→C, C→D by R2. The walk is a single direct edge
    // so closure is trivially satisfied – both paths must agree.
    let inner = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::B],
            &[(
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            R::R2,
            &[S::C, S::D],
            &[(
                Tr::T2,
                &[
                    (SecondOfDay(0), SecondOfDay(15)),
                    (SecondOfDay(25), SecondOfDay(25)),
                ],
            )],
        )
        .footpath(S::B, S::C)
        .transfer_time(S::B, S::C, Duration(3));

    let a = inner.stop_idx_of(&S::A);
    let d = inner.stop_idx_of(&S::D);

    let dijkstra = inner
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(d, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    let closed = ClosedAssert(inner)
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();

    assert_eq!(dijkstra.len(), closed.len(), "journey count must match");
    for (a, b) in dijkstra.iter().zip(closed.iter()) {
        assert_eq!(a.arrival(), b.arrival(), "arrival must match");
        assert_eq!(a.plan, b.plan, "plan must match");
    }
}

#[test]
fn arrival_and_walk_label_tracks_accumulated_walk_time() {
    use crate::Duration;
    use crate::RaptorCache;
    use crate::labels::ArrivalAndWalk;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
    }

    // A→B by R1 (arr 10), walk B→C 7s, then C→D... actually just B→C:
    // we use a R2 from C to test. Walk B→C is 7s; final journey uses
    // 7s of walking.
    let tt = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::B],
            &[(
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            R::R2,
            &[S::C],
            &[(Tr::T2, &[(SecondOfDay(20), SecondOfDay(20))])],
        )
        .footpath(S::B, S::C)
        .transfer_time(S::B, S::C, Duration(7));

    let a = tt.stop_idx_of(&S::A);
    let c = tt.stop_idx_of(&S::C);

    // Default ArrivalTime path
    let default_journeys = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert!(!default_journeys.is_empty());
    assert_eq!(default_journeys[0].arrival(), SecondOfDay(17)); // 10 + 7

    // Public ArrivalAndWalk via the labeled query builder.
    let label_journeys: Vec<crate::Journey<ArrivalAndWalk>> = tt
        .query_with_label::<ArrivalAndWalk>()
        .from(a)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(label_journeys.len(), default_journeys.len());
    assert_eq!(label_journeys[0].arrival(), SecondOfDay(17));
    assert_eq!(
        label_journeys[0].label.walk_time,
        Duration(7),
        "walk time tracked"
    );

    // Same via the labeled builder + cache.
    let mut cache = RaptorCache::<ArrivalAndWalk>::for_timetable(&tt);
    let cached = tt
        .query_with_label::<ArrivalAndWalk>()
        .from(a)
        .to(c)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run_with_cache(&mut cache);
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].label.walk_time, Duration(7));
}

#[test]
fn raptor_range_returns_pareto_profile_across_departures() {
    // R1 has three departures: T1 (0->10), T2 (10->20), T3 (20->30).
    // Querying departures [0, 5, 10, 15, 20]:
    // - depart=0  catches T1, arr=10
    // - depart=5  catches T2, arr=20 (no choice but to wait)
    // - depart=10 catches T2, arr=20 – strictly better than depart=5
    //   (later departure, same arrival) → depart=5 dominated, dropped.
    // - depart=15 catches T3, arr=30 (must wait)
    // - depart=20 catches T3, arr=30 – better than depart=15, drop it.
    // Profile: [(0, arr 10), (10, arr 20), (20, arr 30)].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
        T3,
    }

    let tt = SimpleTimetable::new().route(
        R::R1,
        &[S::A, S::B],
        &[
            (
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            ),
            (
                Tr::T2,
                &[
                    (SecondOfDay(10), SecondOfDay(10)),
                    (SecondOfDay(20), SecondOfDay(20)),
                ],
            ),
            (
                Tr::T3,
                &[
                    (SecondOfDay(20), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            ),
        ],
    );

    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);

    let profile = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(3)
        .depart_in_window([
            SecondOfDay(0),
            SecondOfDay(5),
            SecondOfDay(10),
            SecondOfDay(15),
            SecondOfDay(20),
        ])
        .run();

    let mut points: Vec<(crate::SecondOfDay, crate::SecondOfDay)> = profile
        .iter()
        .map(|p| (p.depart, p.journey.arrival()))
        .collect();
    points.sort();

    assert_eq!(
        points,
        vec![
            (SecondOfDay(0), SecondOfDay(10)),
            (SecondOfDay(10), SecondOfDay(20)),
            (SecondOfDay(20), SecondOfDay(30))
        ]
    );
}

#[test]
fn arrival_and_walk_returns_pareto_front() {
    // Two routes reach two intermediate stops; both stops walk to the
    // target with different walk times. Path via X is faster but with
    // more walking; path via Y is slower but with less walking. Neither
    // dominates the other on (arrival, walk_time), so an
    // `ArrivalAndWalk` query should return both. An `ArrivalTime`
    // query returns only the arrival-min path.
    use crate::labels::ArrivalAndWalk;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        X,
        Y,
        T,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
    }

    // R1 arrives X at t=10; walk X->T is 5s   → (arr 15, walk 5).
    // R2 arrives Y at t=20; walk Y->T is 1s   → (arr 21, walk 1).
    let tt = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::X],
            &[(
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            R::R2,
            &[S::A, S::Y],
            &[(
                Tr::T2,
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

    let a = tt.stop_idx_of(&S::A);
    let t = tt.stop_idx_of(&S::T);

    // ArrivalTime: only the arrival-min path survives.
    let arrival_only = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(t, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(arrival_only.len(), 1);
    assert_eq!(arrival_only[0].arrival(), SecondOfDay(15));

    // ArrivalAndWalk: both Pareto-incomparable journeys are returned.
    let pareto: Vec<crate::Journey<ArrivalAndWalk>> = tt
        .query_with_label::<ArrivalAndWalk>()
        .from(a)
        .to(t)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(
        pareto.len(),
        2,
        "expected Pareto front of two journeys, got {}: {:?}",
        pareto.len(),
        pareto.iter().map(|j| j.label).collect::<Vec<_>>(),
    );
    let mut labels: Vec<_> = pareto.iter().map(|j| j.label).collect();
    labels.sort_by_key(|l| l.arrival);
    assert_eq!(labels[0].arrival, SecondOfDay(15));
    assert_eq!(labels[0].walk_time, Duration(5));
    assert_eq!(labels[1].arrival, SecondOfDay(21));
    assert_eq!(labels[1].walk_time, Duration(1));
}

#[test]
fn with_timing_recovers_per_leg_trip_and_times() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1Early,
        T1Late,
        T2,
    }

    // R1 has two trips: T1Early (0->10) and T1Late (50->60). R2 has one
    // trip from B to C (12->25). A query departing at 0 should ride
    // T1Early (boarding 0, alighting 10), then T2 (boarding 12, alighting
    // 25). Departing at 30 should ride T1Late (boarding 50, alighting 60)
    // – there's no T2 trip available afterwards, so no journey.
    let tt = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::B],
            &[
                (
                    Tr::T1Early,
                    &[
                        (SecondOfDay(0), SecondOfDay(0)),
                        (SecondOfDay(10), SecondOfDay(10)),
                    ],
                ),
                (
                    Tr::T1Late,
                    &[
                        (SecondOfDay(50), SecondOfDay(50)),
                        (SecondOfDay(60), SecondOfDay(60)),
                    ],
                ),
            ],
        )
        .route(
            R::R2,
            &[S::B, S::C],
            &[(
                Tr::T2,
                &[
                    (SecondOfDay(12), SecondOfDay(12)),
                    (SecondOfDay(25), SecondOfDay(25)),
                ],
            )],
        );

    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);
    let c = tt.stop_idx_of(&S::C);

    let journeys = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(c, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1);
    let j = &journeys[0];
    assert_eq!(j.arrival(), SecondOfDay(25));

    let timed = j
        .with_timing(&tt, SecondOfDay(0), Duration::ZERO)
        .expect("plan must reconstruct");
    assert_eq!(timed.len(), 2);

    // Leg 1: A -> B on R1 catching T1Early.
    let l1 = &timed[0];
    assert_eq!(l1.route, tt.route_idx_of(&R::R1));
    assert_eq!(l1.board, a);
    assert_eq!(l1.alight, b);
    assert_eq!(l1.trip, tt.trip_idx_of(&Tr::T1Early));
    assert_eq!(l1.depart, SecondOfDay(0));
    assert_eq!(l1.arrive, SecondOfDay(10));

    // Leg 2: B -> C on R2 catching T2.
    let l2 = &timed[1];
    assert_eq!(l2.route, tt.route_idx_of(&R::R2));
    assert_eq!(l2.board, b);
    assert_eq!(l2.alight, c);
    assert_eq!(l2.trip, tt.trip_idx_of(&Tr::T2));
    assert_eq!(l2.depart, SecondOfDay(12));
    assert_eq!(l2.arrive, SecondOfDay(25));
}

#[test]
fn with_timing_handles_one_hop_walking_transfer() {
    // Journey: ride R1 from A to B (arrive 10), walk B->C (5s), ride R2
    // from C to D (depart 20, arrive 30). The plan is [(R1, B), (R2, D)];
    // with_timing should detect that C is a one-hop walk neighbour of B
    // serving R2 and use C as leg 2's `board`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
    }

    let tt = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::B],
            &[(
                Tr::T1,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(10), SecondOfDay(10)),
                ],
            )],
        )
        .route(
            R::R2,
            &[S::C, S::D],
            &[(
                Tr::T2,
                &[
                    (SecondOfDay(20), SecondOfDay(20)),
                    (SecondOfDay(30), SecondOfDay(30)),
                ],
            )],
        )
        .footpath(S::B, S::C)
        .transfer_time(S::B, S::C, Duration(5));

    let a = tt.stop_idx_of(&S::A);
    let b = tt.stop_idx_of(&S::B);
    let c = tt.stop_idx_of(&S::C);
    let d = tt.stop_idx_of(&S::D);

    let journeys = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(d, Duration::ZERO)])
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();
    assert_eq!(journeys.len(), 1);
    let j = &journeys[0];

    let timed = j
        .with_timing(&tt, SecondOfDay(0), Duration::ZERO)
        .expect("plan must reconstruct");
    assert_eq!(timed.len(), 2);

    let l1 = &timed[0];
    assert_eq!(l1.alight, b, "leg 1 alights at B");

    let l2 = &timed[1];
    assert_eq!(l2.board, c, "leg 2 boards at C, not B (walking transfer)");
    assert_eq!(l2.alight, d);
    assert_eq!(l2.depart, SecondOfDay(20));
    assert_eq!(l2.arrive, SecondOfDay(30));
}

// ── RaptorCachePool tests ───────────────────────────────────────────

/// Trivial 4-stop, 2-route timetable used by the pool tests below.
fn pool_test_timetable() -> SimpleTimetable<char, u32, u32> {
    SimpleTimetable::new()
        .route(
            1,
            &['A', 'B', 'C'],
            &[(
                10,
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(60), SecondOfDay(60)),
                    (SecondOfDay(120), SecondOfDay(120)),
                ],
            )],
        )
        .route(
            2,
            &['C', 'D'],
            &[(
                20,
                &[
                    (SecondOfDay(150), SecondOfDay(150)),
                    (SecondOfDay(210), SecondOfDay(210)),
                ],
            )],
        )
}

#[test]
fn pool_checkout_matches_single_cache_result() {
    let tt = pool_test_timetable();
    let a = tt.stop_idx_of(&'A');
    let d = tt.stop_idx_of(&'D');

    let baseline = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run();

    let pool = RaptorCachePool::for_timetable(&tt);
    let mut cache = pool.checkout();
    let pooled = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run_with_cache(&mut cache);

    assert_eq!(baseline.len(), pooled.len());
    for (a, b) in baseline.iter().zip(&pooled) {
        assert_eq!(a.plan, b.plan);
        assert_eq!(a.arrival(), b.arrival());
    }
}

#[test]
fn pool_reuses_caches_across_checkouts() {
    let tt = pool_test_timetable();
    let pool: RaptorCachePool = RaptorCachePool::for_timetable(&tt);

    // Pool starts empty.
    assert!(format!("{:?}", pool).contains("idle_caches: 0"));

    {
        let _c = pool.checkout(); // allocates fresh
    }
    // Cache returned on drop.
    assert!(format!("{:?}", pool).contains("idle_caches: 1"));

    {
        let _c1 = pool.checkout(); // reuses returned cache
        let _c2 = pool.checkout(); // pool now empty, allocates a second
    }
    // Both returned.
    assert!(format!("{:?}", pool).contains("idle_caches: 2"));
}

#[test]
fn pool_is_sync_across_threads() {
    let tt = pool_test_timetable();
    let a = tt.stop_idx_of(&'A');
    let d = tt.stop_idx_of(&'D');
    let pool: RaptorCachePool = RaptorCachePool::for_timetable(&tt);

    let baseline_arrival = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_at(SecondOfDay(0))
        .run()
        .into_iter()
        .map(|j| j.arrival())
        .collect::<Vec<_>>();

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let pool = &pool;
            let tt = &tt;
            let baseline = &baseline_arrival;
            handles.push(s.spawn(move || {
                let mut cache = pool.checkout();
                let arrivals = tt
                    .query()
                    .from(a)
                    .to(d)
                    .max_transfers(3)
                    .depart_at(SecondOfDay(0))
                    .run_with_cache(&mut cache)
                    .into_iter()
                    .map(|j| j.arrival())
                    .collect::<Vec<_>>();
                assert_eq!(&arrivals, baseline);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    });

    // After 8 threads, the pool has at least 1 idle cache (could have up to
    // 8 if all ran in parallel, fewer if some serialised).
    let idle_caches = format!("{:?}", pool);
    assert!(
        !idle_caches.contains("idle_caches: 0"),
        "pool should have returned caches after threads finished: {}",
        idle_caches
    );
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_range_query_matches_serial() {
    let tt = pool_test_timetable();
    let a = tt.stop_idx_of(&'A');
    let d = tt.stop_idx_of(&'D');

    let departures: Vec<SecondOfDay> = (0..=120).step_by(15).map(SecondOfDay).collect();

    let serial = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_in_window(departures.iter().copied())
        .run();

    let parallel = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_in_window(departures.iter().copied())
        .run_par();

    assert_eq!(serial.len(), parallel.len(), "front sizes differ");
    // The Pareto front filter sorts deterministically by (later depart,
    // fewer transfers, earlier arrival), so output ordering matches.
    for (s, p) in serial.iter().zip(&parallel) {
        assert_eq!(s.depart, p.depart);
        assert_eq!(s.journey.plan, p.journey.plan);
        assert_eq!(s.journey.arrival(), p.journey.arrival());
    }

    // run_with_pool should match too, with reusable pool.
    let pool = RaptorCachePool::for_timetable(&tt);
    let parallel_pooled = tt
        .query()
        .from(a)
        .to(d)
        .max_transfers(3)
        .depart_in_window(departures.iter().copied())
        .run_with_pool(&pool);
    assert_eq!(serial.len(), parallel_pooled.len());
    for (s, p) in serial.iter().zip(&parallel_pooled) {
        assert_eq!(s.depart, p.depart);
        assert_eq!(s.journey.plan, p.journey.plan);
    }
}

#[test]
fn newly_active_stops_marks_only_in_window() {
    use crate::algorithm::range::newly_active_stops_into;
    use fixedbitset::FixedBitSet;

    // Two routes, each with three trips at distinct departures.
    // Route 1 stops: A, B (trips depart A at 100, 200, 300).
    // Route 2 stops: C, D (trips depart C at 150, 250, 350).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum S {
        A,
        B,
        C,
        D,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum R {
        R1,
        R2,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Tr {
        T1,
        T2,
        T3,
        T4,
        T5,
        T6,
    }

    let tt = SimpleTimetable::new()
        .route(
            R::R1,
            &[S::A, S::B],
            &[
                (
                    Tr::T1,
                    &[
                        (SecondOfDay(100), SecondOfDay(100)),
                        (SecondOfDay(110), SecondOfDay(110)),
                    ],
                ),
                (
                    Tr::T2,
                    &[
                        (SecondOfDay(200), SecondOfDay(200)),
                        (SecondOfDay(210), SecondOfDay(210)),
                    ],
                ),
                (
                    Tr::T3,
                    &[
                        (SecondOfDay(300), SecondOfDay(300)),
                        (SecondOfDay(310), SecondOfDay(310)),
                    ],
                ),
            ],
        )
        .route(
            R::R2,
            &[S::C, S::D],
            &[
                (
                    Tr::T4,
                    &[
                        (SecondOfDay(150), SecondOfDay(150)),
                        (SecondOfDay(160), SecondOfDay(160)),
                    ],
                ),
                (
                    Tr::T5,
                    &[
                        (SecondOfDay(250), SecondOfDay(250)),
                        (SecondOfDay(260), SecondOfDay(260)),
                    ],
                ),
                (
                    Tr::T6,
                    &[
                        (SecondOfDay(350), SecondOfDay(350)),
                        (SecondOfDay(360), SecondOfDay(360)),
                    ],
                ),
            ],
        );

    let mut marked = FixedBitSet::with_capacity(tt.n_stops());

    // Window [180, 270): R1's T2 (200) and R2's T5 (250) qualify.
    // Stops marked: A and B (R1 reaches both via T2), C and D (R2 reaches both via T5).
    // T2 also has B at 210 and T5 has D at 260 – both fall in the window.
    newly_active_stops_into(&tt, SecondOfDay(180), SecondOfDay(270), &mut marked);
    assert!(marked.contains(tt.stop_idx_of(&S::A).idx()));
    assert!(marked.contains(tt.stop_idx_of(&S::B).idx()));
    assert!(marked.contains(tt.stop_idx_of(&S::C).idx()));
    assert!(marked.contains(tt.stop_idx_of(&S::D).idx()));

    // Empty window: nothing changes.
    let mut marked2 = FixedBitSet::with_capacity(tt.n_stops());
    newly_active_stops_into(&tt, SecondOfDay(500), SecondOfDay(500), &mut marked2);
    assert_eq!(marked2.count_ones(..), 0);

    // Window with no qualifying trip: nothing changes.
    let mut marked3 = FixedBitSet::with_capacity(tt.n_stops());
    newly_active_stops_into(&tt, SecondOfDay(120), SecondOfDay(140), &mut marked3);
    assert_eq!(marked3.count_ones(..), 0);

    // Boundary: dep == lo is included (lower bound is inclusive).
    // Window [200, 250): T2 has A=200 (hits lo, included) and B=210 (in window),
    // so A and B are marked. T5's C=250 hits hi exactly and is excluded; T4's
    // C=150 is below lo. Hence C and D must NOT be marked.
    let mut marked4 = FixedBitSet::with_capacity(tt.n_stops());
    newly_active_stops_into(&tt, SecondOfDay(200), SecondOfDay(250), &mut marked4);
    assert!(marked4.contains(tt.stop_idx_of(&S::A).idx()));
    assert!(marked4.contains(tt.stop_idx_of(&S::B).idx()));
    assert!(!marked4.contains(tt.stop_idx_of(&S::C).idx()));
    assert!(!marked4.contains(tt.stop_idx_of(&S::D).idx()));

    // Boundary: dep == hi is excluded (upper bound is exclusive).
    // Window [300, 350): T3 has A=300 (hits lo, included) and B=310 (in window),
    // so A and B are marked. T6's C=350 hits hi exactly and is excluded; T5's
    // C=250 is below lo. Hence C and D must NOT be marked.
    let mut marked5 = FixedBitSet::with_capacity(tt.n_stops());
    newly_active_stops_into(&tt, SecondOfDay(300), SecondOfDay(350), &mut marked5);
    assert!(marked5.contains(tt.stop_idx_of(&S::A).idx()));
    assert!(marked5.contains(tt.stop_idx_of(&S::B).idx()));
    assert!(!marked5.contains(tt.stop_idx_of(&S::C).idx()));
    assert!(!marked5.contains(tt.stop_idx_of(&S::D).idx()));

    // Preserves existing bits: pre-set A, then call with an empty window so
    // the helper performs no work. The pre-existing bit must still be set
    // afterwards (the helper only ever calls `insert`, never clears).
    let mut marked6 = FixedBitSet::with_capacity(tt.n_stops());
    marked6.insert(tt.stop_idx_of(&S::A).idx());
    newly_active_stops_into(&tt, SecondOfDay(500), SecondOfDay(500), &mut marked6);
    assert!(marked6.contains(tt.stop_idx_of(&S::A).idx()));
}
