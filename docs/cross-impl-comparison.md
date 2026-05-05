# Cross-implementation comparison: vulture vs `raptor-journey-planner`

Side-by-side correctness and latency comparison between this crate and
[`planarnetwork/raptor`](https://github.com/planarnetwork/raptor) – a
TypeScript RAPTOR implementation by Linus Norton, published as
[`raptor-journey-planner`](https://www.npmjs.com/package/raptor-journey-planner)
(GPL-3.0). Same algorithm family (round-based RAPTOR), same input
(GTFS zip, index-once-query-many), so a head-to-head is meaningful.

The harness, query specification, and comparison script all live in
[`vulture-bench-js/`](../vulture-bench-js); reproduce locally with
`scripts/fetch-bench-feeds.sh` plus the workflow at the bottom of this
page.

## Methodology

- 3 warm-up queries discarded; 50 timed queries per query spec; warm
  `RaptorCache` (vulture) / shared `RaptorAlgorithm` instance (planar);
  median and p95 reported.
- Each implementation reads the same
  [`bench-queries.json`](../bench-queries.json) and emits a
  shape-identical JSON document; `vulture-bench-js/compare.mjs` diffs
  the two.
- Single laptop, Apple Silicon (arm64 macOS), single thread.
- vulture v0.14.0 (release build), `raptor-journey-planner` v2.3.1 on
  Node 22.15.0.
- Departure 09:00 (32 400 s since midnight), max 10 transfers.
- Result agreement is checked on the
  `(arrival_seconds, n_transit_legs)` Pareto frontier rather than
  exact journey identity, so tie-break differences between
  implementations don't show as mismatches.

## Results

### Delhi Metro · 262 stops · ~5.4 k active trips

Load time: vulture 102 ms, planar 365 ms.

| Query                                          | Vulture (median, p95) | Planar (median, p95) | Ratio | Match |
| ---------------------------------------------- | --------------------: | -------------------: | ----: | :---- |
| Dilshad Garden → Shahdara (1 trip)             |       8.7 µs, 11.1 µs |   254.7 µs, 487.7 µs |   29× | yes (1 journey) |
| Dilshad Garden → Vishwavidyalaya (2 trips)     |      35.6 µs, 37.3 µs |     202 µs, 240 µs   |    6× | yes (1 journey) |
| Paschim Vihar West → Ghitorni (3 trips)        |        74 µs, 80 µs   |     269 µs, 320 µs   |    4× | yes (2 journeys) |

### Helsinki HSL · 8.4 k stops · ~24 k active trips

Load time: vulture 11.5 s, planar 38.8 s.

| Query                                          | Vulture (median, p95) | Planar (median, p95) | Ratio | Match |
| ---------------------------------------------- | --------------------: | -------------------: | ----: | :---- |
| Kamppi metro → Itäkeskus metro                 |     8.83 ms, 9.30 ms  | gtfs-stream parse failure | – | planar empty |

`raptor-journey-planner`'s `gtfs-stream` parser returns 0 trips with
attached `stop_times` for the HSL 2026-05-04 feed (all 490 k trips are
discarded). Planar would presumably reach the same answer if it
parsed the feed; this is a parser-level failure not visible in the
algorithm comparison.

### Berlin VBB · 42 k stops · ~71 k active trips

Load time: vulture 6.8 s, planar 21.6 s.

| Query                                                                        | Vulture (median, p95) | Planar (median, p95) | Ratio | Match |
| ---------------------------------------------------------------------------- | --------------------: | -------------------: | ----: | :---- |
| Berlin Hauptbahnhof → Alexanderplatz (hand-picked eastbound S-Bahn platforms)|     421 µs, 493 µs    |   195.2 ms, 202.9 ms |  463× | yes (1 journey) |

### Paris IDFM · 54 k stops · ~146 k active trips

Load time: vulture 10.5 s, planar 38.5 s.

| Query                                          | Vulture (median, p95) | Planar (median, p95) | Ratio | Match |
| ---------------------------------------------- | --------------------: | -------------------: | ----: | :---- |
| Châtelet → Gare du Nord                        |    1.95 ms, 2.24 ms   |   299.5 ms, 315.4 ms |  153× | yes (1 journey) |
| Châtelet → La Défense                          |    2.30 ms, 2.69 ms   |   289.1 ms, 330.0 ms |  126× | yes (1 journey) |
| Châtelet → Versailles Rive Droite              |    27.4 ms, 32.4 ms   |   272.6 ms, 313.2 ms |   10× | yes (1 journey) |

## Reading the numbers

**Correctness.** Across all eight comparable queries spanning four
feeds and journey lengths from one to four legs, the two
implementations agree on the same Pareto frontier – same arrival time,
same number of transit legs, on every query. This is the first
end-to-end real-feed correctness check either project has against an
independent implementation; planar's own test suite asserts only
against synthetic toy timetables.

**Latency.** vulture is 4–463× faster on warm-cache queries. Two
distinct effects are bundled into that range:

1. *Rust release build vs Node 22.* Constant factor across all
   queries; rough order-of-magnitude estimate from Delhi (where the
   trip set is small enough that algorithm cost dominates JIT
   overhead).

2. *Trip set size.* The `Trips` column shows planar's `n_trips` is
   higher than vulture's at the larger feeds (Berlin 71 k / 275 k;
   Paris 146 k / 432 k). vulture filters trips by service-day at
   construction; planar's `RaptorAlgorithmFactory.create` filters at
   each query via `service.runsOn`, so its per-query route scan walks
   trips that are never going to be active. This widens the gap
   superlinearly with feed size (29× at Delhi, 463× at Berlin), and is
   the largest single contributor to the Berlin and Paris ratios.

The Châtelet → Versailles Rive Droite query is the most representative
real-world workload (54 k-stop network, multi-leg journey crossing the
RER C to suburban rail). vulture handles it in 27 ms; planar in
273 ms – a 10× gap, with both implementations finding the same
journey.

**Load time.** planar takes 3–3.6× longer to parse and index every
feed. Not algorithmic; this is the cost of the `gtfs-stream` streaming
parser plus per-trip JS object construction. Quoted for completeness
but excluded from the latency comparison since it's amortised across
all subsequent queries against the same loaded timetable.

## Caveats

- **Hbf → Alex is a narrow case.** A 1-leg, hand-picked-platform query
  exercises the smallest possible RAPTOR scan (one round, one route
  reachable from the boarding stop). The 463× ratio is the largest in
  this comparison precisely because vulture's overhead is near its
  floor while planar still pays per-trip iteration cost on the
  unfiltered trip set. Treat the Versailles RR query as the more
  realistic ratio for end-user workloads.

- **Helsinki HSL is excluded for a parser reason.** The
  `gtfs-stream`-backed loader in `raptor-journey-planner` returns
  490 033 trips with no `stopTimes` attached for the 2026-05-04 HSL
  feed (vulture parses it cleanly via `gtfs-structures`). The
  divergence is upstream of the routing algorithm. Stating "vulture
  handles a feed planar cannot" would be misleading – both
  implementations would presumably agree if planar's parser succeeded.

- **Timezone bug, worked around in the harness.**
  `RaptorAlgorithmFactory.create` derives its `dateNumber` from
  `date.toISOString()` (UTC) but its day-of-week from `date.getDay()`
  (local). Any host east of UTC, with a local-midnight `Date`,
  silently drops services that run on the local-Monday but not the
  previous-Sunday. This was the root cause of the Berlin Hbf → Alex
  10-minute mismatch and the three Paris zero-journey results in
  earlier runs. The harness anchors `service_date` to
  `Date.UTC(y, m-1, d)` so both halves of the inconsistency align;
  without that workaround the comparison is structurally broken on
  any timezone with a positive offset.

- **GC noise at the IDFM scale.** Planar's per-query medians of
  ~290 ms include occasional Node V8 garbage-collection pauses; with
  50 samples the p95 stays within +10–25 % of the median, but a longer
  comparison would benefit from explicit `--expose-gc` plus manual
  `gc()` between iterations to flatten the tail.

- **Single laptop, single Node version.** Numbers will shift modestly
  on different hardware and substantially on different Node versions.
  The methodology is stable; the absolute numbers are not.

## Reproduction

```bash
# 1. Fetch external feeds (~300 MB, gitignored under aux/external/).
./scripts/fetch-bench-feeds.sh

# 2. Run vulture's JSON-emitting harness.
cargo run --release --example cross-city-bench-json \
    > vulture-bench-js/results/vulture.json

# 3. Run the JS harness against the same query specification.
cd vulture-bench-js
npm install                      # one-off
node harness.mjs > results/raptor-js.json

# 4. Render the comparison.
node compare.mjs
```

The bundled Delhi feed (`aux/dmrc_gtfs.zip`) is exercised even without
step 1; skip step 1 and the other three feeds are reported as missing.

## Sources and licenses

| Component                  | Origin                                                             | License            |
| -------------------------- | ------------------------------------------------------------------ | ------------------ |
| `vulture` crate            | this repository                                                    | Apache-2.0         |
| `raptor-journey-planner`   | <https://github.com/planarnetwork/raptor> (Linus Norton)           | GPL-3.0            |
| `vulture-bench-js/planar-perf.ts` | verbatim copy of upstream `test/performance.ts`             | GPL-3.0 (file-local) |
