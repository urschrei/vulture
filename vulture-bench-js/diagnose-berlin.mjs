// Inspect what raptor-journey-planner's GTFS loader actually has for
// the two trips at the centre of the Berlin-divergence diagnosis:
//
//   277442991 – S5 S-Bahn, 09:15:24 Hbf -> 09:20:36 Alex (vulture's pick)
//   292578855 – regional, 09:26:00 Hbf -> 09:31:00 Alex (planar's pick)
//
// Tells us: is the S5 trip in the loaded set at all? Does it have
// stopTimes? Does its service.runsOn say it runs on 2026-05-04?
// Is its boarding stop registered as a route entry-point?

import * as fs from "node:fs";
import { fileURLToPath } from "node:url";
import * as path from "node:path";
import { loadGTFS, RaptorAlgorithmFactory } from "raptor-journey-planner";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..");
const FEED = path.join(REPO_ROOT, "aux/external/berlin.zip");
const SERVICE_DATE = new Date(2026, 4, 4); // 2026-05-04, local
const HBF = "de:11000:900003201:1:50";
const ALEX = "de:11000:900100003:1:50";
const TARGET_TRIPS = ["277442991", "292578855"];

function dateNumber(d) {
    return (
        d.getFullYear() * 10000 + (d.getMonth() + 1) * 100 + d.getDate()
    );
}

async function main() {
    console.error(`loading ${FEED}...`);
    const [tripsRaw, transfers, interchange, stops] = await loadGTFS(
        fs.createReadStream(FEED),
    );
    console.error(`raw trips: ${tripsRaw.length}`);

    const dn = dateNumber(SERVICE_DATE);
    const dow = SERVICE_DATE.getDay();
    console.error(`service date: ${SERVICE_DATE.toISOString().slice(0, 10)}, dateNumber=${dn}, dow=${dow}`);

    // Reproduce planar's exact factory logic: getDateNumber slices
    // toISOString (so UTC) but date.getDay() is local. Confirm whether
    // each trip survives the factory's runsOn check vs. a correct one.
    const isoDateNumber = (() => {
        const s = SERVICE_DATE.toISOString();
        return parseInt(s.slice(0, 4) + s.slice(5, 7) + s.slice(8, 10), 10);
    })();
    console.error(
        `factory dateNumber (UTC slice): ${isoDateNumber} ; date.getDay() (local): ${SERVICE_DATE.getDay()}`,
    );

    for (const tid of TARGET_TRIPS) {
        const trip = tripsRaw.find((t) => String(t.tripId) === tid);
        console.error("\n==", tid, "==");
        if (!trip) {
            console.error("  not in loaded trips array");
            continue;
        }
        console.error(`  serviceId: ${trip.serviceId}`);
        console.error(`  service exists: ${trip.service != null}`);
        console.error(
            `  stopTimes: ${Array.isArray(trip.stopTimes) ? trip.stopTimes.length : "N/A"} entries`,
        );
        if (trip.service && typeof trip.service.runsOn === "function") {
            console.error(`  service.runsOn(${dn}, ${dow}) [correct local]: ${trip.service.runsOn(dn, dow)}`);
            console.error(`  service.runsOn(${isoDateNumber}, ${dow}) [planar factory]: ${trip.service.runsOn(isoDateNumber, dow)}`);
        }
        if (Array.isArray(trip.stopTimes) && trip.stopTimes.length > 0) {
            const first = trip.stopTimes[0];
            const last = trip.stopTimes[trip.stopTimes.length - 1];
            console.error(`  first stop: ${first.stop} dep=${first.departureTime} pickup=${first.pickUp} dropoff=${first.dropOff}`);
            console.error(`  last stop:  ${last.stop} arr=${last.arrivalTime} pickup=${last.pickUp} dropoff=${last.dropOff}`);
            const hbfIdx = trip.stopTimes.findIndex((s) => s.stop === HBF);
            const alexIdx = trip.stopTimes.findIndex((s) => s.stop === ALEX);
            console.error(`  Hbf stop index: ${hbfIdx} (of ${trip.stopTimes.length})`);
            console.error(`  Alex stop index: ${alexIdx}`);
            if (hbfIdx >= 0) {
                const s = trip.stopTimes[hbfIdx];
                console.error(`    Hbf entry: dep=${s.departureTime} pickup=${s.pickUp} dropoff=${s.dropOff}`);
            }
            if (alexIdx >= 0) {
                const s = trip.stopTimes[alexIdx];
                console.error(`    Alex entry: arr=${s.arrivalTime} pickup=${s.pickUp} dropoff=${s.dropOff}`);
            }
        }
    }

    // Also: filter for the date the way the harness does, build the
    // RAPTOR factory, and see whether a route containing trip 277442991
    // gets a routesAtStop entry for the Hbf platform.
    const trips = tripsRaw.filter(
        (t) =>
            Array.isArray(t.stopTimes) &&
            t.stopTimes.length > 0 &&
            t.service != null &&
            typeof t.service.runsOn === "function",
    );
    const datedTrips = trips.filter((t) => t.service.runsOn(dn, dow));
    console.error(`\ntrips after date filter: ${datedTrips.length}`);

    // Build a route_id the same way RaptorAlgorithmFactory.getRouteId does, for both trips.
    const routeIdOf = (trip) =>
        trip.stopTimes
            .map((s) => s.stop + (s.pickUp ? 1 : 0) + (s.dropOff ? 1 : 0))
            .join();

    for (const tid of TARGET_TRIPS) {
        const trip = datedTrips.find((t) => String(t.tripId) === tid);
        if (!trip) {
            console.error(`  ${tid}: not in date-filtered trips`);
            continue;
        }
        const rid = routeIdOf(trip);
        const sameRoute = datedTrips.filter((t) => routeIdOf(t) === rid);
        console.error(
            `  ${tid}: synthetic route has ${sameRoute.length} trip(s); Hbf at index ${trip.stopTimes.findIndex((s) => s.stop === HBF)}; route_id length ${rid.length}`,
        );
    }

    // Build the raptor and reach into internals to check if each route
    // is registered at Hbf as a boarding option.
    const raptor = RaptorAlgorithmFactory.create(
        trips,
        transfers,
        interchange,
        SERVICE_DATE,
    );

    // RaptorAlgorithm has these fields per dist/src/raptor/RaptorAlgorithm.d.ts:
    //   private readonly routeStopIndex: RouteStopIndex
    //   private readonly routePath: RoutePaths
    //   ... QueueFactory holds routesAtStop
    const routesAtStop = raptor.queueFactory.routesAtStop;
    const routesAtHbf = routesAtStop[HBF];
    console.error(
        `\nroutesAtStop["${HBF}"]: ${routesAtHbf ? routesAtHbf.length : "undefined"} route(s)`,
    );

    // For each target trip, check whether its synthetic route_id is in routesAtHbf.
    for (const tid of TARGET_TRIPS) {
        const trip = datedTrips.find((t) => String(t.tripId) === tid);
        if (!trip) continue;
        const rid = routeIdOf(trip);
        const isAtHbf = (routesAtHbf || []).includes(rid);
        const stopIdxOnRoute = raptor.routeStopIndex[rid]?.[HBF];
        const pathLen = raptor.routePath[rid]?.length;
        console.error(
            `  ${tid}: route registered at Hbf=${isAtHbf}, routeStopIndex[rid][Hbf]=${stopIdxOnRoute}, pathLen=${pathLen}`,
        );
    }

    // Find which synthetic route trip 277442991 ACTUALLY ended up on
    // in the factory's tripsByRoute (after overtaking-suffix logic).
    const tripsByRoute = raptor.routeScannerFactory.tripsByRoute;
    for (const tid of TARGET_TRIPS) {
        const ridsContaining = Object.keys(tripsByRoute).filter((rid) =>
            tripsByRoute[rid].some((t) => String(t.tripId) === tid),
        );
        console.error(
            `\ntrip ${tid} appears in tripsByRoute under ${ridsContaining.length} key(s):`,
        );
        for (const rid of ridsContaining) {
            const isAtHbf = (routesAtHbf || []).includes(rid);
            const stopIdx = raptor.routeStopIndex[rid]?.[HBF];
            const pathLen = raptor.routePath[rid]?.length;
            const isOvertakes = rid.endsWith("overtakes");
            console.error(
                `  rid_len=${rid.length}, isOvertakes=${isOvertakes}, atHbf=${isAtHbf}, hbfIdx=${stopIdx}, pathLen=${pathLen}, n_trips=${tripsByRoute[rid].length}`,
            );
        }
    }

    // Sanity: how many distinct routes are at Hbf, and which one would
    // win? Walk each route's first trip departing after 09:00 from Hbf.
    if (routesAtHbf) {
        console.error("\nroutes at Hbf, earliest trip departing >= 32400:");
        const tripsByRoute = raptor.routeScannerFactory.tripsByRoute;
        for (const rid of routesAtHbf) {
            const idx = raptor.routeStopIndex[rid][HBF];
            const candidates = (tripsByRoute[rid] || [])
                .map((t) => ({
                    tripId: t.tripId,
                    dep: t.stopTimes[idx]?.departureTime,
                }))
                .filter((c) => c.dep != null && c.dep >= 32400)
                .sort((a, b) => a.dep - b.dep);
            const winner = candidates[0];
            console.error(
                `  rid_len=${rid.length} hbf_idx=${idx} earliest=${winner ? `${winner.tripId} @ ${winner.dep}` : "none"}`,
            );
            if (rid.length === 259 || rid.length === 467) {
                console.error(`    rid prefix: ${rid.slice(0, 80)}...`);
            }
        }
    }
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
