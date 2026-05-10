# vulture-wasm

WebAssembly bindings for [vulture](https://github.com/urschrei/vulture), a Rust implementation of [RAPTOR](https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/) (Delling, Pajor, Werneck) public-transit routing. Single ES module, built with [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) and [wasm-pack](https://rustwasm.github.io/wasm-pack/).

Pass GTFS zip bytes to `new VultureTimetable(zipBytes, "YYYY-MM-DD")`, then query the handle. No HTTP code in the wasm blob.

Bundle: ~700 KB raw, ~290 KB gzipped.

## Install

```sh
npm install vulture-wasm
```

```js
import init, { VultureTimetable, runArrival, runRange } from "vulture-wasm";

await init();

const zipBytes = new Uint8Array(
    await (await fetch("/path/to/feed.zip")).arrayBuffer(),
);
const tt = new VultureTimetable(zipBytes, "2024-01-15");
```

## Drop-in via CDN (no build step)

```html
<script type="module">
import init, { VultureTimetable, runArrival, runRange }
    from "https://cdn.jsdelivr.net/npm/vulture-wasm@latest/vulture_wasm.js";

await init();

const tt = new VultureTimetable(
    new Uint8Array(await (await fetch("/feed.zip")).arrayBuffer()),
    "2024-01-15",
);
</script>
```

## API

`VultureTimetable` (opaque handle):

- `new VultureTimetable(zipBytes, serviceDate)` — parse a GTFS zip (`Uint8Array`) and pin the timetable to an ISO date (`YYYY-MM-DD`).
- `tt.nStops()`, `tt.nRoutes()` — counts.
- `tt.stopIdx(gtfsId)` — GTFS stop id → opaque `StopIdx`, or `undefined`.
- `tt.stopName(stopIdx)`, `tt.stopCoords(stopIdx)`, `tt.routeName(routeIdx)`, `tt.routeAgency(routeIdx)` — display-data accessors.
- `tt.allStops()` — bulk catalogue of `{idx, id, name, lat, lon, location_type, parent_station}` per stop. `location_type` is the GTFS integer (0 = stop or platform, 1 = parent station, 2 = entrance/exit, 3 = generic node, 4 = boarding area); `parent_station` is the parent's GTFS id or `null`.
- `tt.allRoutes()` — bulk catalogue of `{idx, id, name, agency, route_type, route_color}` per route. `route_type` is the GTFS integer (0 = tram, 1 = metro, 2 = rail, 3 = bus, 4 = ferry, 5 = cable, 6 = aerial, 7 = funicular, …); `route_color` is `"#rrggbb"` or `null`.
- `tt.stationStops(parentId)` — `Uint32Array` of child platform indices for a parent-station GTFS id. Pass to `originStops` / `targetStops` for any-platform queries. Queries rooted at a `location_type = 1` stop return zero journeys without this expansion.
- `tt.withWalkingFootpaths(maxDistMeters, walkSpeedMetersPerSec)` — replace the footpath set with one derived from stop coordinates.
- `tt.resetFootpaths()` — restore the original `transfers.txt` set.
- `tt.features()` — snapshot of the loaded feed as a JS object (camelCase fields): stop / route / trip counts, `transfers.txt` breakdown by `transfer_type`, parent-station and shaped-trip counts, wheelchair-flag counts, and footpath-graph state (`walkingFootpathsAdded`, `footpathsClosed`, `nFootpaths`). Counts are fixed at construction; footpath fields update on mutation.
- `tt.suggestions()` — advisory string array derived from the snapshot, e.g. `"transfers.txt is empty: call withWalkingFootpaths(...)"`. Empty when the feed is in a happy-path state.

Free functions:

- `runArrival(tt, originStops, targetStops, maxTransfers, departSeconds, requireWheelchair)` — single-departure query. Returns an array of journeys with timed legs.
- `runRange(tt, originStops, targetStops, maxTransfers, departures, requireWheelchair)` — depart-in-window Pareto profile. Returns `[{depart, journey}]`; `journey` matches a `runArrival` entry (`origin`, `target`, `arrival`, `legs`).

Each leg in `journey.legs`: `{board_stop, alight_stop, route, trip, route_id, depart, arrive, shape}`. `shape` is a `[lat, lon][]` polyline from the trip's `shapes.txt`, or `null` when absent.

`originStops` / `targetStops` / `departures` are `Uint32Array`s; times are seconds since midnight on the service date. Full type signatures are in the bundled `vulture_wasm.d.ts`.

## Native-only features

The wasm bindings omit:

- **Custom `Label` impls** and the bundled **`ArrivalAndWalk`** / **`ArrivalAndFare`** labels. Generics monomorphise per call-site; a wasm binding can ship only pre-baked label types. `runArrival` / `runRange` use the default arrival-time label.
- **`assert_footpaths_closed()`** — closed-footpath single-pass `O(E)` relaxation.
- **`new_with_overnight_days(...)`** — multi-day load for queries that cross the day boundary.
- **`RaptorCachePool`** — parallel fan-out; browsers do not expose rayon-style threading by default.

For any of these, use the native [`vulture`](https://crates.io/crates/vulture) crate.

## Examples

### 1. Stop-to-stop, single departure

```js
import init, { VultureTimetable, runArrival } from "vulture-wasm";

await init();

const zip = new Uint8Array(
    await (await fetch("https://urschrei.github.io/vulture/dmrc_gtfs.zip"))
        .arrayBuffer(),
);
const tt = new VultureTimetable(zip, "2024-01-15");

const start = tt.stopIdx("1");    // Dilshad Garden
const target = tt.stopIdx("44");  // Vishwavidyalaya

const journeys = runArrival(
    tt,
    new Uint32Array([start]),
    new Uint32Array([target]),
    /* maxTransfers */ 10,
    /* depart        */ 9 * 3600,   // 09:00:00
    /* wheelchair    */ false,
);

const fmt = (s) =>
    `${String(Math.floor(s / 3600)).padStart(2, "0")}:` +
    `${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}`;

for (const j of journeys) {
    console.log(
        `${tt.stopName(j.origin)} -> ${tt.stopName(j.target)},` +
        ` arrives ${fmt(j.arrival)}`,
    );
    for (const leg of j.legs) {
        console.log(
            `  ${fmt(leg.depart)} ${tt.stopName(leg.board_stop)}` +
            ` -> ${fmt(leg.arrive)} ${tt.stopName(leg.alight_stop)}` +
            `  on ${tt.routeName(leg.route)}`,
        );
    }
}
```

### 2. Departure window (range query)

Pareto-optimal options across a range of departures (here, every 5 minutes from 09:00 to 10:00):

```js
import init, { VultureTimetable, runRange } from "vulture-wasm";

await init();

const tt = new VultureTimetable(
    new Uint8Array(
        await (await fetch("https://urschrei.github.io/vulture/dmrc_gtfs.zip"))
            .arrayBuffer(),
    ),
    "2024-01-15",
);

const origin = tt.stopIdx("1");
const target = tt.stopIdx("44");

// Every 5 minutes from 09:00 up to (and including) 10:00.
const departures = new Uint32Array(
    Array.from({ length: 13 }, (_, i) => 9 * 3600 + i * 300),
);

const profile = runRange(
    tt,
    new Uint32Array([origin]),
    new Uint32Array([target]),
    10,
    departures,
    false,
);

for (const entry of profile) {
    console.log(
        `leave ${entry.depart}s -> arrive ${entry.journey.arrival}s` +
        ` in ${entry.journey.legs.length} leg(s)`,
    );
}
```

### 3. Walking footpaths from coordinates

Augment a sparse `transfers.txt` (e.g. Helsinki HSL) from stop coordinates and compare:

```js
import init, { VultureTimetable, runArrival } from "vulture-wasm";

await init();

const tt = new VultureTimetable(
    new Uint8Array(await (await fetch("/path/to/feed.zip")).arrayBuffer()),
    "2024-01-15",
);

const args = [
    tt,
    new Uint32Array([tt.stopIdx("1040601")]),   // Kamppi metro
    new Uint32Array([tt.stopIdx("1453601")]),   // Itäkeskus metro
    /* maxTransfers */ 10,
    /* depart        */ 9 * 3600,
    /* wheelchair    */ false,
];

const before = runArrival(...args);
console.log(`without footpaths: ${before.length} journey(s)`);

tt.withWalkingFootpaths(500, 1.4);   // 500 m at ~5 km/h
const after = runArrival(...args);
console.log(`with footpaths: ${after.length} journey(s)`);
```

`tt.resetFootpaths()` restores the original `transfers.txt` set.

### 4. Querying entire stations (parent-station expansion)

Parent stations (`location_type = 1`) have no boardings of their own; expand to their child platforms (`location_type = 0`) before querying.

```js
import init, { VultureTimetable, runArrival } from "vulture-wasm";

await init();

const tt = new VultureTimetable(
    new Uint8Array(await (await fetch("/path/to/feed.zip")).arrayBuffer()),
    "2026-05-04",
);

// Resolve a stop record (from `tt.allStops()` or any other lookup
// path) into the array of stop indices to query against.
function endpointsFor(stop) {
    if (stop.location_type === 1) {
        const platforms = tt.stationStops(stop.id);
        if (platforms.length > 0) return platforms;
    }
    return new Uint32Array([stop.idx]);
}

const stops = tt.allStops();
const stopByName = new Map(stops.map((s) => [s.name, s]));

const from = stopByName.get("Karlsruhe Hauptbahnhof");   // a parent station
const to = stopByName.get("Basel SBB");                  // also a parent

const journeys = runArrival(
    tt,
    endpointsFor(from),
    endpointsFor(to),
    /* maxTransfers */ 10,
    /* depart        */ 9 * 3600,
    /* wheelchair    */ false,
);
console.log(`${journeys.length} journey(s) across all platform combinations`);
```

The bundled demo uses the same helper across all three panels.

## Perf

No automated browser benchmark yet. The [live demo](https://urschrei.github.io/vulture/) runs every query client-side against the bundled Delhi Metro feed; time it in your browser's performance panel.

Native single-query latency is in the [main README](https://github.com/urschrei/vulture#perf): single-digit microseconds for a small metro feed, single-digit milliseconds for a large regional feed, tens of milliseconds for the heaviest. Browser numbers are slower than native by an engine- and JIT-dependent factor.

## Soundness

The Rust engine is validated by [`vulture-proptest`](https://github.com/urschrei/vulture/tree/main/vulture-proptest), which compares the algorithm against a brute-force reference solver on every test run. Issue-by-issue audit against the paper: [`docs/soundness.md`](https://github.com/urschrei/vulture/blob/main/docs/soundness.md). The bindings layer is exercised by `vulture-wasm/tests/smoke.mjs`.

## Build from source

```sh
# From the repo root:
./scripts/build-demo.sh             # builds vulture-wasm/pkg/ and docs/demo/pkg/
./scripts/build-demo.sh --serve     # ... and then serves docs/demo/ at http://localhost:8765

# Or wasm-pack directly:
wasm-pack build vulture-wasm --target web --release
```

The web (ESM) target is the only output. It works in browsers, modern Node, and any ES-module bundler.

## Node

Same package, same import. Node needs the wasm bytes passed in explicitly (no relative `fetch()` for the bundled `.wasm`):

```js
import init, { VultureTimetable, runArrival } from "vulture-wasm";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasmPath = require.resolve("vulture-wasm/vulture_wasm_bg.wasm");

await init({ module_or_path: readFileSync(wasmPath) });

const tt = new VultureTimetable(readFileSync("feed.zip"), "2024-01-15");
```

`vulture-wasm/tests/smoke.mjs` loads the bundled Delhi Metro feed and exercises every public function:

```sh
wasm-pack build vulture-wasm --target web --release
node vulture-wasm/tests/smoke.mjs
```

## Demo page

`docs/demo/` has three panels (stop-to-stop, departure window, walking-footpath comparison) running client-side against the bundled Delhi Metro feed. Live at <https://urschrei.github.io/vulture/>.

Journeys render on a MapLibre dark basemap; geometry from `leg.shape`, per-mode colour from `route.route_color` / `route.route_type`. Wiring in `docs/demo/app.js` (`drawJourney`).

## Changelog

[CHANGELOG](https://github.com/urschrei/vulture/blob/main/CHANGELOG.md) in the repo. `vulture` (Rust) and `vulture-wasm` (npm) have independent version streams; entries note both numbers when the bindings move with the engine.

## License

[Apache-2.0](https://github.com/urschrei/vulture/blob/main/LICENSE.md), matching the [`vulture`](https://github.com/urschrei/vulture) crate.
