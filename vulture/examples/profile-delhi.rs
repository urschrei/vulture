//! Profiling harness: load Delhi Metro once, loop the same query so the
//! profile reflects the query path rather than GTFS loading. Intended
//! for `samply record -- target/profiling/examples/profile-delhi`.
//!
//! Build with `cargo build --profile profiling --example profile-delhi`.
//! The workspace `[profile.profiling]` block inherits release codegen
//! (so the profile reflects what users actually run) and adds DWARF +
//! a packed `.dSYM` so the profiler resolves symbols. `--release` also
//! works if you do not need symbol resolution; the binary then sits at
//! `target/release/examples/profile-delhi`.

use gtfs_structures::Gtfs;
use jiff::civil::date;
use std::hint::black_box;
use vulture::gtfs::GtfsTimetable;
use vulture::{Duration, RaptorCache, SecondOfDay, Timetable};

const GTFS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aux/dmrc_gtfs.zip");
const DEPARTURE: SecondOfDay = SecondOfDay::hms(9, 0, 0);
const ITERS: usize = 200_000;

fn main() {
    let gtfs = Gtfs::new(GTFS_PATH).expect("load delhi gtfs");
    let tt = GtfsTimetable::new(&gtfs, date(2024, 1, 15)).expect("build timetable");

    // Three Delhi queries chosen to mirror the cross-city-bench /
    // criterion gtfs_query benches: 1-trip, 2-trip cross-network,
    // 3-trip three-line. ITERS apiece keeps the harness honest about
    // which shape dominates.
    let q1 = (
        tt.stop_idx("1").expect("dilshad garden"),
        tt.stop_idx("4").expect("shahdara"),
    );
    let q2 = (
        tt.stop_idx("1").expect("dilshad garden"),
        tt.stop_idx("44").expect("vishwavidyalaya"),
    );
    let q3 = (
        tt.stop_idx("29").expect("paschim vihar west"),
        tt.stop_idx("65").expect("ghitorni"),
    );

    let mut cache = RaptorCache::for_timetable(&tt);

    eprintln!("warming up...");
    for _ in 0..1_000 {
        for &(o, t) in &[q1, q2, q3] {
            let r = tt
                .query()
                .from(&[(o, Duration::ZERO)])
                .to(&[(t, Duration::ZERO)])
                .max_transfers(10)
                .depart_at(DEPARTURE)
                .run_with_cache(&mut cache);
            black_box(&r);
        }
    }

    eprintln!("profiling: {ITERS} iterations × 3 queries");
    let start = std::time::Instant::now();
    for _ in 0..ITERS {
        for &(o, t) in &[q1, q2, q3] {
            let r = tt
                .query()
                .from(&[(o, Duration::ZERO)])
                .to(&[(t, Duration::ZERO)])
                .max_transfers(10)
                .depart_at(DEPARTURE)
                .run_with_cache(&mut cache);
            black_box(&r);
        }
    }
    let elapsed = start.elapsed();
    eprintln!(
        "done in {:?} ({:.2} µs / query)",
        elapsed,
        elapsed.as_secs_f64() * 1e6 / (ITERS as f64 * 3.0)
    );
}
