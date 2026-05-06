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

    /// Build an [`Endpoints`] from a slice of raw `u32` stop indices,
    /// every entry getting walk-time zero. Companion to
    /// [`Endpoints::from_pairs`] for the simpler "any of these stops"
    /// case.
    ///
    /// Primarily here for FFI consumers — Python and JS bindings
    /// typically receive a `list[int]` / `Array<number>` decoded as
    /// `Vec<u32>`, and the [`IntoEndpoints`] trait is invisible
    /// across the language boundary. In-Rust callers can pass
    /// `&[StopIdx]` directly to query methods via [`IntoEndpoints`]
    /// instead of going through this factory.
    pub fn from_stop_indices(stops: &[u32]) -> Self {
        let mut e = Self::new();
        for &s in stops {
            e.push(StopIdx::new(s), Duration::ZERO);
        }
        e
    }

    /// Build an [`Endpoints`] from a slice of raw `(stop_idx, walk_seconds)`
    /// pairs. Inverse of [`Endpoints::as_slice`] modulo the typed
    /// wrappers — accepting `(u32, u32)` rather than `(StopIdx, Duration)`
    /// so the input shape matches what FFI bindings naturally decode
    /// into (`list[tuple[int, int]]` / `Array<[number, number]>`).
    ///
    /// In-Rust callers should prefer passing `&[(StopIdx, Duration)]`
    /// or `Vec<(StopIdx, Duration)>` to query methods via
    /// [`IntoEndpoints`].
    pub fn from_pairs(pairs: &[(u32, u32)]) -> Self {
        let mut e = Self::new();
        for &(s, w) in pairs {
            e.push(StopIdx::new(s), Duration(w));
        }
        e
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_stop_indices_assigns_zero_walk() {
        let e = Endpoints::from_stop_indices(&[3, 7, 11]);
        assert_eq!(e.len(), 3);
        let pairs: Vec<_> = e.as_slice().to_vec();
        assert_eq!(pairs[0], (StopIdx::new(3), Duration::ZERO));
        assert_eq!(pairs[1], (StopIdx::new(7), Duration::ZERO));
        assert_eq!(pairs[2], (StopIdx::new(11), Duration::ZERO));
    }

    #[test]
    fn from_pairs_preserves_walk_offsets() {
        let e = Endpoints::from_pairs(&[(0, 0), (5, 60), (12, 240)]);
        assert_eq!(e.len(), 3);
        let pairs: Vec<_> = e.as_slice().to_vec();
        assert_eq!(pairs[0], (StopIdx::new(0), Duration(0)));
        assert_eq!(pairs[1], (StopIdx::new(5), Duration(60)));
        assert_eq!(pairs[2], (StopIdx::new(12), Duration(240)));
    }

    #[test]
    fn from_stop_indices_empty_slice() {
        let e = Endpoints::from_stop_indices(&[]);
        assert!(e.is_empty());
    }
}
