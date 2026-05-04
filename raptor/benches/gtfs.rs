use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gtfs_structures::Gtfs;
use jiff::civil::{Date, date};
use raptor::gtfs::GtfsTimetable;
use raptor::{RaptorCache, Timetable};
use std::hint::black_box;

const GTFS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aux/dmrc_gtfs.zip");

// Departure time: 09:00:00 in seconds since midnight.
const DEPARTURE_TIME: usize = 9 * 3600;

// A Monday inside the bundled Delhi feed's calendar window
// (which ends 2025-12-31).
fn service_date() -> Date {
    date(2024, 1, 15)
}

// Query pairs picked to exercise different parts of the algorithm.
// Stop IDs are GTFS string IDs from aux/dmrc_gtfs.zip (Delhi Metro).
//
// The Delhi Metro feed has no transfers.txt, so cross-line journeys rely
// on interchange stations being shared physical stops on multiple routes
// (e.g. Kashmere Gate, Rajiv Chowk).
const QUERIES: &[(&str, &str, &str)] = &[
    // Direct trip on the Red Line east end (Dilshad Garden -> Shahdara).
    // One trip, ~10 minutes.
    ("direct_1trip", "1", "4"),
    // Two-trip cross-network (Dilshad Garden -> Vishwavidyalaya) via
    // Kashmere Gate interchange. ~30 minutes.
    ("transfer_2trip", "1", "44"),
    // Three-trip journey across three lines (Paschim Vihar West ->
    // Ghitorni) via Kirti Nagar and Rajiv Chowk. ~82 minutes.
    ("transfer_3trip", "29", "65"),
];

fn bench_gtfs_load(c: &mut Criterion) {
    c.bench_function("gtfs_load", |b| {
        b.iter(|| {
            let gtfs = Gtfs::new(GTFS_PATH).unwrap();
            let timetable = GtfsTimetable::new(&gtfs, service_date()).unwrap();
            black_box(&timetable);
        });
    });
}

fn bench_gtfs_query(c: &mut Criterion) {
    let gtfs = Gtfs::new(GTFS_PATH).unwrap();
    let timetable = GtfsTimetable::new(&gtfs).unwrap();
    let mut cache = RaptorCache::for_timetable(&timetable);

    let mut group = c.benchmark_group("gtfs_query");
    for &(name, start_id, target_id) in QUERIES {
        let start = match timetable.stop_idx(start_id) {
            Some(s) => s,
            None => panic!("query `{name}`: unknown start stop `{start_id}`"),
        };
        let target = match timetable.stop_idx(target_id) {
            Some(s) => s,
            None => panic!("query `{name}`: unknown target stop `{target_id}`"),
        };
        group.bench_with_input(
            BenchmarkId::new("raptor", name),
            &(start, target),
            |b, &(start, target)| {
                b.iter(|| {
                    let journeys =
                        timetable.raptor_with_cache(&mut cache, 10, DEPARTURE_TIME, start, target);
                    black_box(&journeys);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(gtfs_benches, bench_gtfs_load, bench_gtfs_query);
criterion_main!(gtfs_benches);
