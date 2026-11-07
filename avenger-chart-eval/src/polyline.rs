//! The lines of coordinate systems whose segments bend, such as polar plots and maps: polylines
//! through the pixels of the pieces of each segment, and the line, trail, and rule marks along
//! them.

use avenger_chart::proto::Channel;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{line::SceneLineMark, trail::SceneTrailMark};
use avenger_style::Property;

use crate::{
    encode::{at, LayerRows, Result},
    marks::Drawn,
};

/// The vertices of a polyline through the pieces of segments, with an undefined vertex between
/// pieces that do not meet, so the line breaks there.
#[derive(Default)]
pub(crate) struct Polyline {
    pub(crate) x: Vec<f32>,
    pub(crate) y: Vec<f32>,
    pub(crate) defined: Vec<bool>,
    /// The row of each vertex: the first row of its segment.
    pub(crate) rows: Vec<u32>,
    /// The first and the last row of the segment of each vertex.
    ends: Vec<[u32; 2]>,
    /// How far along its piece each vertex is, from 0 at its start to 1 at its end.
    along: Vec<f32>,
}

impl Polyline {
    fn push_vertex(&mut self, [x, y]: [f32; 2], defined: bool, ends: [u32; 2], along: f32) {
        self.x.push(x);
        self.y.push(y);
        self.defined.push(defined);
        self.rows.push(ends[0]);
        self.ends.push(ends);
        self.along.push(along);
    }

    /// Adds `piece`, the pixels of a segment between the rows `ends`, continuing the line where
    /// the piece starts at its last vertex, and breaking it elsewhere.
    pub(crate) fn push_piece(&mut self, piece: &[[f32; 2]], ends: [u32; 2]) {
        let Some(first) = piece.first() else {
            return;
        };
        let meets = match (self.x.last(), self.y.last(), self.defined.last()) {
            (Some(x), Some(y), Some(true)) => {
                (x - first[0]).abs() < 1e-3 && (y - first[1]).abs() < 1e-3
            }
            _ => false,
        };
        if !self.x.is_empty() && !meets {
            self.push_vertex([f32::NAN, f32::NAN], false, ends, 0.0);
        }
        let mut lengths = vec![0.0_f32];
        for pair in piece.windows(2) {
            let length = lengths.last().copied().unwrap_or(0.0)
                + (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
            lengths.push(length);
        }
        let total = lengths.last().copied().unwrap_or(0.0);
        for (index, (point, length)) in piece.iter().zip(&lengths).enumerate() {
            if index == 0 && meets {
                continue;
            }
            let along = if total > 0.0 { length / total } else { 0.0 };
            self.push_vertex(*point, true, ends, along);
        }
    }
}

/// The polyline through the rows of `series` in its order: `place` gives the place of a row,
/// `point` the pixel of a place that the coordinate system shows, and `segment` the pieces of the
/// segment between two places, each a list of pixels. A series of one row is its point.
pub(crate) fn polyline<P: Copy>(
    series: &[u32],
    place: impl Fn(usize) -> P,
    point: impl Fn(P) -> Option<[f32; 2]>,
    segment: impl Fn(P, P) -> Vec<Vec<[f32; 2]>>,
) -> Polyline {
    let mut polyline = Polyline::default();
    if let [row] = series[..] {
        if let Some(point) = point(place(row as usize)) {
            polyline.push_piece(&[point], [row; 2]);
        }
    }
    for pair in series.windows(2) {
        for piece in segment(place(pair[0] as usize), place(pair[1] as usize)) {
            polyline.push_piece(&piece, [pair[0], pair[1]]);
        }
    }
    polyline
}

/// One line mark for each series of a line through the vertices of its polyline.
pub(crate) fn series_lines(
    rows: &LayerRows,
    polylines: Vec<(Vec<u32>, Polyline)>,
) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint()?;
    let width = rows.numbers_or(Channel::StrokeWidth, Property::StrokeWidth)?;
    let dash = rows.dashes_or_style()?;
    (polylines.into_iter().enumerate())
        .map(|(index, (series, polyline))| {
            let first = series[0] as usize;
            let mark = SceneLineMark {
                name: format!("{}/{index}", rows.name),
                len: polyline.x.len() as u32,
                x: ScalarOrArray::new_array(polyline.x),
                y: ScalarOrArray::new_array(polyline.y),
                defined: ScalarOrArray::new_array(polyline.defined),
                stroke: at(&stroke, first),
                stroke_width: at(&width, first),
                stroke_dash: (dash.as_ref())
                    .map(|dash| at(dash, first))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&polyline.rows)?,
            })
        })
        .collect()
}

/// One trail mark for each series of a trail through the vertices of its polyline, whose width
/// at each vertex runs from the size of its segment's first row to the size of its last.
pub(crate) fn series_trails(
    rows: &LayerRows,
    polylines: Vec<(Vec<u32>, Polyline)>,
) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint()?;
    let size = rows.numbers_or(Channel::Size, Property::Size)?;
    (polylines.into_iter().enumerate())
        .map(|(index, (series, polyline))| {
            let first = series[0] as usize;
            let sizes = (polyline.ends.iter().zip(&polyline.along))
                .map(|([a, b], along)| {
                    let (a, b) = (at(&size, *a as usize), at(&size, *b as usize));
                    a + (b - a) * along
                })
                .collect();
            let mark = SceneTrailMark {
                name: format!("{}/{index}", rows.name),
                len: polyline.x.len() as u32,
                x: ScalarOrArray::new_array(polyline.x),
                y: ScalarOrArray::new_array(polyline.y),
                size: ScalarOrArray::new_array(sizes),
                defined: ScalarOrArray::new_array(polyline.defined),
                stroke: at(&stroke, first),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&polyline.rows)?,
            })
        })
        .collect()
}

/// One line mark for each rule through the vertices of its polyline, by its row, without the
/// rules whose polylines have no vertex.
pub(crate) fn rule_lines(
    rows: &LayerRows,
    polylines: Vec<(usize, Polyline)>,
) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint()?;
    let width = rows.numbers_or(Channel::StrokeWidth, Property::StrokeWidth)?;
    let dash = rows.dashes_or_style()?;
    (polylines.into_iter())
        .filter(|(_, polyline)| !polyline.x.is_empty())
        .map(|(row, polyline)| {
            let mark = SceneLineMark {
                name: format!("{}/{row}", rows.name),
                len: polyline.x.len() as u32,
                x: ScalarOrArray::new_array(polyline.x),
                y: ScalarOrArray::new_array(polyline.y),
                defined: ScalarOrArray::new_array(polyline.defined),
                stroke: at(&stroke, row),
                stroke_width: at(&width, row),
                stroke_dash: (dash.as_ref())
                    .map(|dash| at(dash, row))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&polyline.rows)?,
            })
        })
        .collect()
}
