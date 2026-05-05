use crate::RaptorCache;
use crate::Timetable;
use crate::simple::SimpleTimetable;

macro_rules! plan {
    ($tt:expr; $(($route:expr, $stop:expr)),* $(,)?) => {
        vec![$(($tt.route_idx_of(&$route), $tt.stop_idx_of(&$stop))),*]
    };
}

/// When a faster route reaches a mid-route stop, the algorithm must record
/// that stop as the boarding stop — not the earlier stop where the route
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
        .route(R1, &[S, A], &[(R1T1, &[(0, 0), (100, 100)])])
        .route(R2, &[S, B], &[(R2T1, &[(0, 0), (30, 30)])])
        .route(
            R3,
            &[A, B, C, D],
            &[
                (R3Late, &[(105, 105), (110, 110), (120, 120), (130, 130)]),
                (R3Early, &[(25, 25), (30, 30), (40, 40), (50, 50)]),
            ],
        );

    let journeys = tt.raptor(3, 0, &[(tt.stop_idx_of(&S), 0)], &[(tt.stop_idx_of(&D), 0)]);

    // The optimal journey: S->B via R2, then B->D via R3/Early, arriving at t=50
    assert!(!journeys.is_empty(), "should find at least one journey");

    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), 50);
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
            &[(Trip::T1, &[(0, 0), (10, 10)])],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(Trip::T2, &[(0, 0), (10, 10)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::D), 0)],
    );
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
            &[(Trip::T1, &[(0, 0), (50, 50)])],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(Trip::T2, &[(0, 30), (40, 40)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::C), 0)],
    );
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
        &[(Trip::T1, &[(0, 10), (20, 20)])],
    );

    let journeys = tt.raptor(
        3,
        100,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
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
        &[(Trip::T1, &[(0, 0), (10, 10)])],
    );

    let journeys = tt.raptor(
        0,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
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
        &[(Trip::T1, &[(0, 0), (10, 10)])],
    );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::A), 0)],
    );
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
        &[(Trip::T1, &[(0, 0), (10, 10), (20, 20)])],
    );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::C), 0)],
    );
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival(), 20);
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
            &[(Trip::T1, &[(0, 0), (100, 100)])],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(Trip::T2, &[(0, 0), (50, 50)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), 50);
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
            &[(Trip::T1, &[(0, 0), (20, 20)])],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(Trip::T2, &[(0, 20), (30, 30)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::C), 0)],
    );
    assert!(!journeys.is_empty(), "exact-time connection should work");
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), 30);
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
            (Trip::T1, &[(0, 5), (15, 15)]),
            (Trip::T2, &[(0, 15), (25, 25)]),
            (Trip::T3, &[(0, 25), (35, 35)]),
        ],
    );

    // Query at tau=12: T1 departs A@5 (too early), T2 departs A@15 (catchable)
    let journeys = tt.raptor(
        3,
        12,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival(), 25); // T2 arrives B@25
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
            &[(Trip::T1, &[(0, 0), (10, 10)])],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(Trip::T2, &[(0, 10), (20, 20)])],
        )
        .route(
            Route::R3,
            &[Stop::C, Stop::D],
            &[(Trip::T3, &[(0, 20), (30, 30)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::D), 0)],
    );
    assert!(!journeys.is_empty());
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), 30);
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
            &[(Trip::T1, &[(0, 0), (200, 200)])],
        )
        // Fast 2-leg: A→B via R2, B→D via R3
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(Trip::T2, &[(0, 0), (40, 40)])],
        )
        .route(
            Route::R3,
            &[Stop::B, Stop::D],
            &[(Trip::T3, &[(0, 40), (100, 100)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::D), 0)],
    );
    assert_eq!(journeys.len(), 2, "should have 2 pareto-optimal journeys");

    let mut sorted = journeys.clone();
    sorted.sort_by_key(|j| j.arrival());
    // Faster journey (2 legs)
    assert_eq!(sorted[0].arrival(), 100);
    assert_eq!(sorted[0].plan.len(), 2);
    // Slower direct journey (1 leg)
    assert_eq!(sorted[1].arrival(), 200);
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
            &[(Trip::T1, &[(0, 0), (10, 10)])],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(Trip::T2, &[(0, 20), (30, 30)])],
        )
        .footpath(Stop::B, Stop::C)
        .transfer_time(Stop::B, Stop::C, 5);

    // Optimal: board R1 at A, alight at B (arr 10), walk B→C (arr 15),
    // board R2 at C (dep 20), alight at D (arr 30). Two boardings, with a
    // walk leg between them. Reconstruction traces back through the walk.
    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::D), 0)],
    );
    assert_eq!(journeys.len(), 1, "expected one journey, got {journeys:?}");
    assert_eq!(journeys[0].arrival(), 30);
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
            &[(Trip::T1, &[(0, 0), (50, 50)])],
        )
        .route(
            Route::R2,
            &[Stop::C, Stop::D],
            &[(Trip::T2, &[(0, 52), (60, 60)])],
        )
        .footpath(Stop::B, Stop::C)
        .transfer_time(Stop::B, Stop::C, 5); // 50+5=55 > 52

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::D), 0)],
    );
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
        &[(Trip::T1, &[(0, 0), (10, 10)])],
    );

    let j1 = tt.raptor(
        1,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
    let j100 = tt.raptor(
        100,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );

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
            &[(Trip::T1, &[(0, 0), (50, 50)])],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::B],
            &[(Trip::T2, &[(0, 0), (100, 100)])],
        );

    let journeys = tt.raptor(
        3,
        0,
        &[(tt.stop_idx_of(&Stop::A), 0)],
        &[(tt.stop_idx_of(&Stop::B), 0)],
    );
    // Both routes are discovered in round 1, so the slower one is dominated
    assert_eq!(journeys.len(), 1, "dominated journey should be pruned");
    assert_eq!(journeys[0].arrival(), 50);
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
            &[(Trip::T1, &[(0, 0), (10, 10)])],
        )
        .route(
            Route::R2,
            &[Stop::B, Stop::C],
            &[(Trip::T2, &[(15, 15), (25, 25)])],
        );

    // Run several queries with varying parameters through one cache and
    // confirm each result matches a fresh-run baseline.
    let queries = [
        (3, 0, Stop::A, Stop::C),
        (1, 0, Stop::A, Stop::B),
        (5, 5, Stop::A, Stop::C),
        (2, 0, Stop::B, Stop::C),
    ];

    let mut cache: RaptorCache = RaptorCache::for_timetable(&tt);
    for &(transfers, tau, ps, pt) in &queries {
        let ps_idx = tt.stop_idx_of(&ps);
        let pt_idx = tt.stop_idx_of(&pt);
        let baseline = tt.raptor(transfers, tau, &[(ps_idx, 0)], &[(pt_idx, 0)]);
        let cached =
            tt.raptor_with_cache(&mut cache, transfers, tau, &[(ps_idx, 0)], &[(pt_idx, 0)]);
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
        .route(RFast, &[A, C], &[(TFast, &[(0, 0), (10, 10)])])
        .route(RSlow, &[B, C], &[(TSlow, &[(0, 0), (30, 30)])]);

    let a = tt.stop_idx_of(&A);
    let b = tt.stop_idx_of(&B);
    let c = tt.stop_idx_of(&C);

    let journeys = tt.raptor(3, 0, &[(a, 0), (b, 0)], &[(c, 0)]);
    assert_eq!(journeys.len(), 1, "one Pareto-optimal journey expected");
    assert_eq!(journeys[0].arrival(), 10);
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
        .route(RFast, &[A, C], &[(TFast, &[(30, 30), (40, 40)])])
        .route(RSlow, &[B, C], &[(TSlow, &[(0, 0), (30, 30)])]);

    let a = tt.stop_idx_of(&A);
    let b = tt.stop_idx_of(&B);
    let c = tt.stop_idx_of(&C);

    // Walk 30s to A means the user reaches A at tau=30. By then TFast is
    // about to depart; arrival at C = 40.
    // Walk 0s to B means the user reaches B at tau=0. TSlow boards at 0,
    // arrives C at 30.
    let journeys = tt.raptor(3, 0, &[(a, 30), (b, 0)], &[(c, 0)]);
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    assert_eq!(best.arrival(), 30);
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
            &[(Trip::Tr1, &[(0, 0), (10, 10)])],
        )
        .route(
            Route::R2,
            &[Stop::A, Stop::T2],
            &[(Trip::Tr2, &[(0, 0), (25, 25)])],
        );

    let a = tt.stop_idx_of(&Stop::A);
    let t1 = tt.stop_idx_of(&Stop::T1);
    let t2 = tt.stop_idx_of(&Stop::T2);

    let journeys = tt.raptor(3, 0, &[(a, 0)], &[(t1, 30), (t2, 0)]);
    let best = journeys.iter().min_by_key(|j| j.arrival()).unwrap();
    // best raw arrival at T1 = 10, +30 walk = 40 effective
    // best raw arrival at T2 = 25, +0 walk = 25 effective → wins
    assert_eq!(best.arrival(), 25);
    assert_eq!(best.target, t2);
}

#[test]
fn closed_path_dispatch_matches_dijkstra() {
    use crate::{RouteIdx, StopIdx, Tau, TripIdx};

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
        fn get_earliest_trip(&self, route: RouteIdx, at: Tau, pos: u32) -> Option<TripIdx> {
            self.0.get_earliest_trip(route, at, pos)
        }
        fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> Tau {
            self.0.get_arrival_time(trip, pos)
        }
        fn get_departure_time(&self, trip: TripIdx, pos: u32) -> Tau {
            self.0.get_departure_time(trip, pos)
        }
        fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
            self.0.get_footpaths_from(stop)
        }
        fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
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
    // so closure is trivially satisfied — both paths must agree.
    let inner = SimpleTimetable::new()
        .route(R::R1, &[S::A, S::B], &[(Tr::T1, &[(0, 0), (10, 10)])])
        .route(R::R2, &[S::C, S::D], &[(Tr::T2, &[(0, 15), (25, 25)])])
        .footpath(S::B, S::C)
        .transfer_time(S::B, S::C, 3);

    let a = inner.stop_idx_of(&S::A);
    let d = inner.stop_idx_of(&S::D);

    let dijkstra = inner.raptor(3, 0, &[(a, 0)], &[(d, 0)]);
    let closed = ClosedAssert(inner).raptor(3, 0, &[(a, 0)], &[(d, 0)]);

    assert_eq!(dijkstra.len(), closed.len(), "journey count must match");
    for (a, b) in dijkstra.iter().zip(closed.iter()) {
        assert_eq!(a.arrival(), b.arrival(), "arrival must match");
        assert_eq!(a.plan, b.plan, "plan must match");
    }
}

#[test]
fn custom_label_tracks_accumulated_walk_time() {
    use crate::{Label, RaptorCache, Tau};

    // Custom Label that piggybacks the accumulated walking time
    // alongside arrival time. The single-label-per-stop algorithm
    // still picks by arrival, so walk_time is just carried along —
    // but it ends up correctly accumulated on the journey output.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    struct ArrivalAndWalk {
        arrival: Tau,
        walk_time: Tau,
    }

    impl Label for ArrivalAndWalk {
        const UNREACHED: Self = ArrivalAndWalk {
            arrival: Tau::MAX,
            walk_time: 0,
        };

        fn from_departure(tau: Tau) -> Self {
            ArrivalAndWalk {
                arrival: tau,
                walk_time: 0,
            }
        }

        fn extend_by_trip(self, arrival_tau: Tau) -> Self {
            ArrivalAndWalk {
                arrival: arrival_tau,
                walk_time: self.walk_time,
            }
        }

        fn extend_by_footpath(self, walk: Tau) -> Self {
            ArrivalAndWalk {
                arrival: self.arrival.saturating_add(walk),
                walk_time: self.walk_time.saturating_add(walk),
            }
        }

        fn arrival(&self) -> Tau {
            self.arrival
        }
    }

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
        .route(R::R1, &[S::A, S::B], &[(Tr::T1, &[(0, 0), (10, 10)])])
        .route(R::R2, &[S::C], &[(Tr::T2, &[(20, 20)])])
        .footpath(S::B, S::C)
        .transfer_time(S::B, S::C, 7);

    let a = tt.stop_idx_of(&S::A);
    let c = tt.stop_idx_of(&S::C);

    // Default ArrivalTime path
    let default_journeys = tt.raptor(3, 0, &[(a, 0)], &[(c, 0)]);
    assert!(!default_journeys.is_empty());
    assert_eq!(default_journeys[0].arrival(), 17); // 10 + 7

    // Custom label path via raptor_with_label.
    let label_journeys: Vec<crate::Journey<ArrivalAndWalk>> =
        tt.raptor_with_label::<ArrivalAndWalk>(3, 0, &[(a, 0)], &[(c, 0)]);
    assert_eq!(label_journeys.len(), default_journeys.len());
    assert_eq!(label_journeys[0].arrival(), 17);
    assert_eq!(label_journeys[0].label.walk_time, 7, "walk time tracked");

    // Same via raptor_with_cache_and_label (cache infers L).
    let mut cache = RaptorCache::<ArrivalAndWalk>::for_timetable(&tt);
    let cached = tt.raptor_with_cache_and_label(&mut cache, 3, 0, &[(a, 0)], &[(c, 0)]);
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].label.walk_time, 7);
}
