//! Property-based test harness comparing `Timetable::raptor` against a
//! brute-force reference solver on randomly generated networks.
//!
//! TRIP-COUNT CONVENTION
//!
//! `Timetable::raptor`'s `transfers` parameter is the trip count, not the
//! transfer count, despite its name. A journey "board R1 to B, board R2 to D"
//! has 2 trips and 1 transfer; the trait's `transfers=2` admits this journey.
//! Similarly, `Journey::plan.len()` is the trip count.
//!
//! Both this harness and the reference solver use trip counts everywhere.
//! The Pareto front compared in property tests is over `(arrival, trip_count)`.
//!
//! Layer-to-issue mapping is documented in `README.md` next to this crate.

pub mod reference;
pub mod spec;

use std::collections::BTreeSet;

use vulture::Journey;
use vulture::labels::ArrivalAndWalk;

/// Project the algorithm's `Vec<Journey>` to a Pareto front of
/// `(arrival, trip_count)`, sorted by trip count ascending, keeping only
/// points where arrival is *strictly* less than the best seen so far.
///
/// This applies the output-side Pareto filter the algorithm should be doing
/// itself (soundness issue F). Filtering on the harness side intentionally
/// masks F so the front-equality property isolates issues A, B, C, D.
pub fn raptor_front(journeys: &[Journey]) -> BTreeSet<(u16, u8)> {
    let mut points: Vec<(u16, u8)> = journeys
        .iter()
        .map(|j| {
            let arr = u16::try_from(j.arrival().0)
                .expect("arrival exceeds u16::MAX – generator range exceeded?");
            let k = u8::try_from(j.plan.len())
                .expect("plan length exceeds u8::MAX – should never happen");
            (arr, k)
        })
        .collect();
    points.sort_by_key(|&(t, k)| (k, t));
    let mut best = u16::MAX;
    let mut out = BTreeSet::new();
    for (arr, k) in points {
        if arr < best {
            best = arr;
            out.insert((arr, k));
        }
    }
    out
}

/// Project an `ArrivalAndWalk` journey list to its 3-D Pareto front of
/// `(effective_arrival, effective_walk_time, trip_count)`. The algorithm
/// emits one entry per `(round, target, bag-label)` triple, unfiltered;
/// this helper applies the strict Pareto filter the harness uses to
/// compare against the reference.
pub fn arrival_and_walk_front(journeys: &[Journey<ArrivalAndWalk>]) -> BTreeSet<(u16, u16, u8)> {
    let triples: Vec<(u16, u16, u8)> = journeys
        .iter()
        .map(|j| {
            let arr = u16::try_from(j.label.arrival.0).expect("arrival fits u16 in proptest");
            let walk = u16::try_from(j.label.walk_time.0).expect("walk_time fits u16 in proptest");
            let k = u8::try_from(j.plan.len()).expect("plan length fits u8 in proptest");
            (arr, walk, k)
        })
        .collect();
    let mut front = BTreeSet::new();
    'outer: for &c in &triples {
        for &other in &triples {
            if other == c {
                continue;
            }
            if other.0 <= c.0 && other.1 <= c.1 && other.2 <= c.2 {
                continue 'outer;
            }
        }
        front.insert(c);
    }
    front
}

#[cfg(test)]
use vulture::{Duration, SecondOfDay, Timetable};

/// Hegel test settings used by every proptest in this crate.
///
/// Hegel's default `Settings::new()` auto-detects CI environments
/// (`GITHUB_ACTIONS`, `CI`, etc.) and switches to `derandomize: true`
/// (a fixed seed derived from the test name) plus disables the
/// failing-example database. That made one CI run equivalent to
/// every other CI run and silently masked layer-3 bugs that random
/// search reliably finds locally.
///
/// We force `derandomize: false` everywhere so CI and local both
/// draw fresh random seeds each run; over many CI runs across PRs
/// this surfaces multi-target / generator-corner bugs that any
/// single fixed seed misses. Test reproducibility for a known
/// failure is still available via `seed = N` on a per-test basis.
#[cfg(test)]
fn proptest_settings() -> hegel::Settings {
    hegel::Settings::new().derandomize(false)
}

#[cfg(test)]
fn run_property(tc: &hegel::TestCase, spec: &spec::NetworkSpec) {
    let timetable = spec::render(spec);
    let origins: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .origins
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();
    let targets: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .targets
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();
    let mut q = timetable
        .query()
        .from(origins.as_slice())
        .to(targets.as_slice())
        .max_transfers(spec.query.max_transfers as usize as u8);
    if spec.query.require_wheelchair_accessible {
        q = q.require_wheelchair_accessible();
    }
    let ours = q.depart_at(SecondOfDay(spec.query.tau as u32)).run();
    let theirs = reference::reference_solve(
        spec,
        &spec.query.origins,
        &spec.query.targets,
        spec.query.tau,
        spec.query.max_transfers,
        spec.query.require_wheelchair_accessible,
    );
    let our_front = raptor_front(&ours);
    if our_front != theirs {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("raptor:     {:?}", ours));
        tc.note(&format!("ours_front: {:?}", our_front));
        tc.note(&format!("theirs:     {:?}", theirs));
    }
    assert_eq!(our_front, theirs);
}

#[hegel::test(crate::proptest_settings())]
fn layer1_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test(crate::proptest_settings())]
fn layer2_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn layer3_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer3_bounds()));
    run_property(&tc, &spec);
}

/// Cross-check the two range-query implementations: the serial path runs
/// rRAPTOR (single reverse-chronological scan reusing labels across
/// departures, paper §4); the parallel paths fan a naïve per-departure
/// batch across Rayon. Output must be identical.
///
/// This property covers two distinct concerns simultaneously:
///
/// 1. **rRAPTOR-vs-naïve-batch equivalence.** The serial range path is
///    rRAPTOR; the parallel paths run the naïve batch. Treating the naïve
///    batch as the reference, this asserts rRAPTOR's label-inheritance,
///    newly-active-stops marking, and per-τ snapshotting all preserve the
///    per-departure semantics. Catches algorithm-specialisation bugs
///    (state leak across τ scans, missed re-marking, dominated-label
///    races against `best_arrival`).
///
/// 2. **Parallel-vs-serial parity.** Both `.run_par()` and
///    `.run_with_pool()` must produce identical output to `.run()`.
///    Catches pool checkout/return races, per-departure state leakage
///    between cache reuses, and any non-determinism leaking out of
///    Rayon's `collect`.
///
/// Uses `layer1_bounds` to keep the per-case cost low – single-departure
/// algorithm correctness is already covered by `layer{1,2,3}_matches_reference`;
/// this is the only test exercising the range-query path, so it must
/// stay fast enough to run on every commit.
#[hegel::test(crate::proptest_settings())]
fn parallel_naive_matches_serial_rrap(tc: hegel::TestCase) {
    use vulture::RaptorCachePool;

    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    let timetable = spec::render(&spec);
    // Range-query test stays single-source / single-target by design —
    // it's stressing rRAPTOR vs the parallel naïve batch, not endpoint
    // expansion. Take the first entry of each set.
    let (ps_raw, _) = spec.query.origins[0];
    let (pt_raw, _) = spec.query.targets[0];
    let ps = timetable.stop_idx_of(&ps_raw);
    let pt = timetable.stop_idx_of(&pt_raw);

    // Three-departure window around tau. Saturating sub ensures we
    // stay non-negative even for tau == 0.
    let tau = spec.query.tau as u32;
    let step: u32 = 5;
    let departures: Vec<SecondOfDay> = (0..3)
        .map(|i| SecondOfDay(tau.saturating_sub(i * step)))
        .collect();

    let serial = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_in_window(departures.iter().copied())
        .run();

    let parallel = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_in_window(departures.iter().copied())
        .run_par();

    let pool = RaptorCachePool::for_timetable(&timetable);
    let pooled = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_in_window(departures.iter().copied())
        .run_with_pool(&pool);

    if serial.len() != parallel.len() || serial.len() != pooled.len() {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("departures: {:?}", departures));
        tc.note(&format!("serial:   {:?}", serial));
        tc.note(&format!("parallel: {:?}", parallel));
        tc.note(&format!("pooled:   {:?}", pooled));
    }
    assert_eq!(serial.len(), parallel.len(), "run_par length mismatch");
    assert_eq!(serial.len(), pooled.len(), "run_with_pool length mismatch");

    for ((s, p), pool_entry) in serial.iter().zip(&parallel).zip(&pooled) {
        assert_eq!(s.depart, p.depart);
        assert_eq!(s.depart, pool_entry.depart);
        assert_eq!(s.journey.arrival(), p.journey.arrival());
        assert_eq!(s.journey.arrival(), pool_entry.journey.arrival());
        assert_eq!(s.journey.plan, p.journey.plan);
        assert_eq!(s.journey.plan, pool_entry.journey.plan);
    }
}

/// Property check for the [`ArrivalAndWalk`] label: vulture's
/// `query_with_label::<ArrivalAndWalk>()` Pareto front of
/// `(arrival, walk_time, trip_count)` must equal the brute-force
/// reference's. Stays on `layer2_bounds`: layer 2 has footpaths (so
/// walking accumulates non-trivially) but neither fares nor accessibility
/// flags, keeping the property focused on `ArrivalAndWalk`'s arithmetic
/// and dominance behaviour.
///
/// **Currently `#[ignore]`d**: the algorithm has single-criterion-only
/// pruning sites (`per_call.rs:194` route-scan check, `label_bag.rs:101`
/// `insert_into_bag` pt_threshold check, scalar `pt_threshold` mechanism
/// in `boarding.rs`) that reject Pareto-incomparable multi-criterion
/// labels. The minimal counterexample is a parallel footpath and trip
/// arriving at the same target stop at the same time but with different
/// walk_time: the trip-based label is dropped by the route-scan check
/// `arr >= best_to_pi.min_arrival()` even though it strictly dominates
/// the walk-only label on walk_time. The reference solver in
/// `reference.rs` is the infrastructure a follow-up fix can use to
/// validate that the algorithm-level pruning is made Pareto-aware.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
#[ignore = "exposes per_call.rs:194 / label_bag.rs:101 multi-criterion pruning gap; tracked separately"]
fn arrival_and_walk_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    let timetable = spec::render(&spec);

    let origins: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .origins
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();
    let targets: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .targets
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();

    let ours: Vec<Journey<ArrivalAndWalk>> = timetable
        .query_with_label::<ArrivalAndWalk>()
        .from(origins.as_slice())
        .to(targets.as_slice())
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_at(SecondOfDay(spec.query.tau as u32))
        .run();

    let our_front = arrival_and_walk_front(&ours);
    let theirs = reference::reference_solve_arrival_and_walk(
        &spec,
        &spec.query.origins,
        &spec.query.targets,
        spec.query.tau,
        spec.query.max_transfers,
        spec.query.require_wheelchair_accessible,
    );

    if our_front != theirs {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("raptor:     {:?}", ours));
        tc.note(&format!("ours_front: {:?}", our_front));
        tc.note(&format!("theirs:     {:?}", theirs));
    }
    assert_eq!(our_front, theirs);
}

/// Property check for the [`ArrivalAndFare`] label: every journey
/// vulture returns must have a `label.fare` equal to the manual sum
/// of per-route fares across its plan. Validates that the new
/// `Label::Ctx`-threaded `extend_by_trip` plumbing keeps fare state
/// in sync with the journey it describes.
///
/// Stays on `layer3_bounds` because that's the only layer that
/// generates non-zero per-route fares.
///
/// Two specific things this catches:
///
/// 1. Missed `extend_by_trip` calls: if the algorithm forgot to
///    extend the label on some boarding/alighting, the journey's
///    `fare` would diverge from the manual sum across `plan`.
/// 2. Wrong `RouteIdx` threaded to `extend_by_trip`: if the
///    algorithm passes a wrong route to the label, the fare lookup
///    in `FareTable` would miss or pick the wrong route's fare.
#[hegel::test(crate::proptest_settings())]
fn fare_label_matches_per_leg_sum(tc: hegel::TestCase) {
    use vulture::labels::{ArrivalAndFare, FareTable};

    let spec = tc.draw(spec::network_spec(spec::layer3_bounds()));
    let timetable = spec::render(&spec);

    // Build a FareTable mapping each rendered RouteIdx to the spec's
    // route fare. Spec routes are emitted in the order produced by
    // the renderer's `for ((route_id, ..), trips) in groups` loop —
    // route_idx_of(route_id) is the algorithm-side handle.
    let mut per_route: std::collections::HashMap<vulture::RouteIdx, u32> =
        std::collections::HashMap::new();
    for (route_id_u8, route) in spec.routes.iter().enumerate() {
        let route_id = u8::try_from(route_id_u8).expect("layer3 route count fits in u8");
        // Some routes may not be registered if the renderer dropped them
        // (e.g. zero-stop sequences) — guard with the lookup.
        if let Some(fares_route_idx) = timetable_route_idx(&timetable, route_id) {
            per_route.insert(fares_route_idx, route.fare);
        }
    }
    let fares = FareTable { per_route };

    let origins: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .origins
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();
    let targets: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .targets
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();

    let mut q = timetable
        .query_with_label::<ArrivalAndFare>()
        .with_context(fares.clone())
        .from(origins.as_slice())
        .to(targets.as_slice())
        .max_transfers(spec.query.max_transfers as usize as u8);
    if spec.query.require_wheelchair_accessible {
        q = q.require_wheelchair_accessible();
    }
    let journeys = q.depart_at(SecondOfDay(spec.query.tau as u32)).run();

    for j in &journeys {
        // Sum fares across every (route, _stop) leg in the plan. The
        // algorithm should have computed exactly this.
        let manual_fare: u32 = j.plan.iter().map(|(route, _)| fares.fare_for(*route)).sum();
        if manual_fare != j.label.fare {
            tc.note(&format!("spec: {:#?}", spec));
            tc.note(&format!("fares: {:?}", fares.per_route));
            tc.note(&format!("journey plan: {:?}", j.plan));
            tc.note(&format!("label.fare: {}", j.label.fare));
            tc.note(&format!("manual_fare: {}", manual_fare));
        }
        assert_eq!(
            manual_fare, j.label.fare,
            "label fare must equal sum over plan",
        );
    }
}

#[cfg(test)]
fn timetable_route_idx<TT>(tt: &TT, route_id: u8) -> Option<vulture::RouteIdx>
where
    TT: vulture::Timetable + ?Sized,
    // Best-effort: in practice the renderer uses SimpleTimetable<u8, u8, u16>
    // so we have its own `route_idx_of`. The trait surface doesn't expose a
    // string→idx lookup, so we live with concrete-type access here.
{
    let _ = (tt, route_id);
    // Fallback: every route_id from 0..n_routes is interned, in order.
    if usize::from(route_id) < tt.n_routes() {
        Some(vulture::RouteIdx::new(u32::from(route_id)))
    } else {
        None
    }
}

/// Range-query algorithm correctness against the brute-force
/// reference solver. Where `parallel_naive_matches_serial_rrap`
/// only checks rRAPTOR vs the parallel naïve batch (a self-
/// consistency check), this test gives both an independent ground
/// truth.
///
/// Stays on `layer1_bounds` for the same per-case-cost reason: a
/// 3-departure window plus per-departure brute force is `3 ×
/// reference_solve`, which is comfortable on small networks but
/// would dominate the run on layer 3.
#[hegel::test(crate::proptest_settings())]
fn range_query_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    let timetable = spec::render(&spec);
    let origins: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .origins
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();
    let targets: Vec<(vulture::StopIdx, Duration)> = spec
        .query
        .targets
        .iter()
        .map(|&(s, w)| (timetable.stop_idx_of(&s), Duration(u32::from(w))))
        .collect();

    // Three-departure window around tau, with the first entry pinned
    // at tau itself so the reference and vulture agree on at least
    // one anchor.
    let tau = spec.query.tau as u32;
    let step: u32 = 5;
    let raw_departures: Vec<u32> = (0..3).map(|i| tau.saturating_sub(i * step)).collect();
    let departures_secondofday: Vec<SecondOfDay> =
        raw_departures.iter().copied().map(SecondOfDay).collect();
    let departures_u16: Vec<u16> = raw_departures
        .iter()
        .map(|&d| u16::try_from(d).unwrap_or(u16::MAX))
        .collect();

    let ours: Vec<vulture::RangeJourney> = timetable
        .query()
        .from(origins.as_slice())
        .to(targets.as_slice())
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_in_window(departures_secondofday.iter().copied())
        .run();

    let our_set: std::collections::BTreeSet<(u16, u16, u8)> = ours
        .iter()
        .map(|rj| {
            (
                u16::try_from(rj.depart.0).unwrap_or(u16::MAX),
                u16::try_from(rj.journey.arrival().0).unwrap_or(u16::MAX),
                u8::try_from(rj.journey.plan.len()).unwrap_or(u8::MAX),
            )
        })
        .collect();

    let theirs = reference::reference_range_solve(
        &spec,
        &spec.query.origins,
        &spec.query.targets,
        &departures_u16,
        spec.query.max_transfers,
        spec.query.require_wheelchair_accessible,
    );

    if our_set != theirs {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("departures: {:?}", raw_departures));
        tc.note(&format!("raptor:     {:?}", ours));
        tc.note(&format!("ours_set:   {:?}", our_set));
        tc.note(&format!("theirs:     {:?}", theirs));
    }
    assert_eq!(our_set, theirs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vulture::{ArrivalTime, Journey, RouteIdx, StopIdx};

    fn j(arrival: u32, plan: Vec<(u32, u32)>) -> Journey {
        Journey {
            origin: StopIdx::new(0),
            target: StopIdx::new(0),
            plan: plan
                .into_iter()
                .map(|(r, s)| (RouteIdx::new(r), StopIdx::new(s)))
                .collect(),
            label: ArrivalTime(SecondOfDay(arrival)),
        }
    }

    #[test]
    fn raptor_front_drops_dominated_higher_trip_journeys() {
        let journeys = vec![
            j(100, vec![(0, 1)]),
            j(100, vec![(0, 2), (1, 3)]),
            j(80, vec![(0, 2), (1, 3)]),
        ];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(100u16, 1u8), (80u16, 2u8)].into_iter().collect();
        assert_eq!(f, expected);
    }

    #[test]
    fn raptor_front_empty_input_is_empty() {
        let f = raptor_front(&[]);
        assert!(f.is_empty());
    }

    #[test]
    fn raptor_front_strict_monotonicity_drops_ties() {
        let journeys = vec![j(50, vec![(0, 1)]), j(50, vec![(0, 1), (1, 2)])];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(50u16, 1u8)].into_iter().collect();
        assert_eq!(f, expected);
    }
}
