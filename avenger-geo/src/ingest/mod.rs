//! GeoJSON / WKB ingest (implemented in phase 4 of scratch/geo).
//!
//! Planned surface (doc §7): GeoJSON FeatureCollection -> per-feature ISO
//! WKB bytes + lon/lat bbox side-values + typed property columns, with
//! spherical winding normalization; plus a zero-copy WKB -> `GeoStream`
//! streamer via geo-traits.
