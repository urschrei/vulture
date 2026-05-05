// Mirror of vulture/examples/cross-city-bench-json.rs.
//
// Reads ../bench-queries.json (or the path given as argv[2]), runs each
// query through raptor-journey-planner with a Pareto filter, and emits
// JSON of the same shape on stdout. The Rust harness emits its own
// JSON; both can then be diffed.
//
// Limitations relative to the Rust harness:
//   - raptor-journey-planner does not compute coordinate-derived
//     walking footpaths. Feeds whose spec sets `walking_footpaths_m`
//     run with whatever transfers.txt the GTFS already contains; Helsinki
//     in particular ships an empty transfers.txt, so its
//     stop-to-stop query may return no journey on this side.
//   - The library's Trip type does not expose a GTFS route_id, so
//     emitted legs carry `route_id: null`. trip_id is preserved.

import * as fs from "node:fs";
import { fileURLToPath } from "node:url";
import * as path from "node:path";
import {
    loadGTFS,
    RaptorAlgorithmFactory,
    JourneyFactory,
    GroupStationDepartAfterQuery,
    MultipleCriteriaFilter,
} from "raptor-journey-planner";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..");

function resolveSpecPath() {
    const arg = process.argv[2];
    if (arg) return path.resolve(arg);
    return path.join(REPO_ROOT, "bench-queries.json");
}

function resolveFeedPath(p) {
    if (path.isAbsolute(p)) return p;
    return path.join(REPO_ROOT, p);
}

function packageVersion() {
    const pkg = JSON.parse(
        fs.readFileSync(
            path.join(HERE, "node_modules", "raptor-journey-planner", "package.json"),
            "utf8",
        ),
    );
    return pkg.version;
}

function legToOutput(leg) {
    if (!("trip" in leg) || leg.trip == null) return null;
    const first = leg.stopTimes[0];
    const last = leg.stopTimes[leg.stopTimes.length - 1];
    return {
        type: "transit",
        trip_id: leg.trip.tripId,
        route_id: null,
        board_stop_id: leg.origin ?? first.stop,
        board_seconds: first.departureTime,
        alight_stop_id: leg.destination ?? last.stop,
        alight_seconds: last.arrivalTime,
    };
}

function journeysToOutput(journeys) {
    return journeys.map((j) => {
        const legs = j.legs.map(legToOutput).filter((l) => l !== null);
        return {
            arrival_seconds: j.arrivalTime,
            n_transit_legs: legs.length,
            legs,
        };
    });
}

async function runFeed(feed, cfg) {
    const feedPath = resolveFeedPath(feed.path);
    if (!fs.existsSync(feedPath)) {
        console.error(`skipping ${feed.name} (file missing: ${feedPath})`);
        return null;
    }
    if (feed.walking_footpaths_m != null) {
        console.error(
            `note: ${feed.name} requests walking footpaths (${feed.walking_footpaths_m}m); raptor-journey-planner does not compute these – running with transfers.txt only`,
        );
    }

    const [y, m, d] = feed.service_date.split("-").map(Number);
    // Anchor to UTC midnight: raptor-journey-planner's
    // RaptorAlgorithmFactory.create derives its dateNumber from
    // date.toISOString() (UTC) but its day-of-week from date.getDay()
    // (local). For any timezone east of UTC, a local-midnight Date
    // produces a UTC dateNumber for the previous day, so trips
    // get checked against runsOn(prev_day, mon-from-local) – silently
    // dropping any service that doesn't run the previous day. Using
    // UTC midnight keeps both halves of the inconsistency aligned.
    const serviceDate = new Date(Date.UTC(y, m - 1, d));

    const loadStart = process.hrtime.bigint();
    const stream = fs.createReadStream(feedPath);
    const [tripsRaw, transfers, interchange, stops] = await loadGTFS(stream);
    // raptor-journey-planner's RaptorAlgorithmFactory.create assumes
    // every trip has a non-empty `stopTimes` array AND a `service`
    // object. Real feeds (Helsinki HSL, Berlin VBB) sometimes hand
    // back trips that fail one or both – vulture's GtfsTimetable
    // rejects these at construction; mirror that here.
    const trips = tripsRaw.filter(
        (t) =>
            Array.isArray(t.stopTimes) &&
            t.stopTimes.length > 0 &&
            t.service != null &&
            typeof t.service.runsOn === "function",
    );
    const droppedTrips = tripsRaw.length - trips.length;
    if (droppedTrips > 0) {
        console.error(
            `note: ${feed.name} dropped ${droppedTrips} of ${tripsRaw.length} trip(s) (missing stop_times or service)`,
        );
    }
    if (trips.length === 0) {
        console.error(
            `skipping ${feed.name}: 0 trips with usable stop_times after filtering`,
        );
        return {
            name: feed.name,
            path: feed.path,
            service_date: feed.service_date,
            walking_footpaths_m: feed.walking_footpaths_m,
            n_stops: Object.keys(stops).length,
            n_routes: null,
            n_trips: 0,
            load_time_ns: Number(process.hrtime.bigint() - loadStart),
            queries: feed.queries.map((q) => ({
                label: q.label,
                origin_ids: q.origin_ids,
                target_ids: q.target_ids,
                samples_ns: [],
                journeys: [],
                error: "no usable trips after gtfs-stream parse",
            })),
        };
    }
    const raptor = RaptorAlgorithmFactory.create(
        trips,
        transfers,
        interchange,
        serviceDate,
    );
    const loadTimeNs = Number(process.hrtime.bigint() - loadStart);

    const nStops = Object.keys(stops).length;
    const nTrips = trips.length;
    console.error(
        `loaded ${feed.name} (${nStops} stops, ${nTrips} trips) in ${(loadTimeNs / 1e6).toFixed(0)} ms`,
    );

    const journeyFactory = new JourneyFactory();
    const filter = new MultipleCriteriaFilter();
    const query = new GroupStationDepartAfterQuery(
        raptor,
        journeyFactory,
        3,
        [filter],
    );

    const queriesOut = [];
    for (const q of feed.queries) {
        const queryOut = {
            label: q.label,
            origin_ids: q.origin_ids,
            target_ids: q.target_ids,
            samples_ns: [],
            journeys: [],
        };

        const planArgs = [q.origin_ids, q.target_ids, serviceDate, cfg.departure_seconds];

        try {
            for (let i = 0; i < cfg.warmup_iters; i++) {
                query.plan(...planArgs);
            }
        } catch (e) {
            queryOut.error = `warmup: ${e.message}`;
            queriesOut.push(queryOut);
            continue;
        }

        let lastJourneys = [];
        for (let i = 0; i < cfg.measure_iters; i++) {
            const t0 = process.hrtime.bigint();
            try {
                lastJourneys = query.plan(...planArgs);
            } catch (e) {
                queryOut.error = `measure[${i}]: ${e.message}`;
                break;
            }
            queryOut.samples_ns.push(Number(process.hrtime.bigint() - t0));
        }

        queryOut.journeys = journeysToOutput(lastJourneys);
        queriesOut.push(queryOut);
    }

    return {
        name: feed.name,
        path: feed.path,
        service_date: feed.service_date,
        walking_footpaths_m: feed.walking_footpaths_m,
        n_stops: nStops,
        n_routes: null,
        n_trips: nTrips,
        load_time_ns: loadTimeNs,
        queries: queriesOut,
    };
}

async function main() {
    const specPath = resolveSpecPath();
    const spec = JSON.parse(fs.readFileSync(specPath, "utf8"));
    const cfg = spec.configuration;

    const feedsOut = [];
    for (const feed of spec.feeds) {
        const result = await runFeed(feed, cfg);
        if (result) feedsOut.push(result);
    }

    const out = {
        implementation: "raptor-journey-planner",
        version: packageVersion(),
        configuration: cfg,
        feeds: feedsOut,
    };

    process.stdout.write(JSON.stringify(out, null, 2) + "\n");
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
