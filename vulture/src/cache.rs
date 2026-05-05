//! [`RaptorCache`] – reusable scratch buffers for repeated queries —
//! plus the `Sync` freelist variant [`RaptorCachePool`] and its
//! [`PooledCache`] checkout guard.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::sync::Mutex;

use fixedbitset::FixedBitSet;

use crate::K;
use crate::Timetable;
use crate::algorithm::boarding::BoardingTree;
use crate::algorithm::label_bag::LabelBag;
use crate::ids::RouteIdx;
use crate::ids::StopIdx;
use crate::label::ArrivalTime;
use crate::label::Label;
use crate::time::SecondOfDay;

/// Reusable scratch buffers for [`Query::run_with_cache`](crate::Query::run_with_cache).
///
/// `Query::run` allocates a fresh cache on every call. For workloads that
/// run many queries against the same timetable (a server, a batch job),
/// allocate a `RaptorCache` once and pass `&mut cache` to
/// [`Query::run_with_cache`](crate::Query::run_with_cache) at the end of each builder chain – the
/// timetable-sized buffers get reset rather than reallocated between queries.
///
/// ```no_run
/// # use vulture::{RaptorCache, SecondOfDay, Timetable};
/// # fn ex<T: Timetable>(tt: &T, queries: &[(vulture::StopIdx, vulture::StopIdx)]) {
/// let mut cache = RaptorCache::for_timetable(tt);
/// for &(start, end) in queries {
///     let _ = tt.query()
///         .from(start)
///         .to(end)
///         .depart_at(SecondOfDay::hms(9, 0, 0))
///         .run_with_cache(&mut cache);
/// }
/// # }
/// ```
///
/// A cache is sized for a specific timetable's `n_stops` / `n_routes`;
/// passing it to a query against a differently-sized timetable panics on
/// entry. Use [`RaptorCache::with_capacity`] when the timetable isn't yet in
/// scope.
///
/// `RaptorCache` is `!Sync` – give each worker its own, or use
/// [`RaptorCachePool`] (the `Sync` freelist variant).
pub struct RaptorCache<L: Label = ArrivalTime> {
    pub(crate) n_stops: u32,
    pub(crate) n_routes: u32,

    /// labels[k][stop.idx()] = Pareto bag of labels at stop with at
    /// most k trips. Empty bag means unreached.
    pub(crate) labels: Vec<Vec<LabelBag<L>>>,

    /// τ* – Pareto bag of best labels at each stop across all rounds.
    pub(crate) best_arrival: Vec<LabelBag<L>>,

    /// Boarding tree for journey reconstruction.
    pub(crate) board_detail: BoardingTree,

    /// Bitset of marked stops, sized to n_stops.
    pub(crate) marked_stops: FixedBitSet,

    /// Per-round route queue. `q_entry[r.idx()] = Some(boarding_pos)` when
    /// route r has been entered this round; `q_routes` is the dense list
    /// of routes that have entries (for cheap iteration without scanning
    /// `n_routes` empty slots).
    pub(crate) q_entry: Vec<Option<u32>>,
    pub(crate) q_routes: Vec<RouteIdx>,

    /// Scratch buffer for footpath relaxation output.
    pub(crate) walked_buf: Vec<StopIdx>,

    /// Bitset of origin stops for the current query. Reconstruction
    /// terminates when the trace reaches any bit set here.
    pub(crate) origin_set: FixedBitSet,

    /// Min-heap reused across footpath relaxations. Entries are
    /// `(arrival_time, stop_bit)`; lazy deletion via the time field.
    pub(crate) relax_heap: BinaryHeap<Reverse<(SecondOfDay, u32)>>,

    /// Bitset of stops with a non-empty bag in any round seen so
    /// far this query. Used to make per-round carry-forward sparse:
    /// only clone bags at set bits, not all `n_stops`.
    pub(crate) ever_reached: FixedBitSet,
}

impl<L: Label> RaptorCache<L> {
    /// Constructs a cache sized for the given timetable.
    pub fn for_timetable<T: Timetable + ?Sized>(tt: &T) -> Self {
        Self::with_capacity(tt.n_stops() as u32, tt.n_routes() as u32)
    }

    /// Constructs a cache for a timetable with the given counts. Use
    /// [`for_timetable`](Self::for_timetable) when you have the timetable
    /// in scope.
    pub fn with_capacity(n_stops: u32, n_routes: u32) -> Self {
        Self {
            n_stops,
            n_routes,
            labels: Vec::new(),
            best_arrival: (0..n_stops).map(|_| LabelBag::new()).collect(),
            board_detail: BTreeMap::new(),
            marked_stops: FixedBitSet::with_capacity(n_stops as usize),
            q_entry: vec![None; n_routes as usize],
            q_routes: Vec::new(),
            walked_buf: Vec::new(),
            origin_set: FixedBitSet::with_capacity(n_stops as usize),
            relax_heap: BinaryHeap::new(),
            ever_reached: FixedBitSet::with_capacity(n_stops as usize),
        }
    }

    pub(crate) fn reset_for_query(&mut self, transfers: K, tt_n_stops: u32, tt_n_routes: u32) {
        assert_eq!(
            self.n_stops, tt_n_stops,
            "RaptorCache sized for {} stops but timetable has {}",
            self.n_stops, tt_n_stops
        );
        assert_eq!(
            self.n_routes, tt_n_routes,
            "RaptorCache sized for {} routes but timetable has {}",
            self.n_routes, tt_n_routes
        );

        // Resize labels: (transfers + 1) Vecs, each n_stops long, all empty bags.
        let needed = transfers + 1;
        for v in self.labels.iter_mut() {
            for b in v.iter_mut() {
                b.clear();
            }
        }
        if self.labels.len() < needed {
            self.labels.resize_with(needed, || {
                (0..self.n_stops).map(|_| LabelBag::new()).collect()
            });
        } else {
            self.labels.truncate(needed);
        }

        for v in &mut self.best_arrival {
            v.clear();
        }

        self.board_detail.clear();
        self.marked_stops.clear();
        self.ever_reached.clear();

        // Sparse-set reset: walk q_routes, clear corresponding q_entry slots.
        for r in self.q_routes.drain(..) {
            self.q_entry[r.idx()] = None;
        }

        self.walked_buf.clear();
    }
}

/// A `Sync` pool of [`RaptorCache`]s sized for one timetable. Hand it
/// to multiple threads (or a Rayon worker pool) and have each thread
/// `checkout()` a cache for the duration of one query.
///
/// Backed by a mutex-protected freelist; the lock is only held long
/// enough to pop or push, never during the query itself. Caches grow
/// lazily – the pool starts empty and allocates a fresh cache when a
/// thread checks out and the freelist is empty. Returned caches are
/// reused by the next checkout.
///
/// Construct with [`RaptorCachePool::for_timetable`] (or
/// [`RaptorCachePool::with_capacity`] when you don't have the timetable
/// in scope yet). Like [`RaptorCache`] itself, a pool is sized for one
/// specific timetable's `n_stops`/`n_routes` and panics on dimension
/// mismatch when its caches are used.
///
/// ```no_run
/// # use vulture::{RaptorCachePool, SecondOfDay, Timetable};
/// # fn ex<T: Timetable + Sync>(tt: &T, queries: &[(vulture::StopIdx, vulture::StopIdx)]) {
/// let pool = RaptorCachePool::for_timetable(tt);
/// // Sequential or parallel – same code:
/// for &(start, end) in queries {
///     let mut cache = pool.checkout();
///     let _ = tt.query()
///         .from(start)
///         .to(end)
///         .depart_at(SecondOfDay::hms(9, 0, 0))
///         .run_with_cache(&mut cache);
/// }
/// # }
/// ```
pub struct RaptorCachePool<L: Label = ArrivalTime> {
    n_stops: u32,
    n_routes: u32,
    pool: Mutex<Vec<RaptorCache<L>>>,
}

impl<L: Label> RaptorCachePool<L> {
    /// Constructs a pool sized for the given timetable. The pool starts
    /// empty; caches are allocated on first checkout.
    pub fn for_timetable<T: Timetable + ?Sized>(tt: &T) -> Self {
        Self::with_capacity(tt.n_stops() as u32, tt.n_routes() as u32)
    }

    /// Constructs a pool for a timetable with the given counts. Use
    /// [`for_timetable`](Self::for_timetable) when you have the timetable
    /// in scope.
    pub fn with_capacity(n_stops: u32, n_routes: u32) -> Self {
        Self {
            n_stops,
            n_routes,
            pool: Mutex::new(Vec::new()),
        }
    }

    /// Borrow a cache from the pool. Allocates a fresh one if the pool
    /// is empty. The returned guard returns the cache to the pool when
    /// dropped, ready for the next checkout.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (i.e. another thread
    /// panicked while holding it). In normal use this does not occur:
    /// the lock is only held long enough to pop or push the freelist,
    /// never across a query.
    pub fn checkout(&self) -> PooledCache<'_, L> {
        let cache = self
            .pool
            .lock()
            .expect("RaptorCachePool mutex poisoned")
            .pop()
            .unwrap_or_else(|| RaptorCache::with_capacity(self.n_stops, self.n_routes));
        PooledCache {
            pool: self,
            cache: Some(cache),
        }
    }
}

impl<L: Label> std::fmt::Debug for RaptorCachePool<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.pool.lock().map(|p| p.len()).unwrap_or(0);
        f.debug_struct("RaptorCachePool")
            .field("n_stops", &self.n_stops)
            .field("n_routes", &self.n_routes)
            .field("idle_caches", &len)
            .finish()
    }
}

/// RAII guard handed out by [`RaptorCachePool::checkout`]. Derefs to
/// [`RaptorCache`]; pass `&mut pooled_cache` wherever a `&mut RaptorCache`
/// is wanted. Returns the cache to the pool on drop.
pub struct PooledCache<'p, L: Label = ArrivalTime> {
    pool: &'p RaptorCachePool<L>,
    cache: Option<RaptorCache<L>>,
}

impl<L: Label> std::ops::Deref for PooledCache<'_, L> {
    type Target = RaptorCache<L>;
    fn deref(&self) -> &RaptorCache<L> {
        self.cache.as_ref().expect("PooledCache used after drop")
    }
}

impl<L: Label> std::ops::DerefMut for PooledCache<'_, L> {
    fn deref_mut(&mut self) -> &mut RaptorCache<L> {
        self.cache.as_mut().expect("PooledCache used after drop")
    }
}

impl<L: Label> Drop for PooledCache<'_, L> {
    fn drop(&mut self) {
        if let Some(cache) = self.cache.take() {
            // Best-effort return; if the mutex is poisoned, drop the cache.
            if let Ok(mut pool) = self.pool.pool.lock() {
                pool.push(cache);
            }
        }
    }
}
