# vulture-wasm

WebAssembly bindings for [vulture](https://github.com/urschrei/vulture) – a Rust implementation of RAPTOR public-transit routing – exposed as a single browser-friendly ES module via [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) and [wasm-pack](https://rustwasm.github.io/wasm-pack/).

The JS side fetches a GTFS zip with `fetch()`, hands the bytes to `new VultureTimetable(zipBytes, "YYYY-MM-DD")`, and runs queries against it. There is no HTTP code inside the wasm blob.

The wasm bundle is currently ~700 KB raw, ~275 KB gzipped.

## Install

```sh
npm install vulture-wasm
```

```js
import init, { VultureTimetable, runArrival, runRange } from "vulture-wasm";

await init();   // loads the wasm module

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
- `tt.allStops()` — bulk catalogue of `{idx, id, name, lat, lon}` per stop.
- `tt.allRoutes()` — bulk catalogue of `{idx, id, name, agency, route_type, route_color}` per route. `route_type` is the GTFS integer (0=tram, 1=metro, 2=rail, 3=bus, 4=ferry, 5=cable, 6=aerial, 7=funicular, …); `route_color` is `"#rrggbb"` or `null`.
- `tt.withWalkingFootpaths(maxDistMeters, walkSpeedMetersPerSec)` — replace the footpath set with one derived from stop coordinates.
- `tt.resetFootpaths()` — restore the original `transfers.txt` set.

Free functions:

- `runArrival(tt, originStops, targetStops, maxTransfers, departSeconds, requireWheelchair)` — single-departure query. Returns an array of journeys with timed legs.
- `runRange(tt, origin, target, maxTransfers, departures, requireWheelchair)` — depart-in-window Pareto profile. Returns `[{depart, journey}]` where each `journey` has the same shape as a `runArrival` entry (`origin`, `target`, `arrival`, `legs`).

Each leg in `journey.legs` is `{board_stop, alight_stop, route, trip, route_id, depart, arrive, shape}`. `shape` is a `[lat, lon][]` polyline for that leg's segment of the trip's `shapes.txt` geometry, or `null` if the feed has no shape for the trip.

`originStops` / `targetStops` / `departures` are `Uint32Array`s; times are seconds since midnight on the service date. Full type signatures are in the bundled `vulture_wasm.d.ts`.

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

"Show me the Pareto-optimal options if I leave any time between 09:00 and 10:00."

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

const profile = runRange(tt, origin, target, 10, departures, false);

for (const entry of profile) {
    console.log(
        `leave ${entry.depart}s -> arrive ${entry.journey.arrival}s` +
        ` in ${entry.journey.legs.length} leg(s)`,
    );
}
```

### 3. Walking footpaths from coordinates

GTFS feeds with sparse `transfers.txt` (e.g. Helsinki HSL) leave most station-to-station walks unmodelled. Augment from coordinates and compare:

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

## Build from source

```sh
# From the repo root:
./scripts/build-demo.sh             # builds vulture-wasm/pkg/ and docs/demo/pkg/
./scripts/build-demo.sh --serve     # ... and then serves docs/demo/ at http://localhost:8765

# Or wasm-pack directly:
wasm-pack build vulture-wasm --target web --release
```

The web (ESM) target is the canonical and only output. It works in browsers, in modern Node, and inside any bundler that understands ES modules.

## Node

Same package, same import — Node just needs the wasm bytes passed in explicitly because there's no relative `fetch()` for the bundled `.wasm` file:

```js
import init, { VultureTimetable, runArrival } from "vulture-wasm";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasmPath = require.resolve("vulture-wasm/vulture_wasm_bg.wasm");

await init({ module_or_path: readFileSync(wasmPath) });

const tt = new VultureTimetable(readFileSync("feed.zip"), "2024-01-15");
```

The smoke test at `vulture-wasm/tests/smoke.mjs` is a runnable example: it loads the bundled Delhi Metro feed and exercises every public function. Run it with:

```sh
wasm-pack build vulture-wasm --target web --release
node vulture-wasm/tests/smoke.mjs
```

## Demo page

`docs/demo/` is a vanilla-JS / vanilla-CSS demo with three panels – stop-to-stop, departure window, walking-footpath comparison – running entirely client-side against the bundled Delhi Metro feed. Live at <https://urschrei.github.io/vulture/>.

The demo also renders each journey on a MapLibre dark basemap, using `leg.shape` from `runArrival` / `runRange` for the polyline geometry and `route.route_color` / `route.route_type` for the per-mode colour. See `docs/demo/app.js` (`drawJourney`) for the rendering wiring — roughly 80 lines of GeoJSON sources + circle/line layers with a halo+stroke pattern for legibility on the dark tiles.
