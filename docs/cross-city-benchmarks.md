# Cross-city benchmarks

Real-feed performance numbers for `raptor-rs` v0.4.0 across four GTFS
feeds spanning two orders of magnitude in network size. Measured on
Apple Silicon (arm64 macOS), single laptop, single thread; warm
`RaptorCache` reused across queries; criterion-style methodology
(50 samples per query, median reported).

This page is the output of `examples/cross-city-bench.rs`. Reproduce
locally with the steps in [Reproduction](#reproduction).

## Feed scale and load time

`load time` is one-shot construction: parsing the GTFS zip, applying
calendar filtering for the chosen service date, interning identifiers,
and building the per-route arrival/departure tables.

| Feed         | Service date | Stops  | Routes (synthetic, post-filter) | Trips (active) | Load time |
|--------------|--------------|-------:|------------------------------:|---------------:|----------:|
| Delhi Metro  | 2024-01-15   |    262 |                            35 |          5,379 |    101 ms |
| Helsinki HSL | 2026-05-04   |  8,376 |                           912 |         24,379 |   11.28 s |
| Berlin VBB   | 2026-05-04   | 41,986 |                        10,758 |         71,037 |    6.67 s |
| Paris IDFM   | 2026-05-04   | 54,018 |                         8,621 |        145,551 |   10.61 s |

The "Routes" column counts the *synthetic* routes the adapter creates
after splitting each GTFS `route_id` by stop pattern and overtaking,
and after dropping any synthetic that has no trips active on the
service date. The "Trips (active)" column shows how many trips
survived calendar filtering — Helsinki's feed contains 490k trips
across many service days, of which only ~5% run on a single Monday;
Berlin runs ~26% and Paris ~32% on a typical Monday.

Load time is a one-time cost paid at startup; queries against the
loaded timetable do not re-incur it.

## Query latency

Departure 09:00 (32400 s since midnight); 10 max transfers; warm
`RaptorCache::for_timetable(&tt)` reused across every iteration. Median
of 50 measurements per query.

### Delhi Metro (`aux/dmrc_gtfs.zip`)

Bundled in the repo. 262 stops, 36 routes. No `transfers.txt`, so all
multi-leg journeys use shared physical interchange stops.

| Query                                              | Median latency |              Result |
|----------------------------------------------------|---------------:|--------------------:|
| Dilshad Garden → Shahdara (1 trip, Red Line east)  |         1.4 µs | arr 09:09:32 (9 m)  |
| Dilshad Garden → Vishwavidyalaya (2 trips)         |          16 µs | arr 09:31:49 (32 m) |
| Paschim Vihar West → Ghitorni (3 trips, 3 lines)   |          37 µs | arr 10:22:52 (83 m) |

### Helsinki HSL

| Query                                              | Median latency |              Result |
|----------------------------------------------------|---------------:|--------------------:|
| Kamppi metro (1040601) → Itäkeskus metro (1453601) |          26 µs | arr 09:17:00 (17 m) |

The Kamppi-to-Itäkeskus leg is a single direct trip on the M1/M2
metro line. A second query (`Rautatientori 1020112 → Pasila 1174501`,
~3 km between two tram-served plaza stops) currently returns no
journey at this departure time even with 10 transfers allowed; this
appears to be a footpath/transfer-graph density limitation and is
discussed in [Known limitations](#known-limitations) below.

### Berlin VBB

| Query                                                              | Median latency |              Result |
|--------------------------------------------------------------------|---------------:|--------------------:|
| Berlin Hauptbahnhof → Alexanderplatz (S-Bahn, eastbound platforms) |          88 µs | arr 09:20:36 (20 m) |

The 20-minute travel time is the actual S-Bahn journey from Hbf to
Alex *plus the wait* for the next eastbound train: in this snapshot,
the next train boards at 09:15:24 and arrives Alex at 09:21:48, so a
09:00 query waits ~15 minutes plus the ~6-minute ride.

This query benefits from the loop-route fix in v0.5 (the previous
version reported a 245-minute "best journey" — the algorithm couldn't
find a direct S-Bahn between the platforms originally chosen because
the IDs were for opposite-direction tracks, and the loop-route bug
was producing further nonsense in the long alternate routes it
considered). It also illustrates limitation §1 below: until parent-
station aggregation lands, callers picking platform IDs by hand have
to match direction explicitly.

### Paris IDFM

| Query                              | Median latency |              Result |
|------------------------------------|---------------:|--------------------:|
| Châtelet → Gare du Nord            |        0.41 ms | arr 09:05:00 (5 m)  |
| Châtelet → La Défense              |         0.6 ms | arr 09:10:09 (10 m) |
| Châtelet → Versailles Rive Droite  |         9.8 ms | arr 09:43:00 (43 m) |

All three queries return single sensible journeys — the loop-route
soundness bug that produced ARR<DEP results in earlier benchmark runs
was fixed in Phase 0.11 (see [Known limitations](#known-limitations) §2
for the diagnosis history). Latencies are roughly 5–6× faster than the
v0.4 numbers because the position-aware accessors that fixed the
soundness bug also eliminated a per-stop linear scan in
`GtfsTimetable::get_arrival_time` / `get_departure_time`.

## Known limitations

The cross-city run surfaced three classes of issue, all of which were
on the roadmap as future work but had not previously been hit on real
feeds (the bundled Delhi feed is small and clean enough to avoid
them).

### 1. Parent stations are not aggregated

Most large GTFS feeds use a parent-station / child-platform model:
the parent (`location_type=1`) is the named station and the children
(`location_type=0`) are the individual platforms that trips actually
serve. The `Timetable::raptor` query takes a `StopIdx`, and our GTFS
adapter interns one `StopIdx` per row in `stops.txt` — *including*
parent stations, which have no `stop_times` entries and therefore no
routes serving them. A query for a parent-station ID returns no
journey because no route is found.

For now, callers must look up child platform IDs themselves. A
follow-up could either (a) skip parent stations during interning, or
(b) aggregate trips at child platforms onto the parent for query
purposes.

### 2. Routes whose trips revisit a stop (loops) — fixed in Phase 0.11

This was the root cause of the Paris ARR<DEP results in earlier
benchmark runs. Diagnosed by stepping a bad journey through the
algorithm; fixed by the v0.5 trait redesign.

The bug: GTFS allows a trip's `stop_sequence` to revisit the same
stop_id (bus loops, shuttles that turn around, terminus loops). The
v0.4 adapter collapsed each trip's stop sequence into a
`Vec<StopIdx>` and used `Vec::position()` to find a stop's index
within that sequence — `position()` returns the **first** occurrence.
When a trip visited stop X at sequence-index 0 (early morning) and
again at sequence-index 12 (late morning), `get_arrival_time(trip, X)`
returned the early-morning value regardless of which visit the
algorithm meant. The algorithm then wrote labels at downstream stops
with these impossibly-early arrivals.

Trip counts with this property across the benchmarked feeds:

| Feed         | Trips with duplicate stops |             of total | Distinct GTFS `route_id`s affected |
|--------------|---------------------------:|---------------------:|-----------------------------------:|
| Delhi Metro  |                          0 |               5,438  |                                  0 |
| Helsinki HSL |                          0 |             490,033  |                                  0 |
| Berlin VBB   |                      6,872 |             275,263  |                                205 |
| Paris IDFM   |                     12,352 |             459,152  |                                207 |

Helsinki and Delhi were never affected (zero loop trips). Berlin and
Paris had hundreds of bus loop routes each; this is why Paris queries
were the most visibly broken.

The v0.5 fix added an explicit "position within route" parameter to
the `Timetable` trait's accessors, eliminating the `position()`
ambiguity. The proptest harness's layer 3 generator now produces
loop trips and the algorithm passes against the brute-force reference
solver. The cross-city Paris queries above now return single sensible
journeys.

### 3. Calendar / service-day filtering — fixed in v0.6 (Phase 0.10)

`GtfsTimetable::new(&gtfs, service_date)` now takes a
`jiff::civil::Date` and filters trips by `calendar.txt` /
`calendar_dates.txt` at construction. The cross-city numbers above
reflect this — each feed gets a representative weekday in its calendar
window, and only trips active on that day enter the timetable. Per-feed
trip counts dropped substantially (Helsinki's 490k → 35k, Paris's 459k
→ 49k), with corresponding query-latency speedups in the 3–6× range.

### 4. Transfer-graph density

The Helsinki Rautatientori → Pasila query returns no journey despite
10 max transfers. Both stops are well-served, but `transfers.txt` may
not chain them within the algorithm's per-round footpath relaxation.
Roadmap item 3.4 (coordinate-derived walking footpaths) would fill in
the missing edges.

These four items are the priority follow-ups for Phase 0.x; each is
independently scoped, with loop routes (Phase 0.11) being the
soundness item with the largest blast radius.

## Reproduction

```bash
# 1. Fetch the external feeds (~300 MB, gitignored under aux/external/).
./scripts/fetch-bench-feeds.sh

# 2. Run the cross-city benchmark example.
cargo run --release --example cross-city-bench
```

The bundled Delhi feed (`aux/dmrc_gtfs.zip`, 1.1 MB, tracked in the
repo) is exercised even without step 1.

## Sources and licenses

| Feed         | Source                                                                                                    | License                                       |
|--------------|-----------------------------------------------------------------------------------------------------------|-----------------------------------------------|
| Delhi Metro  | bundled `aux/dmrc_gtfs.zip` (snapshot)                                                                    | bundled with repo                             |
| Helsinki HSL | [`http://dev.hsl.fi/gtfs/hsl.zip`](http://dev.hsl.fi/gtfs/hsl.zip)                                        | CC-BY 4.0 — © HSL / Helsingin seudun liikenne |
| Berlin VBB   | [`https://www.vbb.de/vbbgtfs`](https://www.vbb.de/vbbgtfs)                                                | CC-BY 3.0 DE — © Verkehrsverbund Berlin-Brandenburg |
| Paris IDFM   | [`https://eu.ftp.opendatasoft.com/stif/GTFS/IDFM-gtfs.zip`](https://eu.ftp.opendatasoft.com/stif/GTFS/IDFM-gtfs.zip) | Licence Mobilités — © Île-de-France Mobilités |

Snapshots taken on 2026-05-04. Feeds are updated by their publishers
on regular cadences (HSL daily, VBB twice weekly, IDFM three times
daily); rerunning `scripts/fetch-bench-feeds.sh` will refresh them but
the numbers in this page are pinned to the 2026-05-04 snapshot.
