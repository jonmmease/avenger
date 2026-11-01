//! The projection of a plot: the map between its pixels and the positions along the ranges of its
//! two positional dimensions, which tools such as brushes and navigations work in.

use geo::algorithm::bool_ops::FillRule;
use geo::BooleanOps;
use geo_types::{LineString, Polygon};

/// How many steps the default [`PlotProjection::path`] takes between two positions.
const PATH_STEPS: usize = 64;

/// The longest side of a path in pixels, before the default [`PlotProjection::enclosed`] converts
/// it to range positions through a projection that is not linear, so that the polygon follows the
/// path.
pub const RING_STEP: f32 = 8.0;

/// The projection of a plot at one size: the map between its pixels, from its top-left corner, and
/// the positions along the ranges of its two positional dimensions, to which the scales of the
/// dimensions map their data.
pub trait PlotProjection: std::fmt::Debug + Send + Sync {
    /// The range position at `pixel`, or `None` where the plot shows no position, such as outside
    /// a globe.
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]>;

    /// The pixel of the range position `position`, or `None` where the plot does not show it,
    /// such as behind a globe. A dimension that wraps takes any position, so a range across the
    /// wrap maps without a break.
    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]>;

    /// The range positions that the plot shows along each dimension, from low to high.
    fn extent(&self) -> [[f64; 2]; 2];

    /// The period of `dimension`, 0 or 1, in range units, when the dimension wraps, such as 360
    /// degrees of longitude.
    fn period(&self, _dimension: usize) -> Option<f64> {
        None
    }

    /// Whether straight lines in range positions draw as straight lines in pixels, so that paths
    /// need no resampling, as in a Cartesian plot.
    fn is_linear(&self) -> bool {
        false
    }

    /// The pieces in pixels of the path from `from` to `to`, along which the range positions
    /// change steadily, cut where the plot does not show it. The default resamples the path and
    /// maps each sample.
    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        let samples = (0..=PATH_STEPS).map(|step| {
            let along = step as f64 / PATH_STEPS as f64;
            [0, 1].map(|axis| from[axis] + (to[axis] - from[axis]) * along)
        });
        pieces(samples.map(|position| self.pixel(position)))
    }

    /// The rings in pixels of the polygon whose vertices are the range positions `ring`, cut
    /// where the plot does not show it. Each side changes its range positions steadily, the short
    /// way around a dimension that wraps, so a ring across the wrap, such as a lasso's, needs no
    /// unwrapping, and a side that runs half a period or more needs vertices between its ends.
    /// The default joins the paths of the ring's sides, and breaks the ring where a path breaks.
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
