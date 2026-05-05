//! [`Endpoints`] is the value the algorithm consumes for a query's origin
//! and target sets. The [`IntoEndpoints`] trait converts the natural input
//! shapes (a single stop, a slice, an owned `Vec`, with or without per-stop
//! walk-time offsets) into one.

use smallvec::SmallVec;

use crate::ids::StopIdx;
use crate::time::Duration;

/// A bag of `(stop, walk-time-offset)` pairs used as the origin or
/// target of a query.
///
/// Constructed via the [`IntoEndpoints`] trait, which is implemented
/// for the natural input shapes – a single stop, a slice of stops,
/// a slice of `(stop, duration)` pairs, the owning `Vec` forms.
/// Most callers don't construct `Endpoints` directly; they pass
/// whatever they have to a query method that takes `impl IntoEndpoints`.
///
/// Stored inline up to 50 entries – covers the typical
/// parent-station case (the largest real example we've seen is
/// Berlin Hauptbahnhof at ~301 child platforms, which spills to the
/// heap; everything else stays inline).
#[derive(Debug, Clone, Default)]
pub struct Endpoints {
    stops: SmallVec<[(StopIdx, Duration); 50]>,
}

impl Endpoints {
    /// Construct empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of `(stop, walk)` pairs.
    pub fn len(&self) -> usize {
        self.stops.len()
    }

    /// True iff this endpoint set has no stops.
    pub fn is_empty(&self) -> bool {
        self.stops.is_empty()
    }

    /// Borrow the underlying `(stop, walk)` slice.
    pub fn as_slice(&self) -> &[(StopIdx, Duration)] {
        &self.stops
    }

    /// Push a single `(stop, walk)` pair.
    pub fn push(&mut self, stop: StopIdx, walk: Duration) {
        self.stops.push((stop, walk));
    }
}

/// Convert any of the natural query-endpoint inputs into an
/// [`Endpoints`] value the algorithm can consume.
///
/// Implemented for:
/// - `StopIdx` – a single stop, walk-time = 0.
/// - `(StopIdx, Duration)` – a single stop with explicit walk-time.
/// - `&[StopIdx]`, `&[StopIdx; N]` – multiple stops, all walk = 0.
/// - `&[(StopIdx, Duration)]`, `&[(StopIdx, Duration); N]` – multiple
///   stops with explicit walks.
/// - `Vec<StopIdx>`, `Vec<(StopIdx, Duration)>` – owned variants.
/// - `&Endpoints`, `Endpoints` – already-built value, identity.
///
/// For anything else (an iterator, your own collection type),
/// collect into one of the above first or build an [`Endpoints`]
/// directly via [`Endpoints::push`].
pub trait IntoEndpoints {
    /// Convert to an [`Endpoints`] value.
    fn into_endpoints(self) -> Endpoints;
}

impl IntoEndpoints for StopIdx {
    fn into_endpoints(self) -> Endpoints {
        let mut e = Endpoints::new();
        e.push(self, Duration::ZERO);
        e
    }
}

impl IntoEndpoints for (StopIdx, Duration) {
    fn into_endpoints(self) -> Endpoints {
        let mut e = Endpoints::new();
        e.push(self.0, self.1);
        e
    }
}

impl IntoEndpoints for &[StopIdx] {
    fn into_endpoints(self) -> Endpoints {
        let mut e = Endpoints::new();
        for &s in self {
            e.push(s, Duration::ZERO);
        }
        e
    }
}

impl<const N: usize> IntoEndpoints for &[StopIdx; N] {
    fn into_endpoints(self) -> Endpoints {
        (self.as_slice()).into_endpoints()
    }
}

impl IntoEndpoints for &[(StopIdx, Duration)] {
    fn into_endpoints(self) -> Endpoints {
        let mut e = Endpoints::new();
        for &p in self {
            e.push(p.0, p.1);
        }
        e
    }
}

impl<const N: usize> IntoEndpoints for &[(StopIdx, Duration); N] {
    fn into_endpoints(self) -> Endpoints {
        (self.as_slice()).into_endpoints()
    }
}

impl IntoEndpoints for Vec<StopIdx> {
    fn into_endpoints(self) -> Endpoints {
        (self.as_slice()).into_endpoints()
    }
}

impl IntoEndpoints for &Vec<StopIdx> {
    fn into_endpoints(self) -> Endpoints {
        (self.as_slice()).into_endpoints()
    }
}

impl IntoEndpoints for Vec<(StopIdx, Duration)> {
    fn into_endpoints(self) -> Endpoints {
        let mut e = Endpoints::new();
        for p in self {
            e.push(p.0, p.1);
        }
        e
    }
}

impl IntoEndpoints for &Vec<(StopIdx, Duration)> {
    fn into_endpoints(self) -> Endpoints {
        (self.as_slice()).into_endpoints()
    }
}

impl IntoEndpoints for &Endpoints {
    fn into_endpoints(self) -> Endpoints {
        self.clone()
    }
}

impl IntoEndpoints for Endpoints {
    fn into_endpoints(self) -> Endpoints {
        self
    }
}
