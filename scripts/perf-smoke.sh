#!/usr/bin/env bash
# Perf-regression smoke check.
#
# Runs the Delhi `gtfs_query` group from `vulture/benches/gtfs.rs` in
# criterion's `--quick` mode, comparing the median against the last
# saved baseline (criterion stores it in `target/criterion/`).
#
# Wall time on this machine:
#   ~2 s   with no source change since the last build
#   ~24 s  after a source edit (release profile's `lto = true` plus
#          `codegen-units = 1` link cost dominates re-link)
#
# Run before finalising any feature or perf-adjacent change. Especially
# important after touching `Cargo.toml` profile blocks or anything on the
# route-scan / footpath-relax hot path: the May 2026 `opt-level = "z"`
# regression (see CHANGELOG entry "Perf: native release back to opt-level
# = 3") would have shown as roughly +70-80 % on `transfer_2trip`.
#
# Criterion prints "Performance has regressed." when the median moves
# past noise. The first run on a fresh checkout will report no prior
# baseline; run it twice to start the comparison stream.
#
# Only data required is the bundled `aux/dmrc_gtfs.zip` (Delhi Metro);
# no external feed downloads.
#
# Extra arguments are forwarded to criterion, so e.g.
#   scripts/perf-smoke.sh --save-baseline pre-refactor
# is supported.

set -euo pipefail

cd "$(dirname "$0")/.."

exec cargo bench --features gtfs-bench --bench gtfs -- --quick gtfs_query "$@"
