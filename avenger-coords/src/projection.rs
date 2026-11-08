//! The projection of a plot: the map between its pixels and its positions, which have two
//! dimensions and which tools such as brushes and navigations work in.

use arrow::array::{Float32Array, Float32Builder};
use geo::algorithm::bool_ops::FillRule;
use geo::BooleanOps;
use geo_types::{LineString, Polygon};

use crate::positions::Positions;

/// How many equal steps the default [`PlotProjection::path`] starts from, so that it does not miss
/// a bend between an end of the path and its middle.
const PATH_STEPS: usize = 4;

/// The largest distance in pixels between a step of the default [`PlotProjection::path`] and the
/// path that the step follows.
const PATH_TOLERANCE: f32 = 0.25;

/// How many times the default [`PlotProjection::path`] halves a step at most.
const PATH_DEPTH: u32 = 8;

/// The longest side of a path in pixels, before the default [`PlotProjection::enclosed`] converts
/// it to range positions through a projection that is not linear, so that the polygon follows the
/// path.
pub const RING_STEP: f32 = 8.0;

/// The projection of a plot at one size: the map between its pixels, from its top-left corner, and
/// its positions, which have two dimensions. In a Cartesian, polar, or geographic plot, each
/// dimension is one positional dimension of the plot, and a position is the range position to
/// which that dimension's scale maps its data. A plot whose positions combine several columns, such as a
/// ternary plot, whose dimensions are two of its three proportions, has positions that no single
/// scale gives, and it shows a region of them, such as a triangle, rather than its whole extent.
pub trait PlotProjection: std::fmt::Debug + Send + Sync {
    /// The range position at `pixel`, or `None` where the plot shows no position, such as outside
    /// a globe. A projection can give a position outside its [`region`](Self::region), such as
    /// past the edge of a ternary plot, which [`clamp`](Self::clamp) moves into the region.
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]>;

    /// The pixel of the range position `position`, or `None` where the plot does not show it,
    /// such as behind a globe. A dimension that wraps takes any position, so a range across the
    /// wrap maps without a break.
    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]>;

    /// The pixels of the rows at `positions`, as x and y, null where a position is null or the
    /// plot does not show it, such as behind a globe. The default calls [`pixel`](Self::pixel)
    /// for each row.
    ///
    /// # Panics
    ///
    /// When the two dimensions have different lengths.
    fn pixels(&self, positions: [Positions<'_>; 2]) -> [Float32Array; 2] {
        let [x, y] = positions;
        assert_eq!(
            x.len(),
            y.len(),
            "the dimensions of positions have one length"
        );
        let mut xs = Float32Builder::with_capacity(x.len());
        let mut ys = Float32Builder::with_capacity(x.len());
        for row in 0..x.len() {
            let pixel = (x.value(row).zip(y.value(row))).and_then(|(x, y)| self.pixel([x, y]));
            match pixel {
                Some([x, y]) => {
                    xs.append_value(x);
                    ys.append_value(y);
                }
                None => {
                    xs.append_null();
                    ys.append_null();
                }
            }
        }
        [xs.finish(), ys.finish()]
    }

    /// The bounds of the range positions that the plot shows along each dimension, from low to
    /// high. The plot can show a smaller region of them, which [`region`](Self::region) gives.
    fn extent(&self) -> [[f64; 2]; 2];

    /// The polygon of range positions that the plot shows, such as a ternary plot's triangle,
    /// which tools keep what a gesture selects or shows within. The default is the box of the
    /// [`extent`](Self::extent). A projection that changes the region also changes
    /// [`clamp`](Self::clamp) to match it.
    fn region(&self) -> Vec<[f64; 2]> {
        let [[x0, x1], [y0, y1]] = self.extent();
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    /// The position in the [`region`](Self::region) nearest to `position`, where a gesture that
    /// leaves the plot stays. The default limits each dimension to its extent, and leaves a
    /// dimension that wraps as it is, since it takes any position.
    fn clamp(&self, position: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| clamp_along(self, axis, position[axis]))
    }

    /// Whether the plot's [`region`](Self::region) holds `position`. The default tests each
    /// dimension that does not wrap against its extent, with a slack of a millionth of its length,
    /// so a position on the extent's edge holds after rounding.
    fn shows(&self, position: [f64; 2]) -> bool {
        (0..2).all(|axis| shows_along(self, axis, position[axis]))
    }

    /// The plot's [`region`](Self::region) in pixels, as the rings that a background fills and a
    /// clip cuts to. A ring of a hole winds the other way from the ring around it, so the hole
    /// stays empty under either fill rule. The default is the [`polygon`](Self::polygon) of the
    /// region.
    fn outline(&self) -> Vec<Vec<[f32; 2]>> {
        self.polygon(&self.region())
    }

    /// The pixel box of the [`outline`](Self::outline), from its least x and y to its greatest,
    /// or `None` for a plot that draws no outline.
    fn bounds(&self) -> Option<[[f32; 2]; 2]> {
        let mut points = self.outline().into_iter().flatten().peekable();
        points.peek()?;
        let mut bounds = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
        for [x, y] in points {
            bounds = [
                [bounds[0][0].min(x), bounds[0][1].min(y)],
                [bounds[1][0].max(x), bounds[1][1].max(y)],
            ];
        }
        Some(bounds)
    }

    /// The unit directions in pixels in which dimension 0 and dimension 1 increase at
    /// `position`, or `None` where the plot does not show it. They are directions of positions,
    /// not data: a caller multiplies a dimension's direction by the direction of its scale to
    /// learn where its data increase. The default takes a finite difference of
    /// [`pixel`](Self::pixel), a ten-thousandth of each dimension's length long, toward the
    /// inside of an extent at its high end.
    fn tangents(&self, position: [f64; 2]) -> Option<[[f32; 2]; 2]> {
        let extent = self.extent();
        let at = self.pixel(position)?;
        let mut tangents = [[0.0; 2]; 2];
        for (axis, tangent) in tangents.iter_mut().enumerate() {
            let [low, high] = extent[axis];
            let step = (high - low).abs() * 1e-4;
            let inward = self.period(axis).is_none() && position[axis] + step > high;
            let sign = if inward { -1.0 } else { 1.0 };
            let mut next = position;
            next[axis] += sign * step;
            let to = self.pixel(next)?;
            let sign = sign as f32;
            let [dx, dy] = [(to[0] - at[0]) * sign, (to[1] - at[1]) * sign];
            let length = dx.hypot(dy);
            if length.is_nan() || length <= 0.0 {
                return None;
            }
            *tangent = [dx / length, dy / length];
        }
        Some(tangents)
    }

    /// The period of `dimension`, 0 or 1, in positions, when the dimension wraps, such as 360
    /// degrees of longitude.
    fn period(&self, _dimension: usize) -> Option<f64> {
        None
    }

    /// Whether straight lines in range positions draw as straight lines in pixels, as in a
    /// Cartesian plot, so that the default [`path`](Self::path) draws them without resampling.
    fn is_linear(&self) -> bool {
        false
    }

    /// The pieces in pixels of the path from `from` to `to`, along which the range positions
    /// change steadily, cut where the plot does not show it. The default draws a straight line
    /// when the projection [is linear](Self::is_linear). Otherwise it maps positions along the
    /// path at steps that it halves until each step lies within a quarter of a pixel of the path,
    /// so a sharp bend takes more steps than a gentle one, and it halves a step across a cut
    /// until the cut lies within a thousandth of the path.
    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        if self.is_linear() {
            return pieces([self.pixel(from), self.pixel(to)]);
        }
        let mut pixels = vec![self.pixel(from)];
        let mut start = (0.0, pixels[0]);
        for step in 1..=PATH_STEPS {
            let along = step as f64 / PATH_STEPS as f64;
            let end = (along, self.pixel(between(from, to, along)));
            halve(self, [from, to], start, end, PATH_DEPTH, &mut pixels);
            start = end;
        }
        pieces(pixels)
    }

    /// The pieces in pixels of the line that the plot draws between the positions `from` and
    /// `to`: the shortest line in the plot's geometry, such as the great circle between two
    /// places on a globe, cut where the plot does not show it. The default is the
    /// [`path`](Self::path) the short way around a dimension that wraps.
    fn route(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        self.path(from, short_way(self, from, to))
    }

    /// The pieces in pixels of the line along which `dimension`, 0 or 1, holds `value`, across
    /// `span` of the other dimension or its whole span, cut where the plot does not show it: a
    /// grid line, a ring or a spoke of a polar plot, or a meridian or a parallel of a geo plot. The
    /// default is the [`path`](Self::path) from one end of the span to the other, which goes all
    /// the way around a dimension that wraps.
    fn iso_line(&self, dimension: usize, value: f64, span: Option<[f64; 2]>) -> Vec<Vec<[f32; 2]>> {
        let other = 1 - dimension;
        let [low, high] = span.unwrap_or_else(|| whole(self, other));
        let mut from = [0.0; 2];
        from[dimension] = value;
        from[other] = low;
        let mut to = from;
        to[other] = high;
        self.path(from, to)
    }

    /// The rings in pixels of the polygon whose vertices are the range positions `ring`, cut
    /// where the plot does not show it. Each side changes its range positions steadily, the short
    /// way around a dimension that wraps, so a ring across the wrap, such as a lasso's, needs no
    /// unwrapping, and a side that runs half a period or more needs vertices between its ends.
    /// The default joins the paths of the ring's sides, breaks the ring where a path breaks, and
    /// leaves out a ring's last vertex where it repeats the first.
    fn polygon(&self, ring: &[[f64; 2]]) -> Vec<Vec<[f32; 2]>> {
        let mut rings: Vec<Vec<[f32; 2]>> = Vec::new();
        let mut current: Vec<[f32; 2]> = Vec::new();
        for (index, from) in ring.iter().enumerate() {
            let to = short_way(self, *from, ring[(index + 1) % ring.len()]);
            for (piece_index, piece) in self.path(*from, to).into_iter().enumerate() {
                if piece_index > 0 && !current.is_empty() {
                    rings.push(std::mem::take(&mut current));
                }
                let joins = current.last() == piece.first();
                current.extend(piece.into_iter().skip(usize::from(joins)));
            }
        }
        if !current.is_empty() {
            rings.push(current);
        }
        for ring in &mut rings {
            if ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }
        }
        rings.retain(|ring| ring.len() > 2);
        rings
    }

    /// The polygons in range positions that the closed path `path`, in pixels, encloses, cut to
    /// where the plot shows positions, such as the parts of a lasso on a globe. Along a dimension
    /// that wraps, each polygon lies within one period from the low end of the dimension's
    /// extent, so a planar test of a position in that period tells whether the path encloses it:
    /// a polygon across the wrap splits there, and a path around an end of the other dimension,
    /// such as the center of a polar plot, closes through that end. The default converts each
    /// vertex, after it resamples the path when the projection is not linear, gives no polygon
    /// when a vertex has no position, and lays the polygon within one period with
    /// [`within_period`].
    fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        let ring: Option<Vec<[f64; 2]>> = if !self.is_linear() {
            densified(path).map(|pixel| self.position(pixel)).collect()
        } else {
            path.iter().map(|pixel| self.position(*pixel)).collect()
        };
        let rings = ring.filter(|ring| ring.len() > 2).into_iter().collect();
        within_period(self, path, rings)
    }
}

/// The position `along` the way from `from` to `to`, from 0 at `from` to 1 at `to`.
fn between(from: [f64; 2], to: [f64; 2], along: f64) -> [f64; 2] {
    [0, 1].map(|axis| from[axis] + (to[axis] - from[axis]) * along)
}

/// Push to `pixels` the pixels of the step of the path `[from, to]` of `projection` from `start`
/// to `end`, each the distance along the path and its pixel, after the pixel of `start`. The step
/// is halved, at most `depth` times, while the pixel of its middle lies farther than
/// [`PATH_TOLERANCE`] from the segment between the pixels of its ends, or while one of the three
/// has a pixel and another has none, so that the path is cut close to where the plot stops
/// showing it.
fn halve<P: PlotProjection + ?Sized>(
    projection: &P,
    [from, to]: [[f64; 2]; 2],
    start: (f64, Option<[f32; 2]>),
    end: (f64, Option<[f32; 2]>),
    depth: u32,
    pixels: &mut Vec<Option<[f32; 2]>>,
) {
    if depth > 0 {
        let along = (start.0 + end.0) / 2.0;
        let middle = (along, projection.pixel(between(from, to, along)));
        let straight = match (start.1, middle.1, end.1) {
            (Some(a), Some(m), Some(b)) => distance(m, [a, b]) <= PATH_TOLERANCE,
            (None, None, None) => true,
            _ => false,
        };
        if !straight {
            halve(projection, [from, to], start, middle, depth - 1, pixels);
            halve(projection, [from, to], middle, end, depth - 1, pixels);
            return;
        }
    }
    pixels.push(end.1);
}

/// The distance in pixels from `point` to the segment from `a` to `b`.
fn distance(point: [f32; 2], [a, b]: [[f32; 2]; 2]) -> f32 {
    let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
    let [px, py] = [point[0] - a[0], point[1] - a[1]];
    let length = dx * dx + dy * dy;
    let along = if length > 0.0 {
        ((px * dx + py * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (px - along * dx).hypot(py - along * dy)
}

/// The whole span of `dimension` of `projection` in positions: its extent, or one period from the
/// extent's low end for a dimension that wraps.
pub(crate) fn whole<P: PlotProjection + ?Sized>(projection: &P, dimension: usize) -> [f64; 2] {
    let [low, high] = projection.extent()[dimension];
    [
        low,
        projection
            .period(dimension)
            .map_or(high, |period| low + period),
    ]
}

/// `position` along `dimension` of `projection` moved inside the dimension's extent, or as it is
/// along a dimension that wraps, which takes any position.
pub(crate) fn clamp_along<P: PlotProjection + ?Sized>(
    projection: &P,
    dimension: usize,
    position: f64,
) -> f64 {
    let [low, high] = projection.extent()[dimension];
    match projection.period(dimension) {
        Some(_) => position,
        None => position.clamp(low, high),
    }
}

/// Whether `position` along `dimension` of `projection` lies in the dimension's extent, with a
/// slack of a millionth of its length, so a position on the extent's edge holds after rounding.
/// A dimension that wraps holds every position.
pub(crate) fn shows_along<P: PlotProjection + ?Sized>(
    projection: &P,
    dimension: usize,
    position: f64,
) -> bool {
    let [low, high] = projection.extent()[dimension];
    let slack = (high - low).abs() * 1e-6;
    projection.period(dimension).is_some() || (position >= low - slack && position <= high + slack)
}

/// `to`, moved by whole periods along each dimension of `projection` that wraps to within half a
/// period of `from`, so that a side from `from` to it runs the short way around.
pub fn short_way<P: PlotProjection + ?Sized>(
    projection: &P,
    from: [f64; 2],
    to: [f64; 2],
) -> [f64; 2] {
    std::array::from_fn(|axis| match projection.period(axis) {
        Some(period) => {
            from[axis] + (to[axis] - from[axis] + period / 2.0).rem_euclid(period) - period / 2.0
        }
        None => to[axis],
    })
}

/// `rings`, polygons in range positions inside the closed path `path` in pixels, laid within one
/// period from the low end of the extent of a dimension of `projection` that wraps. A ring that runs
/// across the wrap splits into a polygon on each side of it, and a ring that winds around the
/// other dimension, as a path around the center of a polar plot does, closes through the end of
/// the other dimension that the path encloses. Without a dimension that wraps, `rings` stay as
/// they are.
pub fn within_period<P: PlotProjection + ?Sized>(
    projection: &P,
    path: &[[f32; 2]],
    rings: Vec<Vec<[f64; 2]>>,
) -> Vec<Vec<[f64; 2]>> {
    let Some((wrap, period)) = (0..2).find_map(|axis| Some((axis, projection.period(axis)?)))
    else {
        return rings;
    };
    let other = 1 - wrap;
    let extent = projection.extent();
    let at = |along: f64, across: f64| {
        let mut position = [0.0; 2];
        position[wrap] = along;
        position[other] = across;
        position
    };
    let mut polygons = Vec::new();
    for ring in rings {
        let Some(&first) = ring.first() else {
            continue;
        };
        // Each vertex moves to the short way around from the one before it.
        let mut unwrapped = vec![first];
        for position in &ring[1..] {
            let last = unwrapped[unwrapped.len() - 1];
            unwrapped.push(short_way(projection, last, *position));
        }
        let closing = short_way(projection, unwrapped[unwrapped.len() - 1], first);
        let turns = ((closing[wrap] - first[wrap]) / period).round();
        if turns != 0.0 {
            // The ring winds around an end of the other dimension: it encloses the end that the
            // path holds, which the ring reaches along its closing position and leaves along its
            // first, with a vertex every quarter period between them.
            let encloses = |end: f64| {
                (projection.pixel(at(first[wrap], end)))
                    .is_some_and(|pixel| winding(path, pixel) != 0)
            };
            let [low, high] = extent[other];
            let end = if !encloses(low) && encloses(high) {
                high
            } else {
                low
            };
            unwrapped.push(closing);
            let steps = (turns.abs() * 4.0) as usize;
            unwrapped.extend((0..=steps).map(|step| {
                let along =
                    closing[wrap] + (first[wrap] - closing[wrap]) * step as f64 / steps as f64;
                at(along, end)
            }));
        }
        polygons.extend(split(&unwrapped, wrap, extent[wrap][0], period));
    }
    polygons
}

/// The pieces of the polygon `ring`, whose positions along `wrap` change steadily, each within
/// the period from `start` that it lies in, moved by whole periods into the first period. Each
/// piece reaches a millionth of a period past the ends of its period, so a planar test holds a
/// position on the wrap, such as the start of the period, in the pieces on both sides of it.
fn split(ring: &[[f64; 2]], wrap: usize, start: f64, period: f64) -> Vec<Vec<[f64; 2]>> {
    let along = ring.iter().map(|position| position[wrap]);
    let [low, high] = along.fold([f64::INFINITY, f64::NEG_INFINITY], |[low, high], value| {
        [low.min(value), high.max(value)]
    });
    let [first, last] = [
        ((low - start) / period).floor(),
        ((high - start) / period).ceil() - 1.0,
    ];
    let moved = |ring: &[[f64; 2]], by: f64| -> Vec<[f64; 2]> {
        (ring.iter())
            .map(|position| {
                let mut position = *position;
                position[wrap] += by;
                position
            })
            .collect()
    };
    if first >= last {
        return vec![moved(ring, -first * period)];
    }
    let other = 1 - wrap;
    let across = ring.iter().map(|position| position[other]);
    let [bottom, top] = across.fold([f64::INFINITY, f64::NEG_INFINITY], |[low, high], value| {
        [low.min(value), high.max(value)]
    });
    let point = |along: f64, across: f64| {
        let mut position = [0.0; 2];
        position[wrap] = along;
        position[other] = across;
        (position[0], position[1])
    };
    let polygon = Polygon::new(ring.iter().map(|p| (p[0], p[1])).collect(), Vec::new());
    let mut pieces = Vec::new();
    for index in (first as i64)..=(last as i64) {
        let overlap = period * 1e-6;
        let [from, to] = [index, index + 1].map(|index| start + index as f64 * period);
        let [from, to] = [from - overlap, to + overlap];
        let strip: LineString<f64> = [
            point(from, bottom - 1.0),
            point(to, bottom - 1.0),
            point(to, top + 1.0),
            point(from, top + 1.0),
        ]
        .into_iter()
        .collect();
        let strip = Polygon::new(strip, Vec::new());
        let inside = polygon.intersection_with_fill_rule(&strip, FillRule::NonZero);
        for piece in inside.iter() {
            let mut exterior: Vec<[f64; 2]> =
                piece.exterior().coords().map(|c| [c.x, c.y]).collect();
            exterior.pop();
            if exterior.len() > 2 {
                pieces.push(moved(&exterior, -(index as f64) * period));
            }
        }
    }
    pieces
}

/// How many times the closed path `path` winds around `at`, counterclockwise on the screen less
/// clockwise.
fn winding(path: &[[f32; 2]], at: [f32; 2]) -> i32 {
    let mut winding = 0;
    for (index, from) in path.iter().enumerate() {
        let to = path[(index + 1) % path.len()];
        let side = (to[0] - from[0]) * (at[1] - from[1]) - (at[0] - from[0]) * (to[1] - from[1]);
        if from[1] <= at[1] {
            if to[1] > at[1] && side > 0.0 {
                winding += 1;
            }
        } else if to[1] <= at[1] && side < 0.0 {
            winding -= 1;
        }
    }
    winding
}

/// The vertices of the closed path `path`, without its closing vertex, with more on each side so
/// that none lies farther than [`RING_STEP`] from the next.
pub fn densified(path: &[[f32; 2]]) -> impl Iterator<Item = [f32; 2]> + '_ {
    (0..path.len()).flat_map(move |index| {
        let (from, to) = (path[index], path[(index + 1) % path.len()]);
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / RING_STEP).ceil();
        let steps = steps.max(1.0) as usize;
        (0..steps).map(move |step| {
            let along = step as f32 / steps as f32;
            [0, 1].map(|axis| from[axis] + along * (to[axis] - from[axis]))
        })
    })
}

/// The runs of consecutive pixels of `pixels`, broken where a pixel is `None`. A run of one pixel
/// draws nothing, so it is dropped.
pub fn pieces(pixels: impl IntoIterator<Item = Option<[f32; 2]>>) -> Vec<Vec<[f32; 2]>> {
    let mut pieces = Vec::new();
    let mut piece = Vec::new();
    for pixel in pixels {
        match pixel {
            Some(pixel) => piece.push(pixel),
            None if piece.is_empty() => {}
            None => pieces.push(std::mem::take(&mut piece)),
        }
    }
    pieces.push(piece);
    pieces.retain(|piece| piece.len() > 1);
    pieces
}
