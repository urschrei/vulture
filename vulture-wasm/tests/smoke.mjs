// Smoke test: load the bundled Delhi Metro GTFS, build a timetable,
// run a query, assert the journey shape matches what vulture's
// native tests assert.
//
// Run with:
//   wasm-pack build vulture-wasm --target web --release
//   node vulture-wasm/tests/smoke.mjs

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import init, { VultureTimetable, runArrival, runRange } from "../pkg/vulture_wasm.js";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");

// The web-target bundle does not auto-fetch the wasm in Node (no
// relative `fetch` for file://). Pass the bytes explicitly.
await init({
    module_or_path: readFileSync(
        new URL("../pkg/vulture_wasm_bg.wasm", import.meta.url),
    ),
});

const gtfsBytes = readFileSync(join(repoRoot, "aux", "dmrc_gtfs.zip"));

console.log(`loaded ${gtfsBytes.length} bytes of GTFS zip`);

const tt = new VultureTimetable(gtfsBytes, "2024-01-15");
console.log(`stops=${tt.nStops()} routes=${tt.nRoutes()}`);

const start = tt.stopIdx("1"); // Dilshad Garden
const target = tt.stopIdx("44"); // Vishwavidyalaya
if (start === undefined || target === undefined) {
    throw new Error("known stops not found");
}
console.log(`stop_idx: start=${start} target=${target}`);

// Depart at 09:00:00 = 32400 s; up to 10 transfers; no wheelchair filter.
const journeys = runArrival(
    tt,
    new Uint32Array([start]),
    new Uint32Array([target]),
    10,
    9 * 3600,
    false,
);

const fmt = (s) => {
    const hh = Math.floor(s / 3600);
    const mm = Math.floor((s % 3600) / 60);
    const ss = s % 60;
    return `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
};

console.log(`got ${journeys.length} journey(s):`);
for (const j of journeys) {
    console.log(
        `  ${tt.stopName(j.origin)} -> ${tt.stopName(j.target)}, arrives ${fmt(j.arrival)}`,
    );
    for (const leg of j.legs) {
        console.log(
            `    ${fmt(leg.depart)} ${tt.stopName(leg.board_stop)} -> ${fmt(leg.arrive)} ${tt.stopName(leg.alight_stop)}  on ${tt.routeName(leg.route)}`,
        );
    }
}

if (journeys.length === 0) {
    throw new Error("expected at least one journey");
}
const j = journeys[0];
if (j.origin !== start || j.target !== target) {
    throw new Error(`origin/target mismatch: ${JSON.stringify(j)}`);
}
if (j.legs.length === 0) {
    throw new Error("expected at least one transit leg");
}
if (!j.legs.every((l) => l.depart > 0 && l.arrive > l.depart)) {
    throw new Error("legs missing timing");
}

// Range-query smoke: 09:00, 09:15, 09:30 → at least one Pareto entry.
const departures = new Uint32Array([9 * 3600, 9 * 3600 + 900, 9 * 3600 + 1800]);
const range = runRange(
    tt,
    new Uint32Array([start]),
    new Uint32Array([target]),
    10,
    departures,
    false,
);
console.log(`runRange returned ${range.length} entries`);
if (range.length === 0) {
    throw new Error("expected at least one range entry");
}

// Bulk catalogue smoke.
const stops = tt.allStops();
const routes = tt.allRoutes();
console.log(`allStops: ${stops.length} stops; allRoutes: ${routes.length} routes`);
console.log(`  example stop: ${JSON.stringify(stops[0])}`);
console.log(`  example route: ${JSON.stringify(routes[0])}`);

// New v0.19+ fields: per-leg shape on transit legs, route_type +
// route_color on routes.
const firstLeg = j.legs[0];
if (!Array.isArray(firstLeg.shape) || firstLeg.shape.length === 0) {
    throw new Error(
        `expected non-empty shape on first leg, got ${JSON.stringify(firstLeg.shape)}`,
    );
}
if (
    !firstLeg.shape.every(
        (p) => Array.isArray(p) && p.length === 2 && typeof p[0] === "number",
    )
) {
    throw new Error("expected shape entries to be [lat, lon] number pairs");
}
console.log(
    `  first-leg shape: ${firstLeg.shape.length} points, first=${JSON.stringify(firstLeg.shape[0])}`,
);

if (typeof routes[0].route_type !== "number") {
    throw new Error(
        `expected route_type to be number, got ${typeof routes[0].route_type}`,
    );
}
console.log(
    `  first route: type=${routes[0].route_type} color=${routes[0].route_color ?? "(none)"}`,
);

// Parent-station fields on stops + stationStops accessor.
if (typeof stops[0].location_type !== "number") {
    throw new Error(
        `expected location_type to be number, got ${typeof stops[0].location_type}`,
    );
}
console.log(
    `  first stop: location_type=${stops[0].location_type}` +
        ` parent_station=${stops[0].parent_station ?? "(none)"}`,
);

const stationStops = tt.stationStops("not-a-real-parent");
if (!(stationStops instanceof Uint32Array)) {
    throw new Error(
        `expected stationStops to return Uint32Array, got ${stationStops?.constructor?.name}`,
    );
}
if (stationStops.length !== 0) {
    throw new Error(
        `expected stationStops to be empty for unknown parent, got ${stationStops.length}`,
    );
}
console.log(`  stationStops("not-a-real-parent") returned ${stationStops.length} platforms`);

console.log("smoke test passed");
