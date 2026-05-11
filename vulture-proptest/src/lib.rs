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

/// Project the algorithm's `Vec<Journey>` to a per-`(target_stop,
/// target_walk)` Pareto front of `(arrival, trip_count)`. Each unique
/// `(target_stop, target_walk)` slot in the query's targets list is its
/// own destination from the user's perspective; journeys to different
/// slots are not directly comparable. Within each slot, journeys are
/// sorted by trip count ascending and only points whose arrival is
/// strictly less than the best seen so far within the slot are kept.
///
/// Output tuple: `(target_stop, target_walk_secs, arrival, trip_count)`.
///
/// `arrival` is a `u16` because the reference solver in `reference.rs` is
/// u16-native throughout, and the two outputs must share a type to be
/// comparable. The generator currently caps `tau` and trip times well
/// below `u16::MAX`; if you ever widen those, widen this and the
/// reference solver's `u16` time type together — otherwise the
/// `u16::try_from` below will panic on the first oversized arrival.
pub fn raptor_front(journeys: &[Journey]) -> BTreeSet<(u32, u32, u16, u8)> {
    let mut by_slot: std::collections::BTreeMap<(u32, u32), Vec<(u16, u8)>> =
        std::collections::BTreeMap::new();
    for j in journeys {
        let arr = u16::try_from(j.arrival().0)
            .expect("arrival exceeds u16::MAX – generator range exceeded?");
        let k =
            u8::try_from(j.plan.len()).expect("plan length exceeds u8::MAX – should never happen");
        by_slot
            .entry((j.target.get(), j.target_walk.0))
            .or_default()
            .push((arr, k));
    }
    let mut out = BTreeSet::new();
    for ((stop, walk), mut points) in by_slot {
        points.sort_by_key(|&(t, k)| (k, t));
        let mut best = u16::MAX;
        for (arr, k) in points {
            if arr < best {
                best = arr;
                out.insert((stop, walk, arr, k));
            }
        }
    }
    out
}

/// Project an `ArrivalAndWalk` journey list to a per-`(target_stop,
/// target_walk)` 3-D Pareto front of `(effective_arrival,
/// effective_walk_time, trip_count)`. Each `(target_stop, target_walk)`
/// slot in the query's targets list is its own destination; journeys
/// to different slots are not directly comparable.
///
/// Output tuple: `(target_stop, target_walk_secs, arrival, walk_time,
/// trip_count)`.
pub fn arrival_and_walk_front(
    journeys: &[Journey<ArrivalAndWalk>],
) -> BTreeSet<(u32, u32, u16, u16, u8)> {
    type Triple = (u16, u16, u8);
    let mut by_slot: std::collections::BTreeMap<(u32, u32), Vec<Triple>> =
        std::collections::BTreeMap::new();
    for j in journeys {
        let arr = u16::try_from(j.label.arrival.0).expect("arrival fits u16 in proptest");
        let walk = u16::try_from(j.label.walk_time.0).expect("walk_time fits u16 in proptest");
        let k = u8::try_from(j.plan.len()).expect("plan length fits u8 in proptest");
        by_slot
            .entry((j.target.get(), j.target_walk.0))
            .or_default()
            .push((arr, walk, k));
    }
    let mut front = BTreeSet::new();
    for ((stop, walk_off), triples) in by_slot {
        'outer: for &c in &triples {
            for &other in &triples {
                if other == c {
                    continue;
                }
                if other.0 <= c.0 && other.1 <= c.1 && other.2 <= c.2 {
                    continue 'outer;
                }
            }
            front.insert((stop, walk_off, c.0, c.1, c.2));
        }
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
        .max_transfers(spec.query.max_transfers);
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

#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn layer1_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn layer2_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn layer3_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer3_bounds()));
    run_property(&tc, &spec);
}

/// Builder idempotence: registering every footpath a second time via
/// `SimpleTimetable::footpath()` should not change the user-visible query
/// result. The builder pushes unconditionally into the from-stop's
/// adjacency list (`manual/mod.rs::footpath`), so a duplicate call leaves
/// a duplicate `StopIdx` in `footpaths[a_idx]`. RAPTOR's footpath
/// relaxation iterates that list; duplicates there should be redundant
/// but harmless. This property surfaces that — if duplicates ever change
/// the output, we want to know.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn duplicate_footpaths_preserve_query_result(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    let single = spec::render(&spec);
    let double = double_footpaths(&spec);

    let run = |tt: &vulture::manual::SimpleTimetable<u8, u8, u16>| {
        let origins: Vec<(vulture::StopIdx, Duration)> = spec
            .query
            .origins
            .iter()
            .map(|&(s, w)| (tt.stop_idx_of(&s), Duration(u32::from(w))))
            .collect();
        let targets: Vec<(vulture::StopIdx, Duration)> = spec
            .query
            .targets
            .iter()
            .map(|&(s, w)| (tt.stop_idx_of(&s), Duration(u32::from(w))))
            .collect();
        let journeys = tt
            .query()
            .from(origins.as_slice())
            .to(targets.as_slice())
            .max_transfers(spec.query.max_transfers)
            .depart_at(SecondOfDay(spec.query.tau as u32))
            .run();
        raptor_front(&journeys)
    };

    let single_front = run(&single);
    let double_front = run(&double);

    if single_front != double_front {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("single front: {:?}", single_front));
        tc.note(&format!("double front: {:?}", double_front));
    }
    assert_eq!(single_front, double_front);
}

#[cfg(test)]
fn double_footpaths(spec: &spec::NetworkSpec) -> vulture::manual::SimpleTimetable<u8, u8, u16> {
    // Re-render the spec, then layer a second `.footpath()` call on top
    // of every transitively-closed edge — leaving each from-stop's
    // adjacency list with a duplicate StopIdx.
    let mut tt = spec::render(spec);
    let closed = spec::close_footpaths(spec);
    for from in 0..spec.n_stops {
        for to in 0..spec.n_stops {
            if from == to {
                continue;
            }
            if closed[from as usize][to as usize].is_some() {
                tt = tt.footpath(from, to);
            }
        }
    }
    tt
}

/// Robustness: the algorithm must not panic for any `depart_at` value in
/// the full `u32` range — including `0`, `u32::MAX`, and values past the
/// largest trip departure in the network. No reference solver: this is
/// pure no-crash coverage of the algorithm's time arithmetic, since the
/// main suite caps `tau` at 500 via the generator. Real GTFS feeds reach
/// 86 399 (one second before midnight) and our internal time type is
/// `u32`, so the algorithm needs to be defined across the whole range.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn extreme_tau_does_not_panic(tc: hegel::TestCase) {
    use hegel::generators;

    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    let timetable = spec::render(&spec);
    let tau = tc.draw(generators::integers::<u32>());

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

    let _journeys = timetable
        .query()
        .from(origins.as_slice())
        .to(targets.as_slice())
        .max_transfers(spec.query.max_transfers)
        .depart_at(SecondOfDay(tau))
        .run();
}

/// Adversarial robustness: the `SimpleTimetable` builder accepts shapes
/// the spec/render path doesn't normally produce — self-loop footpaths,
/// zero walk times, zero-duration legs. The algorithm must not panic on
/// any of them. No reference comparison: this is pure no-crash coverage.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn adversarial_builder_does_not_panic(tc: hegel::TestCase) {
    use hegel::generators;
    use vulture::manual::SimpleTimetable;

    let mut tt: SimpleTimetable<u8, u8, u16> = SimpleTimetable::new();
    // One route with zero-duration legs (every stop reached at the same
    // second) — defensible as a degenerate but valid GTFS shape.
    tt = tt.route(
        0u8,
        &[0u8, 1u8, 2u8],
        &[(
            0u16,
            &[
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(0), SecondOfDay(0)),
                (SecondOfDay(0), SecondOfDay(0)),
            ],
        )],
    );

    // A scatter of self-loop and zero-walk footpaths.
    let n_extra = tc.draw(generators::integers::<u8>().min_value(0).max_value(6));
    for _ in 0..n_extra {
        let from = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        let to = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        let walk = tc.draw(generators::integers::<u16>().min_value(0).max_value(50));
        tt = tt.footpath(from, to);
        tt = tt.transfer_time(from, to, Duration(u32::from(walk)));
    }

    let origin = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
    let target = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
    let max_t = tc.draw(generators::integers::<u8>().min_value(0).max_value(5));
    let tau = tc.draw(generators::integers::<u32>().min_value(0).max_value(1000));

    let _ = tt
        .query()
        .from(&[(tt.stop_idx_of(&origin), Duration::ZERO)])
        .to(&[(tt.stop_idx_of(&target), Duration::ZERO)])
        .max_transfers(max_t)
        .depart_at(SecondOfDay(tau))
        .run();
}

/// Wheelchair sibling-trip switching: when two trips on the same route
/// depart in close succession and only the later one is accessible, a
/// `require_wheelchair_accessible()` query must pick the accessible
/// trip — even though the earlier sibling is faster. This exercises
/// `earliest_accessible_trip`'s skip-forward directly; the layer-3
/// proptest hits the same path only when its ~10% accessibility-flag
/// roll happens to produce exactly this shape.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn wheelchair_picks_accessible_sibling(tc: hegel::TestCase) {
    use hegel::generators;
    use vulture::manual::SimpleTimetable;

    let dep1 = tc.draw(generators::integers::<u16>().min_value(0).max_value(200));
    let leg = tc.draw(generators::integers::<u16>().min_value(1).max_value(50));
    let gap = tc.draw(generators::integers::<u16>().min_value(1).max_value(30));
    let dep2 = dep1 + gap;
    let arr1 = dep1 + leg;
    let arr2 = dep2 + leg;

    let t = |secs: u16| (SecondOfDay(u32::from(secs)), SecondOfDay(u32::from(secs)));
    let tt: SimpleTimetable<u8, u8, u16> = SimpleTimetable::new()
        .route(
            0u8,
            &[0u8, 1u8],
            &[(0u16, &[t(dep1), t(arr1)]), (1u16, &[t(dep2), t(arr2)])],
        )
        .no_wheelchair_on_trip(0u16);

    let a = tt.stop_idx_of(&0u8);
    let b = tt.stop_idx_of(&1u8);

    let plain = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(1)
        .depart_at(SecondOfDay(0))
        .run();
    let accessible = tt
        .query()
        .from(&[(a, Duration::ZERO)])
        .to(&[(b, Duration::ZERO)])
        .max_transfers(1)
        .require_wheelchair_accessible()
        .depart_at(SecondOfDay(0))
        .run();

    assert_eq!(plain.len(), 1, "plain query should find 1 journey");
    assert_eq!(
        accessible.len(),
        1,
        "wheelchair query should find 1 journey"
    );
    assert_eq!(
        plain[0].arrival(),
        SecondOfDay(u32::from(arr1)),
        "plain picks earlier (inaccessible) trip"
    );
    assert_eq!(
        accessible[0].arrival(),
        SecondOfDay(u32::from(arr2)),
        "wheelchair picks later accessible sibling"
    );
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
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
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

    // Window of 2-5 departures spaced by a hegel-drawn step, descending
    // from tau. Saturating sub stays non-negative even at tau == 0.
    let tau = u32::from(spec.query.tau);
    let n_dep = tc.draw(
        hegel::generators::integers::<u32>()
            .min_value(2)
            .max_value(5),
    );
    let step = tc.draw(
        hegel::generators::integers::<u32>()
            .min_value(1)
            .max_value(60),
    );
    let departures: Vec<SecondOfDay> = (0..n_dep)
        .map(|i| SecondOfDay(tau.saturating_sub(i * step)))
        .collect();

    let serial = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers)
        .depart_in_window(departures.iter().copied())
        .run();

    let parallel = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers)
        .depart_in_window(departures.iter().copied())
        .run_par();

    let pool = RaptorCachePool::for_timetable(&timetable);
    let pooled = timetable
        .query()
        .from(&[(ps, Duration::ZERO)])
        .to(&[(pt, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers)
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
/// `query_with_label::<ArrivalAndWalk>()` per-`(target_stop,
/// target_walk)` Pareto front of `(arrival, walk_time, trip_count)`
/// equals the brute-force reference's. Stays on `layer2_bounds`: layer
/// 2 has footpaths (so walking accumulates non-trivially) but neither
/// fares nor accessibility flags, keeping the property focused on
/// `ArrivalAndWalk`'s arithmetic and dominance behaviour.
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
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
        .max_transfers(spec.query.max_transfers)
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
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
fn fare_label_matches_per_leg_sum(tc: hegel::TestCase) {
    use vulture::labels::{ArrivalAndFare, FareTable};

    let spec = tc.draw(spec::network_spec(spec::layer3_bounds()));
    let timetable = spec::render(&spec);

    // Build a FareTable mapping each rendered RouteIdx to the spec's
    // route fare. `render` registers every spec route under key u8 = its
    // enumeration index (it never silently drops; bad routes panic), so
    // a direct `route_idx_of` lookup is total.
    let mut per_route: std::collections::HashMap<vulture::RouteIdx, u32> =
        std::collections::HashMap::new();
    for (route_id_u8, route) in spec.routes.iter().enumerate() {
        let route_id = u8::try_from(route_id_u8).expect("layer3 route count fits in u8");
        per_route.insert(timetable.route_idx_of(&route_id), route.fare);
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
        .max_transfers(spec.query.max_transfers);
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
#[hegel::test(crate::proptest_settings(), test_cases = 500)]
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

    // Window of 2-5 departures descending from tau (which is always the
    // first entry, so vulture and the reference share an anchor). Step
    // size is hegel-drawn to vary how the window arranges around trip
    // departures in the rendered network.
    let tau = u32::from(spec.query.tau);
    let n_dep = tc.draw(
        hegel::generators::integers::<u32>()
            .min_value(2)
            .max_value(5),
    );
    let step = tc.draw(
        hegel::generators::integers::<u32>()
            .min_value(1)
            .max_value(60),
    );
    let raw_departures: Vec<u32> = (0..n_dep).map(|i| tau.saturating_sub(i * step)).collect();
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
        .max_transfers(spec.query.max_transfers)
        .depart_in_window(departures_secondofday.iter().copied())
        .run();

    let our_set: std::collections::BTreeSet<(u16, u32, u32, u16, u8)> = ours
        .iter()
        .map(|rj| {
            (
                u16::try_from(rj.depart.0).unwrap_or(u16::MAX),
                rj.journey.target.get(),
                rj.journey.target_walk.0,
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
            target_walk: Duration::ZERO,
            plan: plan
                .into_iter()
                .map(|(r, s)| (RouteIdx::new(r), StopIdx::new(s)))
                .collect(),
            label: ArrivalTime(SecondOfDay(arrival)),
        }
    }

    #[test]
    fn raptor_front_drops_dominated_higher_trip_journeys() {
        // All three journeys are at the same `(target, target_walk)`
        // slot (StopIdx(0), Duration::ZERO).
        let journeys = vec![
            j(100, vec![(0, 1)]),
            j(100, vec![(0, 2), (1, 3)]),
            j(80, vec![(0, 2), (1, 3)]),
        ];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u32, u32, u16, u8)> =
            [(0u32, 0u32, 100u16, 1u8), (0u32, 0u32, 80u16, 2u8)]
                .into_iter()
                .collect();
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
        let expected: BTreeSet<(u32, u32, u16, u8)> =
            [(0u32, 0u32, 50u16, 1u8)].into_iter().collect();
        assert_eq!(f, expected);
    }
}
