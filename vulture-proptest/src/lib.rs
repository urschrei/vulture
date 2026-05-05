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

#[cfg(test)]
use vulture::{Duration, SecondOfDay, Timetable};

#[cfg(test)]
fn run_property(tc: &hegel::TestCase, spec: &spec::NetworkSpec) {
    let timetable = spec::render(spec);
    let ps_idx = timetable.stop_idx_of(&spec.query.ps);
    let pt_idx = timetable.stop_idx_of(&spec.query.pt);
    let ours = timetable
        .query()
        .from(&[(ps_idx, Duration::ZERO)])
        .to(&[(pt_idx, Duration::ZERO)])
        .max_transfers(spec.query.max_transfers as usize as u8)
        .depart_at(SecondOfDay(spec.query.tau as u32))
        .run();
    let theirs = reference::reference_solve(
        spec,
        spec.query.ps,
        spec.query.pt,
        spec.query.tau,
        spec.query.max_transfers,
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

#[hegel::test]
fn layer1_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test]
fn layer2_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    run_property(&tc, &spec);
}

#[hegel::test(test_cases = 500)]
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
#[hegel::test]
fn parallel_naive_matches_serial_rrap(tc: hegel::TestCase) {
    use vulture::RaptorCachePool;

    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    let timetable = spec::render(&spec);
    let ps = timetable.stop_idx_of(&spec.query.ps);
    let pt = timetable.stop_idx_of(&spec.query.pt);

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
