//! Triangle meshes that draw Web Mercator tiles on a projection.
//!
//! A tile's texture is uniform in Web Mercator, and a projection bends it, so a mesh carries the
//! texture across the tile in pieces small enough that the bend between vertices stays within the
//! projector's precision, as resampling keeps lines within it. The design follows OpenLayers'
//! raster reprojection, which subdivides each tile until the error falls under a threshold.
//!
//! Adjacent tiles meet without cracks because they share their edges' vertices exactly:
//!
//! - Every vertex lies on one lattice over the whole Web Mercator square, whose coordinates are
//!   exact in `f64`, and one mesher projects each lattice point once for every tile.
//! - Each tile picks a grid level from its own error, and an edge between two tiles takes the
//!   finer of their levels. The coarser tile fans its cells along that edge to the finer edge's
//!   vertices.
//! - Where the projection's own edge crosses a cell, the azimuthal horizon or the antimeridian's
//!   cut, the cell is cut at crossings that bisection finds on each crossed segment, run from the
//!   segment's lower lattice point, so the cells on both sides of a segment find the same
//!   crossing.
//!
//! The mesh follows the projection's own edge, widened to a circle that reaches as far as a
//! caller's circle or region clip that reaches past it, so a caller's clip lies within it: the
//! layer's clip to the map's outline draws those, as it draws the clip extent. The
//! map's edge follows the boundary within the precision: a chord along the horizon or the
//! antimeridian gains boundary points until it does, and every boundary vertex moves outward by
//! the precision, so the outline clip, not the mesh, draws the edge.
//!
//! A cut projection draws each pole of its rotated frame as a line or a point, and a cell around
//! a pole reaches it in every direction. Such a cell draws as sectors, one for each of its edges,
//! that reach from the edge to the pole's line, with an interior vertex on each sector's middle
//! direction to follow the texture there.
//!
//! An azimuthal projection that shows nearly the whole sphere spreads the cells near the
//! antipode of its center around the map's edge, where the chords between a cell's corners cut
//! across the map. The mesh stops short of the antipode, at a circle where the finest grid still
//! follows the projection within the precision, and the sphere's fill shows in the ring beyond
//! it.

use super::{column, latitude, longitude, row, TileId};
use crate::error::AvengerGeoError;
use crate::math::{HALF_PI, PI, TAU};
use crate::projector::Projector;
use rustc_hash::FxHashMap;

/// The finest grid: `2^MAX_LEVEL` cells along a tile's side.
const MAX_LEVEL: u32 = 6;

/// The lattice's depth below a tile: one level under the finest grid, for its cells' centers and
/// edge midpoints.
const DEPTH: u32 = MAX_LEVEL + 1;

/// The bisection steps that find where a segment crosses a boundary: they put the crossing
/// within 2^-24 of the segment's length, far below a pixel for any cell.
const CROSSING_STEPS: u32 = 24;

/// The most halvings of a chord along the map's edge.
const BOUNDARY_DEPTH: u32 = 8;

/// The widest sector, in radians of the frame's longitude, that a cell around a frame pole draws
/// without an interior vertex.
const MAX_SECTOR: f64 = PI / 16.0;

/// The most halvings of a sector around a frame pole.
const SECTOR_DEPTH: u32 = 6;

/// The latitude in degrees where the Web Mercator square ends.
const MERCATOR_LATITUDE: f64 = 85.051_128_779_806_59;

/// A tile's mesh, ready for a mesh image instance: positions in output units, texture
/// coordinates over the tile with `v = 0` at its north edge, and triangles, three indices each.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TileMesh {
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A lattice point: column and row at the lattice's depth for the mesher's zoom.
type Lattice = (u64, u64);

/// A lattice point through the projector.
#[derive(Debug, Clone, Copy)]
struct Point {
    /// The point in the spherical clip's frame, in radians.
    frame: (f64, f64),
    /// The point in output units, where the clip keeps it and its projection is finite.
    at: Option<[f64; 2]>,
}

/// The boundary that a segment crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Boundary {
    /// The projection's own spherical clip: an azimuthal horizon, or the edge of a projection
    /// whose points stop being finite.
    Clip,
    /// The frame's antimeridian, approached from its east end, at a longitude of π.
    East,
    /// The frame's antimeridian, approached from its west end, at a longitude of -π.
    West,
    /// The frame's prime meridian, which isn't an edge of the map: both sides share it.
    Meridian,
}

/// A mesh vertex: a lattice point, or where a segment between two lattice points crosses a
/// boundary, the segment's lower lattice point first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Vertex {
    Lattice(Lattice),
    Crossing {
        segment: (Lattice, Lattice),
        boundary: Boundary,
    },
}

/// A point on a boundary: its normalized Web Mercator coordinates, its frame coordinates, and
/// its position in output units, before it moves outward.
#[derive(Debug, Clone, Copy)]
struct OnBoundary {
    source: [f64; 2],
    frame: (f64, f64),
    position: [f64; 2],
}

/// A corner of a piece of a cell: a mesh vertex, and the lattice point that stands for the side
/// of the boundary it is on.
#[derive(Debug, Clone, Copy)]
struct Corner {
    vertex: Vertex,
    lattice: Lattice,
}

/// A pole of the rotated frame inside the Web Mercator square, which a cut projection draws as a
/// line or a point.
#[derive(Debug, Clone, Copy)]
struct Pole {
    /// The pole's latitude in the frame: π / 2 or -π / 2.
    phi: f64,
    /// The pole's normalized Web Mercator coordinates.
    source: [f64; 2],
}

/// An end of a sector around a frame pole: a corner of the cell's outline, or a point inside the
/// cell, with its direction from the pole, the frame's longitude.
#[derive(Debug, Clone, Copy)]
struct Ray {
    item: Item,
    lambda: f64,
    source: [f64; 2],
    /// The antimeridian's side that a crossing of the cut lies on, which the sector's edge from
    /// the crossing to the pole follows.
    cut: Option<Boundary>,
}

/// Meshes the Web Mercator tiles of one zoom for one projector. Use one mesher for every tile of
/// a view, so that the tiles share their edges' vertices.
pub struct TileMesher<'a> {
    projector: &'a Projector,
    zoom: u8,
    tolerance: f64,
    cuts: bool,
    circle: bool,
    /// The cosine of the angle from the frame's origin of the circle that the mesh follows, for
    /// a circle clip.
    reach: Option<f64>,
    origin: [f64; 2],
    poles: Vec<Pole>,
    points: FxHashMap<Lattice, Point>,
    crossings: FxHashMap<((Lattice, Lattice), Boundary), Option<OnBoundary>>,
    levels: FxHashMap<TileId, u32>,
}

impl<'a> TileMesher<'a> {
    /// A mesher for the tiles of zoom `zoom` on `projector`, which follows the projection to the
    /// projector's precision. A precision of 0 keeps the coarsest grids, as it keeps lines'
    /// vertices without resampling.
    ///
    /// # Errors
    /// Returns an error for a planar projection, which has no longitudes and latitudes, and for a
    /// zoom beyond [`super::MAX_ZOOM`].
    pub fn new(projector: &'a Projector, zoom: u8) -> Result<Self, AvengerGeoError> {
        if projector.is_planar() {
            return Err(AvengerGeoError::InvalidConfig(
                "a planar projection can't draw Web Mercator tiles".into(),
            ));
        }
        if zoom > super::MAX_ZOOM {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "zoom {zoom} is beyond the deepest zoom, {}",
                super::MAX_ZOOM
            )));
        }
        let cuts = projector.domain_cuts();
        // A cut projection's frame poles that tiles cover and that project to finite points.
        let poles = [HALF_PI, -HALF_PI]
            .into_iter()
            .filter(|_| cuts)
            .filter_map(|phi| {
                let (lon, lat) = projector.frame_to_lon_lat(0.0, phi);
                let finite = [0.0, PI, -PI]
                    .iter()
                    .all(|&lambda| projector.project_frame(lambda, phi).is_some());
                (lat.abs() < MERCATOR_LATITUDE && finite).then(|| Pole {
                    phi,
                    source: [column(lon), row(lat)],
                })
            })
            .collect();
        let reach = projector
            .domain_circle()
            .map(|angle| angle.min(PI - antipode_gap(projector, zoom, angle)).cos());
        Ok(Self {
            projector,
            zoom,
            tolerance: projector.precision(),
            cuts,
            circle: reach.is_some(),
            reach,
            origin: projector.frame_origin(),
            poles,
            points: FxHashMap::default(),
            crossings: FxHashMap::default(),
            levels: FxHashMap::default(),
        })
    }

    /// The mesh of `tile`, which has no triangles where the projection's own edge leaves out the
    /// whole tile.
    ///
    /// # Errors
    /// Returns an error for a tile of another zoom than the mesher's.
    pub fn mesh(&mut self, tile: TileId) -> Result<TileMesh, AvengerGeoError> {
        if tile.z != self.zoom {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "a mesher for zoom {} can't mesh tile {}/{}/{}",
                self.zoom, tile.z, tile.x, tile.y
            )));
        }
        let level = self.level(tile);
        // The level of each edge: north, east, south, west.
        let edges = [(0, -1), (1, 0), (0, 1), (-1, 0)].map(|(dx, dy)| {
            tile.neighbor(dx, dy)
                .map_or(level, |neighbor| self.level(neighbor).max(level))
        });
        let mut builder = Builder::new(tile);
        let cells = 1u64 << level;
        let step = 1u64 << (DEPTH - level);
        for row in 0..cells {
            for column in 0..cells {
                let polygon = cell_polygon(tile, [column, row], step, cells, edges);
                self.fill_cell(&mut builder, tile, [column, row], step, &polygon);
            }
        }
        Ok(builder.finish())
    }

    /// A tile's grid level: the coarsest at which every cell draws its edges' midpoints and its
    /// center within the precision of where its triangles put them. Along the map's edge, a cell
    /// also draws the points halfway from its kept corners to where its edges leave the map within
    /// the precision. Cells span at most 22.5 degrees of longitude, as resampling divides arcs
    /// longer than 30 degrees, so that a bend that their midpoints miss still shows. A precision
    /// of 0 keeps the coarsest grids whose cells span at most 90 degrees, so that no cell
    /// reaches across a hemisphere.
    pub(super) fn level(&mut self, tile: TileId) -> u32 {
        if let Some(&level) = self.levels.get(&tile) {
            return level;
        }
        let widest: u32 = if self.tolerance > 0.0 { 4 } else { 2 };
        let coarsest = widest.saturating_sub(u32::from(tile.z)).min(MAX_LEVEL);
        let level = if self.tolerance > 0.0 {
            (coarsest..MAX_LEVEL)
                .find(|&level| self.fits(tile, level))
                .unwrap_or(MAX_LEVEL)
        } else {
            coarsest
        };
        self.levels.insert(tile, level);
        level
    }

    fn fits(&mut self, tile: TileId, level: u32) -> bool {
        // A cell's corners clockwise from its northwest corner, its edges' midpoints, and its
        // center, in halves of a cell.
        const OFFSETS: [(u64, u64); 9] = [
            (0, 0),
            (2, 0),
            (2, 2),
            (0, 2),
            (1, 0),
            (2, 1),
            (1, 2),
            (0, 1),
            (1, 1),
        ];
        // Each midpoint and the two points of the chord it should lie on: the edges, and the
        // diagonal from the first corner to the third, which the cell's triangles share.
        const CHECKS: [(usize, usize, usize); 5] =
            [(4, 0, 1), (5, 1, 2), (6, 2, 3), (7, 3, 0), (8, 0, 2)];
        let cells = 1u64 << level;
        let half = 1u64 << (DEPTH - level - 1);
        for row in 0..cells {
            for column in 0..cells {
                let [a, b] = [column * half * 2, row * half * 2];
                let lattices = OFFSETS.map(|(dx, dy)| local(tile, a + dx * half, b + dy * half));
                let points = lattices.map(|lattice| self.point(lattice));
                for (middle, i, j) in CHECKS {
                    if !self.together(&[points[i], points[j]]) {
                        continue;
                    }
                    if !self.together(&[points[i], points[j], points[middle]])
                        || self.off_chord(points[middle], points[i], points[j])
                    {
                        return false;
                    }
                }
                for i in 0..4 {
                    for (from, to) in [(i, (i + 1) % 4), ((i + 1) % 4, i)] {
                        let ends = [(lattices[from], points[from]), (lattices[to], points[to])];
                        if !self.fits_to_edge(ends) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// Whether a cell's edge from a kept corner draws the point halfway to where it leaves the
    /// map within the precision, or doesn't leave the map.
    fn fits_to_edge(&mut self, [(from, start), (to, end)]: [(Lattice, Point); 2]) -> bool {
        let Some(boundary) = self.leaving(start, end) else {
            return true;
        };
        let Some(crossing) = self.crossing(ordered(from, to), boundary) else {
            return true;
        };
        let halfway = self.project(midpoint(self.source(from), crossing.source));
        let Some(at) = halfway.at.filter(|_| self.together(&[start, halfway])) else {
            return false;
        };
        distance(at, midpoint(start.at.expect("kept"), crossing.position)) <= self.tolerance
    }

    /// The boundary that a segment from a kept point leaves the map at, seen from that point, or
    /// `None` where the segment doesn't leave the map or starts outside it.
    fn leaving(&self, start: Point, end: Point) -> Option<Boundary> {
        start.at?;
        if end.at.is_none() {
            Some(Boundary::Clip)
        } else if self.straddles(&[start, end]) {
            Some(if start.frame.0 >= 0.0 {
                Boundary::East
            } else {
                Boundary::West
            })
        } else {
            None
        }
    }

    /// Whether the clip keeps points and they lie on one side of the antimeridian's cut.
    fn together(&self, points: &[Point]) -> bool {
        points.iter().all(|point| point.at.is_some()) && !self.straddles(points)
    }

    /// Whether a kept point lies farther than the precision from the middle of a kept chord.
    fn off_chord(&self, middle: Point, a: Point, b: Point) -> bool {
        let at = |point: Point| point.at.expect("kept");
        distance(at(middle), midpoint(at(a), at(b))) > self.tolerance
    }

    /// Whether points of one cell lie on both sides of the frame's antimeridian, which cuts the
    /// map there.
    fn straddles(&self, points: &[Point]) -> bool {
        if !self.cuts {
            return false;
        }
        let (low, high) = points
            .iter()
            .map(|point| point.frame.0)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), lambda| {
                (low.min(lambda), high.max(lambda))
            });
        high - low > PI
    }

    /// Add a cell's triangles: two for a plain cell that the clip keeps whole, a fan from its
    /// center for one along a finer edge, and its kept pieces' triangles for one that a boundary
    /// crosses.
    fn fill_cell(
        &mut self,
        builder: &mut Builder,
        tile: TileId,
        [column, row]: [u64; 2],
        step: u64,
        polygon: &[Corner],
    ) {
        if let Some(pole) = self.pole_in(tile, [column, row], step) {
            if self.fill_polar(builder, polygon, pole) {
                return;
            }
        }
        let points: Vec<Point> = polygon
            .iter()
            .map(|corner| self.point(corner.lattice))
            .collect();
        if points.iter().all(|point| point.at.is_some()) && !self.straddles(&points) {
            let vertices: Vec<Vertex> = polygon.iter().map(|corner| corner.vertex).collect();
            if vertices.len() == 4 {
                builder.triangle(self, [vertices[0], vertices[1], vertices[2]]);
                builder.triangle(self, [vertices[0], vertices[2], vertices[3]]);
                return;
            }
            let center = local(tile, column * step + step / 2, row * step + step / 2);
            let mut with_center = points;
            with_center.push(self.point(center));
            if with_center.last().is_some_and(|point| point.at.is_some())
                && !self.straddles(&with_center)
            {
                let hub = Vertex::Lattice(center);
                for i in 0..vertices.len() {
                    let next = vertices[(i + 1) % vertices.len()];
                    builder.triangle(self, [hub, vertices[i], next]);
                }
            } else {
                let piece: Vec<Item> = vertices.into_iter().map(Item::Vertex).collect();
                builder.polygon(self, &piece);
            }
            return;
        }
        // The clip keeps some of the cell, and then the antimeridian may cut what it keeps.
        let kept = self.clip_kept(polygon);
        if kept.len() < 3 {
            return;
        }
        let kept_points: Vec<Point> = kept
            .iter()
            .map(|corner| self.point(corner.lattice))
            .collect();
        let pieces = if self.straddles(&kept_points) {
            vec![self.clip_side(&kept, true), self.clip_side(&kept, false)]
        } else {
            vec![kept.iter().map(|corner| corner.vertex).collect()]
        };
        for piece in pieces {
            if piece.len() >= 3 {
                let piece = self.along_boundaries(&piece);
                builder.polygon(self, &piece);
            }
        }
    }

    /// The frame pole in a cell, on its outline included, for a cut projection.
    fn pole_in(&self, tile: TileId, [column, row]: [u64; 2], step: u64) -> Option<Pole> {
        let [s0, t0] = self.source(local(tile, column * step, row * step));
        let [s1, t1] = self.source(local(tile, (column + 1) * step, (row + 1) * step));
        let tolerance = (s1 - s0) * 1e-9 + 4.0 * f64::EPSILON;
        self.poles.iter().copied().find(|pole| {
            let [s, t] = pole.source;
            // The square wraps at the antimeridian, so a pole at one side is at the other too.
            [s - 1.0, s, s + 1.0]
                .iter()
                .any(|&s| s > s0 - tolerance && s < s1 + tolerance)
                && t > t0 - tolerance
                && t < t1 + tolerance
        })
    }

    /// Whether a segment between lattice points touches a frame pole: passes through it or ends
    /// at it.
    fn touches(&self, a: Lattice, b: Lattice, pole: Pole) -> bool {
        let ([sa, ta], [sb, tb]) = (self.source(a), self.source(b));
        let tolerance = ((sb - sa).abs() + (tb - ta).abs()) * 1e-9 + 4.0 * f64::EPSILON;
        let [s, t] = pole.source;
        [s - 1.0, s, s + 1.0].iter().any(|&s| {
            // Segments of the lattice run along a column or a row.
            let (along, across) = if sa == sb {
                ((t, ta, tb), s - sa)
            } else {
                ((s, sa, sb), t - ta)
            };
            let (value, low, high) = (along.0, along.1.min(along.2), along.1.max(along.2));
            across.abs() <= tolerance && value > low - tolerance && value < high + tolerance
        })
    }

    /// Fill a cell around a frame pole of a cut projection as sectors from its edges to the
    /// pole's line, or return false where some of the cell doesn't project. An edge that
    /// touches the pole spans no sector.
    fn fill_polar(&mut self, builder: &mut Builder, polygon: &[Corner], pole: Pole) -> bool {
        let points: Vec<Point> = polygon
            .iter()
            .map(|corner| self.point(corner.lattice))
            .collect();
        if points.iter().any(|point| point.at.is_none()) {
            return false;
        }
        let n = polygon.len();
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            if self.touches(a.lattice, b.lattice, pole) {
                continue;
            }
            let ray = |corner: Corner, point: Point, source: [f64; 2]| Ray {
                item: Item::Vertex(corner.vertex),
                lambda: point.frame.0,
                source,
                cut: None,
            };
            let start = ray(a, points[i], self.source(a.lattice));
            let end = ray(b, points[(i + 1) % n], self.source(b.lattice));
            if (end.lambda - start.lambda).abs() <= PI {
                self.sector(builder, start, end, pole, SECTOR_DEPTH);
                continue;
            }
            // The edge crosses the cut: a sector reaches from each end to the cut.
            let segment = ordered(a.lattice, b.lattice);
            let side = |lambda: f64| {
                if lambda >= 0.0 {
                    Boundary::East
                } else {
                    Boundary::West
                }
            };
            let Some(crossing) = self.crossing(segment, side(start.lambda)) else {
                continue;
            };
            let at_cut = |boundary: Boundary| Ray {
                item: Item::Vertex(Vertex::Crossing { segment, boundary }),
                lambda: if boundary == Boundary::East { PI } else { -PI },
                source: crossing.source,
                cut: Some(boundary),
            };
            self.sector(
                builder,
                start,
                at_cut(side(start.lambda)),
                pole,
                SECTOR_DEPTH,
            );
            self.sector(builder, at_cut(side(end.lambda)), end, pole, SECTOR_DEPTH);
        }
        true
    }

    /// The part of a cell around a frame pole between two rays from the pole, from the chord
    /// between the rays' ends to the pole's line. A wide sector draws the triangle between the
    /// chord and a point halfway along its middle ray to the pole, and the two sectors beside
    /// that point.
    fn sector(&mut self, builder: &mut Builder, start: Ray, end: Ray, pole: Pole, depth: u32) {
        let (low, high) = (start.lambda.min(end.lambda), start.lambda.max(end.lambda));
        if depth > 0 && self.tolerance > 0.0 && high - low > MAX_SECTOR {
            let middle = self.project(midpoint(start.source, end.source));
            // The middle direction lies between the rays, on their side of the cut.
            let lambda = (start.lambda + wrap(middle.frame.0 - start.lambda)).clamp(low, high);
            let phi = (middle.frame.1 + pole.phi) / 2.0;
            if let Some(inside) = self.frame_point(lambda, phi, start.source) {
                builder.polygon(self, &[start.item, end.item, Item::Inside(inside)]);
                let ray = Ray {
                    item: Item::Inside(inside),
                    lambda,
                    source: inside.source,
                    cut: None,
                };
                self.sector(builder, start, ray, pole, depth - 1);
                self.sector(builder, ray, end, pole, depth - 1);
                return;
            }
        }
        let mut items = vec![start.item, end.item];
        let pole_at = |mesher: &Self, lambda: f64| {
            mesher
                .frame_point(lambda, pole.phi, start.source)
                .map(|point| OnBoundary {
                    source: [point.source[0], pole.source[1]],
                    ..point
                })
        };
        let (Some(first), Some(last)) = (pole_at(self, end.lambda), pole_at(self, start.lambda))
        else {
            return;
        };
        // Up the cut from a crossing at the sector's end, along the pole's line, and down the cut
        // to a crossing at its start.
        if let (Some(boundary), Item::Vertex(Vertex::Crossing { segment, .. })) =
            (end.cut, end.item)
        {
            if let Some(crossing) = self.crossing(segment, boundary) {
                let mut along = Vec::new();
                self.refine(crossing, first, boundary, BOUNDARY_DEPTH, &mut along);
                items.extend(along.into_iter().map(Item::Boundary));
            }
        }
        let mut line = vec![first];
        self.refine_pole(first, last, pole, BOUNDARY_DEPTH, &mut line);
        line.push(last);
        items.extend(line.into_iter().map(Item::Boundary));
        if let (Some(boundary), Item::Vertex(Vertex::Crossing { segment, .. })) =
            (start.cut, start.item)
        {
            if let Some(crossing) = self.crossing(segment, boundary) {
                let mut along = Vec::new();
                self.refine(last, crossing, boundary, BOUNDARY_DEPTH, &mut along);
                items.extend(along.into_iter().map(Item::Boundary));
            }
        }
        builder.polygon(self, &items);
    }

    /// Points along a frame pole's line between two of its points, halving the chord until the
    /// line lies within the precision of it.
    fn refine_pole(
        &self,
        start: OnBoundary,
        end: OnBoundary,
        pole: Pole,
        depth: u32,
        out: &mut Vec<OnBoundary>,
    ) {
        if depth == 0 || self.tolerance <= 0.0 {
            return;
        }
        let lambda = (start.frame.0 + end.frame.0) / 2.0;
        let Some(position) = self.projector.project_frame(lambda, pole.phi) else {
            return;
        };
        if distance(position, midpoint(start.position, end.position)) <= self.tolerance {
            return;
        }
        let middle = OnBoundary {
            frame: (lambda, pole.phi),
            position,
            ..start
        };
        self.refine_pole(start, middle, pole, depth - 1, out);
        out.push(middle);
        self.refine_pole(middle, end, pole, depth - 1, out);
    }

    /// A point of the frame, with its normalized Web Mercator coordinates continuing `near`'s
    /// across the antimeridian, or `None` where it doesn't project.
    fn frame_point(&self, lambda: f64, phi: f64, near: [f64; 2]) -> Option<OnBoundary> {
        let position = self.projector.project_frame(lambda, phi)?;
        let (lon, lat) = self.projector.frame_to_lon_lat(lambda, phi);
        let mut s = column(lon);
        s += ((near[0] - s) + 0.5).floor();
        Some(OnBoundary {
            source: [s, row(lat)],
            frame: (lambda, phi),
            position,
        })
    }

    /// The part of a cell's outline that the clip keeps: Sutherland-Hodgman against the clip,
    /// with a crossing where the outline leaves or enters it.
    fn clip_kept(&mut self, polygon: &[Corner]) -> Vec<Corner> {
        let mut kept = Vec::new();
        for i in 0..polygon.len() {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let keep_a = self.point(a.lattice).at.is_some();
            let keep_b = self.point(b.lattice).at.is_some();
            if keep_a {
                kept.push(a);
            }
            if keep_a != keep_b {
                kept.push(Corner {
                    vertex: Vertex::Crossing {
                        segment: ordered(a.lattice, b.lattice),
                        boundary: Boundary::Clip,
                    },
                    lattice: if keep_a { a.lattice } else { b.lattice },
                });
            }
        }
        kept
    }

    /// The part of a kept polygon on one side of the frame's antimeridian, its east end for
    /// `east` and its west end otherwise. A segment that jumps across the antimeridian gets a
    /// crossing for that side, and one that crosses the prime meridian gets a crossing that both
    /// sides share.
    fn clip_side(&mut self, kept: &[Corner], east: bool) -> Vec<Vertex> {
        let mut piece = Vec::new();
        for i in 0..kept.len() {
            let (a, b) = (kept[i], kept[(i + 1) % kept.len()]);
            let (lambda_a, lambda_b) =
                (self.point(a.lattice).frame.0, self.point(b.lattice).frame.0);
            let (in_a, in_b) = ((lambda_a >= 0.0) == east, (lambda_b >= 0.0) == east);
            if in_a {
                piece.push(a.vertex);
            }
            if in_a != in_b {
                let boundary = if (lambda_a - lambda_b).abs() <= PI {
                    Boundary::Meridian
                } else if east {
                    Boundary::East
                } else {
                    Boundary::West
                };
                piece.push(Vertex::Crossing {
                    segment: ordered(a.lattice, b.lattice),
                    boundary,
                });
            }
        }
        piece
    }

    /// A piece's corners, with boundary points between consecutive crossings of a circle clip or
    /// of the antimeridian, so the piece's edge along the map's edge follows it within the
    /// precision.
    fn along_boundaries(&mut self, piece: &[Vertex]) -> Vec<Item> {
        let mut items = Vec::with_capacity(piece.len());
        for i in 0..piece.len() {
            let (a, b) = (piece[i], piece[(i + 1) % piece.len()]);
            items.push(Item::Vertex(a));
            let (
                Vertex::Crossing {
                    segment: from,
                    boundary,
                },
                Vertex::Crossing {
                    segment: to,
                    boundary: other,
                },
            ) = (a, b)
            else {
                continue;
            };
            let follows = match boundary {
                Boundary::Clip => self.circle,
                Boundary::East | Boundary::West => true,
                Boundary::Meridian => false,
            };
            if !follows || boundary != other {
                continue;
            }
            let (Some(start), Some(end)) =
                (self.crossing(from, boundary), self.crossing(to, boundary))
            else {
                continue;
            };
            let mut between = Vec::new();
            self.refine(start, end, boundary, BOUNDARY_DEPTH, &mut between);
            items.extend(between.into_iter().map(Item::Boundary));
        }
        items
    }

    /// Points along a boundary between two of its points, halving the chord until the boundary
    /// lies within the precision of it.
    fn refine(
        &self,
        start: OnBoundary,
        end: OnBoundary,
        boundary: Boundary,
        depth: u32,
        out: &mut Vec<OnBoundary>,
    ) {
        if depth == 0 || self.tolerance <= 0.0 {
            return;
        }
        let Some(middle) = self.boundary_between(start, end, boundary) else {
            return;
        };
        if distance(middle.position, midpoint(start.position, end.position)) <= self.tolerance {
            return;
        }
        self.refine(start, middle, boundary, depth - 1, out);
        out.push(middle);
        self.refine(middle, end, boundary, depth - 1, out);
    }

    /// The boundary's point midway between two of its points: along the horizon's circle, or
    /// along the frame's antimeridian.
    fn boundary_between(
        &self,
        start: OnBoundary,
        end: OnBoundary,
        boundary: Boundary,
    ) -> Option<OnBoundary> {
        let frame = match boundary {
            Boundary::East | Boundary::West => {
                let lambda = if boundary == Boundary::East { PI } else { -PI };
                (lambda, (start.frame.1 + end.frame.1) / 2.0)
            }
            Boundary::Clip => {
                // The circle surrounds the frame's origin, the x axis: turn about it from one
                // point to the other by the shorter way.
                let [xa, ya, za] = cartesian(start.frame);
                let [xb, yb, zb] = cartesian(end.frame);
                let (ta, tb) = (za.atan2(ya), zb.atan2(yb));
                let t = ta + wrap(tb - ta) / 2.0;
                let x = (xa + xb) / 2.0;
                let rho = (1.0 - x * x).max(0.0).sqrt();
                let (y, z) = (rho * t.cos(), rho * t.sin());
                (y.atan2(x), z.clamp(-1.0, 1.0).asin())
            }
            Boundary::Meridian => unreachable!("the prime meridian is inside the map"),
        };
        self.frame_point(frame.0, frame.1, start.source)
    }

    /// A lattice point through the projector, projected once.
    fn point(&mut self, lattice: Lattice) -> Point {
        if let Some(&point) = self.points.get(&lattice) {
            return point;
        }
        let point = self.project(self.source(lattice));
        self.points.insert(lattice, point);
        point
    }

    /// A lattice point's normalized Web Mercator coordinates.
    fn source(&self, (column, row): Lattice) -> [f64; 2] {
        let n = (1u64 << (u32::from(self.zoom) + DEPTH)) as f64;
        [column as f64 / n, row as f64 / n]
    }

    /// A point of the Web Mercator square through the projector.
    fn project(&self, [s, t]: [f64; 2]) -> Point {
        let frame = self.projector.to_frame(longitude(s), latitude(t));
        let keeps = match self.reach {
            Some(reach) => frame.0.cos() * frame.1.cos() > reach,
            None => self.projector.domain_keeps(frame.0, frame.1),
        };
        let at = if keeps {
            self.projector.project_frame(frame.0, frame.1)
        } else {
            None
        };
        Point { frame, at }
    }

    /// Where a segment crosses a boundary, found once.
    fn crossing(&mut self, segment: (Lattice, Lattice), boundary: Boundary) -> Option<OnBoundary> {
        if let Some(&crossing) = self.crossings.get(&(segment, boundary)) {
            return crossing;
        }
        let crossing = self.find_crossing(segment, boundary);
        self.crossings.insert((segment, boundary), crossing);
        crossing
    }

    /// Where a segment crosses a boundary. The bisection runs from the segment's lower lattice
    /// point, so every cell that shares the segment finds the same crossing.
    fn find_crossing(
        &mut self,
        (low, high): (Lattice, Lattice),
        boundary: Boundary,
    ) -> Option<OnBoundary> {
        let [s0, t0] = self.source(low);
        let [s1, t1] = self.source(high);
        let at = |t: f64| [s0 + (s1 - s0) * t, t0 + (t1 - t0) * t];
        let start = self.point(low);
        let class = |point: &Point| match boundary {
            Boundary::Clip => point.at.is_some(),
            _ => point.frame.0 >= 0.0,
        };
        let start_class = class(&start);
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..CROSSING_STEPS {
            let middle = (lo + hi) / 2.0;
            if class(&self.project(at(middle))) == start_class {
                lo = middle;
            } else {
                hi = middle;
            }
        }
        let (near, far) = (self.project(at(lo)), self.project(at(hi)));
        match boundary {
            Boundary::Clip => {
                let (t, point) = if start_class { (lo, near) } else { (hi, far) };
                Some(OnBoundary {
                    source: at(t),
                    frame: point.frame,
                    position: point.at?,
                })
            }
            Boundary::Meridian => Some(OnBoundary {
                source: at(lo),
                frame: near.frame,
                position: near.at?,
            }),
            Boundary::East | Boundary::West => {
                let lambda = if boundary == Boundary::East { PI } else { -PI };
                let phi = (near.frame.1 + far.frame.1) / 2.0;
                Some(OnBoundary {
                    source: at(lo),
                    frame: (lambda, phi),
                    position: self.projector.project_frame(lambda, phi)?,
                })
            }
        }
    }

    /// A crossing's position, moved off the map by the precision, away from the frame's origin,
    /// which the horizon surrounds and the antimeridian's cut runs around. A crossing of the
    /// prime meridian is inside the map and stays.
    fn crossing_position(&self, boundary: Boundary, position: [f64; 2]) -> [f64; 2] {
        match boundary {
            Boundary::Meridian => position,
            _ => self.away(position, self.origin),
        }
    }

    /// A point of the map's edge, moved away from `from` by the precision.
    fn away(&self, position: [f64; 2], from: [f64; 2]) -> [f64; 2] {
        let offset = [position[0] - from[0], position[1] - from[1]];
        let length = offset[0].hypot(offset[1]);
        if length <= f64::EPSILON {
            return position;
        }
        let reach = self.tolerance / length;
        [
            position[0] + offset[0] * reach,
            position[1] + offset[1] * reach,
        ]
    }
}

/// A cell's outline, clockwise from its northwest corner in the tile's texture: its corners, and
/// the vertices of a finer edge along the tile's border.
fn cell_polygon(
    tile: TileId,
    [column, row]: [u64; 2],
    step: u64,
    cells: u64,
    edges: [u32; 4],
) -> Vec<Corner> {
    let [a0, b0] = [column * step, row * step];
    let [a1, b1] = [a0 + step, b0 + step];
    // The lattice step along each side: finer where the side is on a finer tile edge.
    let side_step = |edge: usize, on_border: bool| {
        if on_border {
            1u64 << (DEPTH - edges[edge])
        } else {
            step
        }
    };
    let north = side_step(0, row == 0);
    let east = side_step(1, column == cells - 1);
    let south = side_step(2, row == cells - 1);
    let west = side_step(3, column == 0);
    let mut corners = Vec::new();
    let mut push = |a: u64, b: u64| {
        let lattice = local(tile, a, b);
        corners.push(Corner {
            vertex: Vertex::Lattice(lattice),
            lattice,
        });
    };
    let mut a = a0;
    while a < a1 {
        push(a, b0);
        a += north;
    }
    let mut b = b0;
    while b < b1 {
        push(a1, b);
        b += east;
    }
    let mut a = a1;
    while a > a0 {
        push(a, b1);
        a -= south;
    }
    let mut b = b1;
    while b > b0 {
        push(a0, b);
        b -= west;
    }
    corners
}

/// A tile's lattice point at local column `a` and row `b`, each from 0 to `2^DEPTH`.
fn local(tile: TileId, a: u64, b: u64) -> Lattice {
    (
        (u64::from(tile.x) << DEPTH) + a,
        (u64::from(tile.y) << DEPTH) + b,
    )
}

/// A segment's ends, lower lattice point first.
fn ordered(a: Lattice, b: Lattice) -> (Lattice, Lattice) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// An angle in radians turned by whole turns into (-π, π].
/// How far short of the antipode of a circle clip's center the mesh stops, in radians. A finest
/// cell of angular size `c` at an angle `d` from the antipode spans about `c / d` radians around
/// it, which the map's edge, at a radius `r` from the center, spreads around its arc. The
/// diagonal that the cell's triangles share spans up to `√2 c / d`, and its chord misses the arc
/// by about `r (√2 c / d)^2 / 8`, which stays within the precision `p` where
/// `d >= c sqrt(r / (4 p))`. A clip that leaves out a cap as wide needs no gap.
fn antipode_gap(projector: &Projector, zoom: u8, angle: f64) -> f64 {
    let (_, lat) = projector.frame_to_lon_lat(PI, 0.0);
    let cell = TAU / f64::from(1u32 << (u32::from(zoom) + MAX_LEVEL))
        * lat
            .clamp(-MERCATOR_LATITUDE, MERCATOR_LATITUDE)
            .to_radians()
            .cos();
    let Some(edge) = projector.project_frame(angle, 0.0) else {
        return 0.0;
    };
    let radius = distance(edge, projector.frame_origin());
    // A mesh without resampling keeps the default precision's gap.
    let precision = if projector.precision() > 0.0 {
        projector.precision()
    } else {
        std::f64::consts::FRAC_1_SQRT_2
    };
    let gap = cell * (radius / (4.0 * precision)).sqrt();
    if gap > PI - angle {
        gap
    } else {
        0.0
    }
}

fn wrap(angle: f64) -> f64 {
    let angle = angle.rem_euclid(TAU);
    if angle > PI {
        angle - TAU
    } else {
        angle
    }
}

fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A point of the frame as a unit vector, with x toward the frame's origin.
fn cartesian((lambda, phi): (f64, f64)) -> [f64; 3] {
    let (sin_phi, cos_phi) = phi.clamp(-HALF_PI, HALF_PI).sin_cos();
    [lambda.cos() * cos_phi, lambda.sin() * cos_phi, sin_phi]
}

/// A corner of a piece to fill: a mesh vertex, or a point that belongs to the piece alone, along
/// the map's edge or inside the map.
#[derive(Debug, Clone, Copy)]
enum Item {
    Vertex(Vertex),
    Boundary(OnBoundary),
    Inside(OnBoundary),
}

/// Collects one tile's vertices, each once, and its triangles.
struct Builder {
    tile: TileId,
    positions: Vec<[f64; 2]>,
    uvs: Vec<[f64; 2]>,
    indices: Vec<u32>,
    index: FxHashMap<Vertex, Option<u32>>,
}

impl Builder {
    fn new(tile: TileId) -> Self {
        Self {
            tile,
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            index: FxHashMap::default(),
        }
    }

    /// Add a vertex at a position, with the texture coordinate of a source point.
    fn add(&mut self, source: [f64; 2], position: [f64; 2]) -> u32 {
        let n = f64::from(1u32 << self.tile.z);
        self.positions.push(position);
        self.uvs.push([
            source[0] * n - f64::from(self.tile.x),
            source[1] * n - f64::from(self.tile.y),
        ]);
        (self.positions.len() - 1) as u32
    }

    /// The index of a vertex, adding it on first use, or `None` for a vertex without a position.
    fn vertex(&mut self, mesher: &mut TileMesher, vertex: Vertex) -> Option<u32> {
        if let Some(&index) = self.index.get(&vertex) {
            return index;
        }
        let index = match vertex {
            Vertex::Lattice(lattice) => mesher
                .point(lattice)
                .at
                .map(|position| self.add(mesher.source(lattice), position)),
            Vertex::Crossing { segment, boundary } => {
                mesher.crossing(segment, boundary).map(|point| {
                    let position = mesher.crossing_position(boundary, point.position);
                    self.add(point.source, position)
                })
            }
        };
        self.index.insert(vertex, index);
        index
    }

    fn item(&mut self, mesher: &mut TileMesher, item: Item) -> Option<u32> {
        match item {
            Item::Vertex(vertex) => self.vertex(mesher, vertex),
            Item::Boundary(point) => {
                let position = mesher.away(point.position, mesher.origin);
                Some(self.add(point.source, position))
            }
            Item::Inside(point) => Some(self.add(point.source, point.position)),
        }
    }

    fn triangle(&mut self, mesher: &mut TileMesher, vertices: [Vertex; 3]) {
        let [Some(a), Some(b), Some(c)] = vertices.map(|vertex| self.vertex(mesher, vertex)) else {
            return;
        };
        if a != b && b != c && a != c {
            self.indices.extend([a, b, c]);
        }
    }

    /// Triangles that fill a piece's outline without overlapping, by ear clipping on the
    /// corners' positions, so a piece that isn't convex is filled too.
    fn polygon(&mut self, mesher: &mut TileMesher, piece: &[Item]) {
        let mut ring: Vec<u32> = Vec::with_capacity(piece.len());
        for &item in piece {
            if let Some(index) = self.item(mesher, item) {
                if ring.last() != Some(&index) {
                    ring.push(index);
                }
            }
        }
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        let at = |index: u32| self.positions[index as usize];
        let area: f64 = (0..ring.len())
            .map(|i| {
                let ([xa, ya], [xb, yb]) = (at(ring[i]), at(ring[(i + 1) % ring.len()]));
                xa * yb - xb * ya
            })
            .sum();
        if ring.len() < 3 || area == 0.0 {
            return;
        }
        let turn = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
            area.signum() * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
        };
        let mut triangles = Vec::new();
        while ring.len() > 3 {
            let n = ring.len();
            let corners = |i: usize| [ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]];
            let ear = (0..n).find(|&i| {
                let [a, b, c] = corners(i).map(at);
                turn(a, b, c) > 0.0
                    && ring.iter().all(|&k| {
                        corners(i).contains(&k) || {
                            let p = at(k);
                            turn(a, b, p) < 0.0 || turn(b, c, p) < 0.0 || turn(c, a, p) < 0.0
                        }
                    })
            });
            // Only an outline whose corners all lie on a line has no ear left.
            let Some(i) = ear else { break };
            triangles.push(corners(i));
            ring.remove(i);
        }
        for i in 1..ring.len().saturating_sub(1) {
            triangles.push([ring[0], ring[i], ring[i + 1]]);
        }
        for [a, b, c] in triangles {
            if a != b && b != c && a != c {
                self.indices.extend([a, b, c]);
            }
        }
    }

    fn finish(self) -> TileMesh {
        TileMesh {
            positions: self
                .positions
                .iter()
                .map(|&[x, y]| [x as f32, y as f32])
                .collect(),
            uvs: self
                .uvs
                .iter()
                .map(|&[u, v]| [u as f32, v as f32])
                .collect(),
            indices: self.indices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projections::EqualEarth;
    use crate::Projection;

    /// Meridians bend more toward an Equal Earth map's sides, so the tiles there get finer grids
    /// than the tiles at its center, and a precision of 0 keeps the coarsest grids.
    #[test]
    fn levels_follow_the_bend() {
        let world = |precision: f64| {
            Projection::new(EqualEarth)
                .with_view(150.0, [400.0, 200.0])
                .with_precision(precision)
                .build()
                .unwrap()
        };
        let projector = world(0.5_f64.sqrt());
        let mut mesher = TileMesher::new(&projector, 3).unwrap();
        let levels: Vec<u32> = (4..8)
            .map(|x| mesher.level(TileId::new(3, x, 3).unwrap()))
            .collect();
        assert!(
            levels.windows(2).all(|pair| pair[0] <= pair[1]),
            "{levels:?}"
        );
        assert!(levels[0] < levels[3], "{levels:?}");
        let flat = world(0.0);
        let mut mesher = TileMesher::new(&flat, 3).unwrap();
        assert_eq!(mesher.level(TileId::new(3, 7, 3).unwrap()), 0);
    }
}
