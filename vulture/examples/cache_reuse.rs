//! Reusing scratch allocations across queries with [`RaptorCache`]
//! ([`RaptorCachePool`] is the `Sync` variant for thread pools).
//!
//! Server use-case: a single timetable is loaded once and many
//! queries hit it over the lifetime of the process. Each query
//! needs scratch buffers (label bags, marked-stop bitsets, route
//! queue, boarding tree). Allocating those fresh for every query is
//! wasted work; allocating one [`RaptorCache`] and finishing each
//! chain with `.run_with_cache(&mut cache)` reuses them.
//!
//! This example issues 50 queries with both shapes:
//!
//! - one-shot `.run()` (allocates internally, drops afterwards),
//! - `.run_with_cache(&mut cache)` (cache outlives the call).
//!
//! It asserts that the journeys returned are identical between
//! the two modes — the cache is purely an allocation-amortisation
//! detail, not a behaviour-changing feature. Run with
//! `cargo run --release --example cache_reuse`.

use vulture::manual::SimpleTimetable;
use vulture::{Duration, Journey, RaptorCache, SecondOfDay, Timetable};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum S {
    A,
    B,
    C,
}

fn main() {
    let tt = SimpleTimetable::new()
        .route(
            "R_AB",
            &[S::A, S::B],
            &[(
                "T_AB",
                &[
                    (SecondOfDay(0), SecondOfDay(0)),
                    (SecondOfDay(60), SecondOfDay(60)),
                ],
            )],
        )
        .route(
            "R_BC",
            &[S::B, S::C],
            &[(
                "T_BC",
                &[
                    (SecondOfDay(120), SecondOfDay(120)),
                    (SecondOfDay(180), SecondOfDay(180)),
                ],
            )],
        );

    let a = tt.stop_idx_of(&S::A);
    let c = tt.stop_idx_of(&S::C);

    // Allocate a single cache sized for this timetable. Reuse it
    // across every query in this thread.
    let mut cache: RaptorCache<vulture::ArrivalTime> = RaptorCache::for_timetable(&tt);

    let n = 50usize;
    let mut one_shot_results: Vec<Vec<Journey<vulture::ArrivalTime>>> = Vec::with_capacity(n);
    let mut cached_results: Vec<Vec<Journey<vulture::ArrivalTime>>> = Vec::with_capacity(n);

    for _ in 0..n {
        // One-shot: allocates scratch internally, drops on return.
        let one_shot = tt
            .query()
            .from(&[(a, Duration::ZERO)])
            .to(&[(c, Duration::ZERO)])
            .max_transfers(2)
            .depart_at(SecondOfDay(0))
            .run();
        one_shot_results.push(one_shot);

        // Cached: scratch lives in `cache` across iterations.
        let cached = tt
            .query()
            .from(&[(a, Duration::ZERO)])
            .to(&[(c, Duration::ZERO)])
            .max_transfers(2)
            .depart_at(SecondOfDay(0))
            .run_with_cache(&mut cache);
        cached_results.push(cached);
    }

    println!(
        "Ran {} A→C queries, one-shot vs. cached: result-set sizes match = {}",
        n,
        one_shot_results
            .iter()
            .zip(cached_results.iter())
            .all(|(o, c)| o.len() == c.len()),
    );
    if let Some(first) = one_shot_results.first().and_then(|v| v.first()) {
        println!(
            "  example journey: A → C, arrives {}, plan {} legs",
            first.label.0,
            first.plan.len(),
        );
    }

    // Behaviour must be identical: the cache amortises allocation,
    // it doesn't change what the algorithm computes.
    assert_eq!(one_shot_results.len(), cached_results.len());
    for (one, cached) in one_shot_results.iter().zip(cached_results.iter()) {
        assert_eq!(one.len(), cached.len());
        for (a_j, b_j) in one.iter().zip(cached.iter()) {
            assert_eq!(a_j.origin, b_j.origin);
            assert_eq!(a_j.target, b_j.target);
            assert_eq!(a_j.label, b_j.label);
            assert_eq!(a_j.plan, b_j.plan);
        }
    }

    // Sanity-check the journey itself: A → C requires boarding R_AB
    // (A→B), then transferring to R_BC (B→C), arriving at 180.
    let final_journey = one_shot_results
        .last()
        .and_then(|v| v.first())
        .expect("expected at least one A→C journey");
    assert_eq!(final_journey.label.0.0, 180);
    assert_eq!(final_journey.plan.len(), 2);
}
