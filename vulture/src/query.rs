//! [`Query`] – the typestate builder used to configure and run a
//! RAPTOR call – plus the marker types ([`NeedsDeparture`],
//! [`SingleDeparture`], [`RangeDeparture`]) that drive the typestate
//! transitions and the [`RangeJourney`] returned by range queries.

use std::marker::PhantomData;

use crate::Timetable;
use crate::algorithm::per_call::run_per_call_query;
use crate::algorithm::range::filter_range_pareto_front;
use crate::algorithm::range::raptor_range_rrap_arrival;
use crate::cache::RaptorCache;
#[cfg(feature = "parallel")]
use crate::cache::RaptorCachePool;
use crate::endpoints::Endpoints;
use crate::endpoints::IntoEndpoints;
use crate::journey::Journey;
use crate::label::ArrivalTime;
use crate::label::Label;
use crate::time::SecondOfDay;
use crate::time::Transfers;

/// One entry in a range-query profile: a departure time paired with
/// the [`Journey`] it produces. Returned by [`Query::run`] /
/// [`Query::run_with_cache`] when the builder was configured with
/// [`Query::depart_in_window`].
#[derive(Debug, Clone)]
pub struct RangeJourney<L: Label = ArrivalTime> {
    /// The departure time this journey assumes – the user leaves the
    /// origin (or starts the origin walk) at this time.
    pub depart: SecondOfDay,
    /// The journey itself, as if `depart` had been passed to
    /// [`Query::depart_at`] directly.
    pub journey: Journey<L>,
}

// ── Query builder ──────────────────────────────────────────────────

/// Marker type: a `Query` that hasn't yet had a departure configured.
/// `.run()` is not callable in this state; call [`Query::depart_at`]
/// (single departure) or [`Query::depart_in_window`] (range query)
/// first.
#[derive(Debug, Clone, Copy)]
pub struct NeedsDeparture;

/// Marker type: a single-departure `Query`. `.run()` returns
/// `Vec<Journey<L>>`.
#[derive(Debug, Clone, Copy)]
pub struct SingleDeparture {
    pub(crate) at: SecondOfDay,
}

/// Marker type: a range-query `Query` over a window of departure
/// times. The supplied iterator is collected eagerly at builder time
/// and normalised to a descending, deduplicated `Vec<SecondOfDay>`
/// (the order rRAPTOR scans them in). `.run()` returns
/// `Vec<RangeJourney<L>>`.
#[derive(Debug, Clone)]
pub struct RangeDeparture {
    pub(crate) departures: Vec<SecondOfDay>,
}

/// Builder for a RAPTOR query. Constructed via [`Timetable::query`].
///
/// Typestate enforces the construction order:
///
/// - The initial state ([`NeedsDeparture`]) admits the optional-input
///   methods ([`Query::from`], [`Query::to`], [`Query::max_transfers`])
///   plus a single departure-mode transition
///   ([`Query::depart_at`] or [`Query::depart_in_window`]).
/// - Once a departure mode is set ([`SingleDeparture`] or
///   [`RangeDeparture`]) the only further method is [`Query::run`] —
///   plus [`Query::run_with_cache`] for explicit cache reuse. The
///   return type of `.run()` matches the departure mode.
/// - `Query<L, RangeDeparture>` for custom `L != ArrivalTime` exposes
///   only `.run_par()` / `.run_with_pool()` (with the `parallel`
///   feature). The serial rRAPTOR specialisation only fires for
///   `L = ArrivalTime`.
///
/// Type parameters:
///
/// - `'tt`: borrow of the [`Timetable`].
/// - `T`: the timetable type.
/// - `L`: the [`Label`] type. Defaults to [`ArrivalTime`]. Pass a
///   different label by entering via [`Timetable::query_with_label`].
/// - `M`: the typestate marker. Defaults to [`NeedsDeparture`].
#[derive(Debug, Clone)]
pub struct Query<'tt, T, L = ArrivalTime, M = NeedsDeparture>
where
    T: Timetable + ?Sized,
    L: Label,
{
    pub(crate) tt: &'tt T,
    pub(crate) origins: Endpoints,
    pub(crate) targets: Endpoints,
    pub(crate) max_transfers: Transfers,
    /// When `true`, the algorithm filters out trips for which
    /// [`Timetable::trip_wheelchair_accessible`] returns `false` and
    /// stops for which [`Timetable::stop_wheelchair_accessible`] returns
    /// `false`. Defaults to `false` (no filtering).
    pub(crate) require_wheelchair_accessible: bool,
    /// Sidecar data threaded into every [`Label`] call. For
    /// `L::Ctx = ()` (the default for `ArrivalTime` and
    /// `ArrivalAndWalk`) this is zero-sized and free; custom labels
    /// like a fare-aware `ArrivalAndFare` use it to carry per-route
    /// fare tables, stop → zone maps, etc. Replace via
    /// [`Query::with_context`].
    pub(crate) ctx: L::Ctx,
    pub(crate) mode: M,
    pub(crate) _label: PhantomData<L>,
}

// ----- Stage 1: NeedsDeparture – optional inputs and mode transitions -----

impl<'tt, T, L> Query<'tt, T, L, NeedsDeparture>
where
    T: Timetable + ?Sized,
    L: Label,
{
    /// Set the origin endpoints. Replaces any previously-set origins.
    /// Accepts any [`IntoEndpoints`] shape (single stop, slice, vec).
    pub fn from(mut self, ep: impl IntoEndpoints) -> Self {
        self.origins = ep.into_endpoints();
        self
    }

    /// Set the target endpoints. Replaces any previously-set targets.
    pub fn to(mut self, ep: impl IntoEndpoints) -> Self {
        self.targets = ep.into_endpoints();
        self
    }

    /// Cap the number of transit boardings the algorithm explores.
    /// The default is 10. Pass an integer literal – `.max_transfers(10)`
    /// works directly, no suffix needed.
    pub fn max_transfers(mut self, n: u8) -> Self {
        self.max_transfers = Transfers(n);
        self
    }

    /// Configure a single-departure query. After this call, `.run()`
    /// returns `Vec<Journey<L>>`.
    pub fn depart_at(self, t: impl Into<SecondOfDay>) -> Query<'tt, T, L, SingleDeparture> {
        Query {
            tt: self.tt,
            origins: self.origins,
            targets: self.targets,
            max_transfers: self.max_transfers,
            require_wheelchair_accessible: self.require_wheelchair_accessible,
            ctx: self.ctx,
            mode: SingleDeparture { at: t.into() },
            _label: PhantomData,
        }
    }

    /// Configure a range query over the supplied departure times.
    /// The iterator is collected eagerly at builder time and normalised
    /// to descending, deduplicated order (the order rRAPTOR scans them
    /// in; the parallel naïve-batch path is order-insensitive). After
    /// this call, `.run()` returns `Vec<RangeJourney<L>>` (for
    /// `L = ArrivalTime` via the rRAPTOR specialisation; for other
    /// labels only the parallel paths are exposed).
    pub fn depart_in_window(
        self,
        deps: impl IntoIterator<Item = SecondOfDay>,
    ) -> Query<'tt, T, L, RangeDeparture> {
        let mut departures: Vec<SecondOfDay> = deps.into_iter().collect();
        // rRAPTOR processes descending; sort+dedupe once at builder
        // time so both serial and parallel paths see canonical input.
        departures.sort_unstable_by(|a, b| b.cmp(a));
        departures.dedup();
        Query {
            tt: self.tt,
            origins: self.origins,
            targets: self.targets,
            max_transfers: self.max_transfers,
            require_wheelchair_accessible: self.require_wheelchair_accessible,
            ctx: self.ctx,
            mode: RangeDeparture { departures },
            _label: PhantomData,
        }
    }

    /// Supply the per-[`Label`] sidecar context — the lookup tables
    /// the label needs to evaluate its criteria. Build once per
    /// query (or share across queries via [`Clone`]); the algorithm
    /// borrows it immutably and threads `&L::Ctx` into every
    /// [`Label::from_departure`], [`Label::extend_by_trip`] and
    /// [`Label::extend_by_footpath`] call.
    ///
    /// The default is [`Default::default`] for `L::Ctx`. Labels
    /// whose `Ctx` is `()` (e.g. [`ArrivalTime`](crate::ArrivalTime)
    /// and [`ArrivalAndWalk`](crate::labels::ArrivalAndWalk)) never
    /// need this call. Labels carrying tables —
    /// [`ArrivalAndFare`](crate::labels::ArrivalAndFare) with a
    /// route → fare lookup, your own custom `Label` with a per-route
    /// preference map — chain `.with_context(ctx)` before
    /// `.depart_at(...)` / `.depart_in_window(...)`. The context
    /// outlives the query call, not the label values: labels
    /// produced during the scan only need to *carry* the criterion
    /// values they read out of `ctx`, never the table itself.
    ///
    /// ```no_run
    /// use std::collections::HashMap;
    /// use vulture::{RouteIdx, SecondOfDay, StopIdx, Timetable};
    /// use vulture::labels::{ArrivalAndFare, FareTable};
    ///
    /// # fn run<T: Timetable>(tt: &T, start: StopIdx, target: StopIdx, premium: RouteIdx) {
    /// let mut per_route = HashMap::new();
    /// per_route.insert(premium, 500);
    /// let fares = FareTable { per_route };
    ///
    /// let journeys = tt
    ///     .query_with_label::<ArrivalAndFare>()
    ///     .with_context(fares)
    ///     .from(start)
    ///     .to(target)
    ///     .max_transfers(5)
    ///     .depart_at(SecondOfDay::hms(9, 0, 0))
    ///     .run();
    /// # let _ = journeys;
    /// # }
    /// ```
    ///
    /// For range queries (`.depart_in_window(...).run_par()`) the
    /// parallel path requires `L::Ctx: Sync`; in practice every
    /// natural `Ctx` shape (a [`HashMap`](std::collections::HashMap),
    /// a flat `Vec`-backed table, etc.) already satisfies it.
    pub fn with_context(mut self, ctx: L::Ctx) -> Self {
        self.ctx = ctx;
        self
    }

    /// Restrict the algorithm to wheelchair-accessible trips and stops.
    ///
    /// With this set, trips for which
    /// [`Timetable::trip_wheelchair_accessible`] returns `false` are
    /// skipped at boarding (the algorithm walks forward to the next
    /// trip on the route), and stops for which
    /// [`Timetable::stop_wheelchair_accessible`] returns `false` cannot
    /// be alighting points. Default GTFS semantics treat
    /// `wheelchair_accessible = 2` (no accommodations) and
    /// `wheelchair_boarding = 2` (not accessible) as failing the
    /// requirement; "no info" and "some accessibility" both pass.
    ///
    /// Composes with the existing [`Label`](crate::Label) machinery —
    /// a wheelchair-aware journey is still arrival-time-optimal within
    /// the filtered subnetwork.
    pub fn require_wheelchair_accessible(mut self) -> Self {
        self.require_wheelchair_accessible = true;
        self
    }
}

// ----- Stage 2a: SingleDeparture – terminal `.run()` -----

impl<'tt, T, L> Query<'tt, T, L, SingleDeparture>
where
    T: Timetable + Sized,
    L: Label,
{
    /// Execute the query, allocating a fresh [`RaptorCache`].
    pub fn run(self) -> Vec<Journey<L>> {
        let mut cache = RaptorCache::<L>::for_timetable(self.tt);
        self.run_with_cache(&mut cache)
    }

    /// Execute the query, reusing `cache`.
    ///
    /// # Panics
    ///
    /// Panics if `cache` was sized for a different timetable
    /// (`tt.n_stops()` or `tt.n_routes()` mismatch). Use
    /// [`RaptorCache::for_timetable`] with the same timetable you call
    /// the query on to avoid this.
    pub fn run_with_cache(self, cache: &mut RaptorCache<L>) -> Vec<Journey<L>> {
        run_per_call_query(
            self.tt,
            &self.ctx,
            cache,
            self.max_transfers.0 as usize,
            self.mode.at,
            self.origins,
            self.targets,
            self.require_wheelchair_accessible,
        )
    }
}

// ----- Stage 2b: RangeDeparture, ArrivalTime – rRAPTOR -----

impl<'tt, T> Query<'tt, T, ArrivalTime, RangeDeparture>
where
    T: Timetable + Sized,
{
    /// Execute the range query, allocating a fresh [`RaptorCache`].
    /// Runs rRAPTOR (single reverse-chronological scan reusing labels
    /// across departures).
    pub fn run(self) -> Vec<RangeJourney<ArrivalTime>> {
        let mut cache = RaptorCache::<ArrivalTime>::for_timetable(self.tt);
        self.run_with_cache(&mut cache)
    }

    /// Execute the range query, reusing `cache`. Runs rRAPTOR.
    ///
    /// # Panics
    ///
    /// Panics if `cache` was sized for a different timetable. See
    /// [`RaptorCache::for_timetable`].
    pub fn run_with_cache(
        self,
        cache: &mut RaptorCache<ArrivalTime>,
    ) -> Vec<RangeJourney<ArrivalTime>> {
        raptor_range_rrap_arrival(
            self.tt,
            cache,
            self.max_transfers.0 as usize,
            &self.mode.departures,
            self.origins,
            self.targets,
            self.require_wheelchair_accessible,
        )
    }
}

#[cfg(feature = "parallel")]
impl<'tt, T, L> Query<'tt, T, L, RangeDeparture>
where
    T: Timetable + Sized + Sync,
    L: Label + Send + Sync,
    L::Ctx: Sync,
{
    /// Execute the range query in parallel, allocating a fresh
    /// [`RaptorCachePool`]. Per-departure work fans out across Rayon's
    /// global thread pool. Output is identical to [`Self::run`].
    ///
    /// Available with the `parallel` feature (on by default). For
    /// repeated range queries against the same timetable, prefer
    /// [`Self::run_with_pool`] to amortise pool construction.
    pub fn run_par(self) -> Vec<RangeJourney<L>> {
        let pool = RaptorCachePool::<L>::for_timetable(self.tt);
        self.run_with_pool(&pool)
    }

    /// Execute the range query in parallel, reusing caches from `pool`.
    ///
    /// # Panics
    ///
    /// Panics if `pool` was sized for a different timetable
    /// (mismatch surfaces in the per-departure scan via
    /// [`RaptorCache::for_timetable`]'s sizing assertion).
    pub fn run_with_pool(self, pool: &RaptorCachePool<L>) -> Vec<RangeJourney<L>> {
        use rayon::prelude::*;

        let transfers = self.max_transfers.0 as usize;
        let origins = self.origins;
        let targets = self.targets;
        let tt = self.tt;
        let departures = self.mode.departures;
        let require_accessible = self.require_wheelchair_accessible;
        let ctx = self.ctx;

        let all: Vec<RangeJourney<L>> = departures
            .par_iter()
            .flat_map_iter(|&depart| {
                let mut cache = pool.checkout();
                let journeys = run_per_call_query(
                    tt,
                    &ctx,
                    &mut *cache,
                    transfers,
                    depart,
                    &origins,
                    &targets,
                    require_accessible,
                );
                journeys
                    .into_iter()
                    .map(move |j| RangeJourney { depart, journey: j })
            })
            .collect();

        filter_range_pareto_front(all)
    }
}
