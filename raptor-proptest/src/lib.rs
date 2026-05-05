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

use raptor::Journey;

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
            let arr = u16::try_from(j.arrival())
                .expect("arrival exceeds u16::MAX — generator range exceeded?");
            let k = u8::try_from(j.plan.len())
                .expect("plan length exceeds u8::MAX — should never happen");
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
use raptor::Timetable;

#[cfg(test)]
fn run_property(tc: &hegel::TestCase, spec: &spec::NetworkSpec) {
    let timetable = spec::render(spec);
    let ps_idx = timetable.stop_idx_of(&spec.query.ps);
    let pt_idx = timetable.stop_idx_of(&spec.query.pt);
    let ours = timetable.raptor(
        spec.query.max_transfers as usize,
        spec.query.tau as usize,
        &[(ps_idx, 0)],
        &[(pt_idx, 0)],
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use raptor::{ArrivalTime, Journey, RouteIdx, StopIdx};

    fn j(arrival: usize, plan: Vec<(u32, u32)>) -> Journey {
        Journey {
            origin: StopIdx::new(0),
            target: StopIdx::new(0),
            plan: plan
                .into_iter()
                .map(|(r, s)| (RouteIdx::new(r), StopIdx::new(s)))
                .collect(),
            label: ArrivalTime(arrival),
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
