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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&S), tt.stop_idx_of(&D));

    // The optimal journey: S->B via R2, then B->D via R3/Early, arriving at t=50
    assert!(!journeys.is_empty(), "should find at least one journey");

    let best = journeys.iter().min_by_key(|j| j.arrival).unwrap();
    assert_eq!(best.arrival, 50);
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::D));
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::C));
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

    let journeys = tt.raptor(3, 100, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
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

    let journeys = tt.raptor(0, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::A));
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::C));
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival, 20);
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
    let best = journeys.iter().min_by_key(|j| j.arrival).unwrap();
    assert_eq!(best.arrival, 50);
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::C));
    assert!(!journeys.is_empty(), "exact-time connection should work");
    let best = journeys.iter().min_by_key(|j| j.arrival).unwrap();
    assert_eq!(best.arrival, 30);
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
    let journeys = tt.raptor(3, 12, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
    assert_eq!(journeys.len(), 1);
    assert_eq!(journeys[0].arrival, 25); // T2 arrives B@25
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::D));
    assert!(!journeys.is_empty());
    let best = journeys.iter().min_by_key(|j| j.arrival).unwrap();
    assert_eq!(best.arrival, 30);
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::D));
    assert_eq!(journeys.len(), 2, "should have 2 pareto-optimal journeys");

    let mut sorted = journeys.clone();
    sorted.sort_by_key(|j| j.arrival);
    // Faster journey (2 legs)
    assert_eq!(sorted[0].arrival, 100);
    assert_eq!(sorted[0].plan.len(), 2);
    // Slower direct journey (1 leg)
    assert_eq!(sorted[1].arrival, 200);
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
    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::D));
    assert_eq!(journeys.len(), 1, "expected one journey, got {journeys:?}");
    assert_eq!(journeys[0].arrival, 30);
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::D));
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

    let j1 = tt.raptor(1, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
    let j100 = tt.raptor(100, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));

    assert_eq!(j1.len(), j100.len());
    assert_eq!(
        j1.iter().min_by_key(|j| j.arrival).unwrap().arrival,
        j100.iter().min_by_key(|j| j.arrival).unwrap().arrival,
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

    let journeys = tt.raptor(3, 0, tt.stop_idx_of(&Stop::A), tt.stop_idx_of(&Stop::B));
    // Both routes are discovered in round 1, so the slower one is dominated
    assert_eq!(journeys.len(), 1, "dominated journey should be pruned");
    assert_eq!(journeys[0].arrival, 50);
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
        let baseline = tt.raptor(transfers, tau, ps_idx, pt_idx);
        let cached = tt.raptor_with_cache(&mut cache, transfers, tau, ps_idx, pt_idx);
        assert_eq!(
            cached.len(),
            baseline.len(),
            "journey count differs at query {ps:?}->{pt:?}"
        );
        for (b, c) in baseline.iter().zip(cached.iter()) {
            assert_eq!(b.arrival, c.arrival);
            assert_eq!(b.plan, c.plan);
        }
    }
}
