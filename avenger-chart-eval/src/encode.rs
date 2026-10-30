//! The scene marks of a unit: one function for each mark, from the columns of the unit's table
//! and its configured scales.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, RecordBatch, UInt32Array},
    compute::{cast, take_record_batch},
    datatypes::DataType,
    row::{RowConverter, SortField},
};
use avenger_color::{apply_opacity_to_color, ColorOrGradient};
use avenger_common::{
    pattern::{PatternFill, PatternFills},
    types::{
        AreaOrientation, FontWeight, ImageAlign, ImageBaseline, PathTransform, SymbolShape,
        TextAlign, TextBaseline, TextSyntaxMode,
    },
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_format::PrecisionSharing;
use avenger_scales::scales::{
    band, coerce::Coercer, nested_band::NestedBandLayout, ConfiguredScale,
};
use avenger_scenegraph::marks::{
    area::SceneAreaMark,
    image::{SceneImageMark, SceneImageResource, SceneImageSource},
    line::SceneLineMark,
    mark::SceneMark,
    path::ScenePathMark,
    rect::SceneRectMark,
    rule::SceneRuleMark,
    symbol::SceneSymbolMark,
    text::SceneTextMark,
    trail::SceneTrailMark,
};
use avenger_typst_label::{escape_text, LabelEngine, LabelValue};
use lyon_path::geom::{euclid::Vector2D, Angle};

use avenger_chart::{
    proto::{self, condition, encoding, mark, Channel},
    resolve::scale_channel,
    Chart, FitKind,
};

use crate::{scale::eval_error, EvalError, Theme};

type Result<T> = std::result::Result<T, EvalError>;

/// A scene mark, with the rows of its unit's table that draw its instances, in their order.
pub(crate) struct Drawn {
    pub(crate) mark: SceneMark,
    pub(crate) rows: RecordBatch,
}

/// The rows of `table` that draw an instance: those whose encoded position columns are not null.
pub(crate) fn drawn_rows(unit: &proto::Unit, table: &RecordBatch) -> Result<RecordBatch> {
    let positions = [
        Channel::X,
        Channel::Y,
        Channel::X2,
        Channel::Y2,
        Channel::Theta,
        Channel::Radius,
        Channel::Theta2,
        Channel::Radius2,
        Channel::Longitude,
        Channel::Latitude,
        Channel::Longitude2,
        Channel::Latitude2,
    ];
    let columns: Vec<&ArrayRef> = unit
        .encodings
        .iter()
        .filter(|encoding| {
            positions
                .iter()
                .any(|channel| *channel as i32 == encoding.channel)
        })
        .filter_map(|encoding| match &encoding.value {
            Some(encoding::Value::Column(name)) => table.column_by_name(name),
            _ => None,
        })
        .collect();
    if columns.iter().all(|column| column.null_count() == 0) {
        return Ok(table.clone());
    }
    let rows: UInt32Array = (0..table.num_rows())
        .filter(|row| columns.iter().all(|column| column.is_valid(*row)))
        .map(|row| row as u32)
        .collect();
    Ok(take_record_batch(table, &rows)?)
}

/// Where a value of an encoding or a condition comes from.
#[derive(Clone, Copy)]
enum Source<'a> {
    Column(&'a str),
    Constant(u32),
    Scaled(u32),
}

impl<'a> Source<'a> {
    fn of_encoding(value: &'a encoding::Value) -> Self {
        match value {
            encoding::Value::Column(name) => Self::Column(name),
            encoding::Value::Constant(index) => Self::Constant(*index),
            encoding::Value::ScaledConstant(index) => Self::Scaled(*index),
        }
    }

    fn of_condition(value: &'a condition::Value) -> Self {
        match value {
            condition::Value::Column(name) => Self::Column(name),
            condition::Value::Constant(index) => Self::Constant(*index),
            condition::Value::ScaledConstant(index) => Self::Scaled(*index),
        }
    }
}

/// The positions of the drawn rows of a unit in a map plot or a polar plot, in pixels from the
/// plot's top left: where its `longitude` and `latitude` project to, or the points of its `theta`
/// and `radius`.
pub(crate) struct Projected {
    pub(crate) x: Vec<f32>,
    pub(crate) y: Vec<f32>,
}

/// The vertices of a polyline through the projected pieces of segments, with an undefined vertex
/// between pieces that do not meet, so the line breaks there.
#[derive(Default)]
pub(crate) struct Polyline {
    pub(crate) x: Vec<f32>,
    pub(crate) y: Vec<f32>,
    pub(crate) defined: Vec<bool>,
    /// The row of each vertex: the first row of its segment.
    pub(crate) rows: Vec<u32>,
    /// The first and the last row of the segment of each vertex.
    pub(crate) ends: Vec<[u32; 2]>,
    /// How far along its piece each vertex is, from 0 at its start to 1 at its end.
    pub(crate) along: Vec<f32>,
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

    /// Adds `piece`, the projection of a segment between the rows `ends`, continuing the line
    /// where the piece starts at its last vertex, and breaking it elsewhere.
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

/// The polyline through the rows of `series` in its order, for a projection whose segments bend:
/// `place` gives the place of a row, `point` the pixel position of a place that the projection
/// shows, and `segment` the pieces of the segment between two places, each a list of pixel
/// positions. A series of one row is its point.
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
    rows: &UnitRows,
    polylines: Vec<(Vec<u32>, Polyline)>,
) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint(false)?;
    let width = rows
        .numbers(Channel::StrokeWidth)?
        .unwrap_or_else(|| scalar(rows.theme.stroke_width));
    let dash = rows.dashes(Channel::StrokeDash)?;
    polylines
        .into_iter()
        .enumerate()
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
                stroke_dash: dash
                    .as_ref()
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

/// One trail mark for each series of a trail through the vertices of its polyline.
pub(crate) fn series_trails(
    rows: &UnitRows,
    polylines: Vec<(Vec<u32>, Polyline)>,
) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint(false)?;
    let size = rows
        .numbers(Channel::Size)?
        .unwrap_or_else(|| scalar(rows.theme.trail_size));
    polylines
        .into_iter()
        .enumerate()
        .map(|(index, (series, polyline))| {
            let first = series[0] as usize;
            // The width of each vertex runs from the size of its segment's first row to the size
            // of its last.
            let sizes = polyline
                .ends
                .iter()
                .zip(&polyline.along)
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
pub(crate) fn rule_lines(rows: &UnitRows, polylines: Vec<(usize, Polyline)>) -> Result<Vec<Drawn>> {
    let (_, stroke) = rows.paint(false)?;
    let width = rows
        .numbers(Channel::StrokeWidth)?
        .unwrap_or_else(|| scalar(1.0));
    let dash = rows.dashes(Channel::StrokeDash)?;
    polylines
        .into_iter()
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
                stroke_dash: dash
                    .as_ref()
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

/// A converter of the values of a channel, through its scale when it has one.
type Convert<'c, T> = &'c dyn Fn(&ArrayRef, Option<&ConfiguredScale>) -> Result<ScalarOrArray<T>>;

/// The drawn rows of a unit, with what encodes them at a size.
pub(crate) struct UnitRows<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) unit: &'a proto::Unit,
    pub(crate) mark: &'a mark::Mark,
    /// The rows that draw an instance, in order.
    pub(crate) table: RecordBatch,
    /// The positional scales at the size of the plot, by the channel of their group, from the
    /// plot's projection.
    pub(crate) positions: BTreeMap<Channel, &'a ConfiguredScale>,
    /// The positions of the rows in a map plot or of a mark at points in a polar plot, which
    /// take the place of `x` and `y`.
    pub(crate) projected: Option<Projected>,
    /// The non-position scales, by the channel of their group.
    pub(crate) scales: &'a BTreeMap<Channel, ConfiguredScale>,
    /// Whether each condition of each channel selects each row.
    pub(crate) masks: BTreeMap<Channel, Vec<Vec<bool>>>,
    pub(crate) theme: &'a Theme,
    pub(crate) engine: &'a LabelEngine,
    /// The size of the plot.
    pub(crate) size: [f32; 2],
    /// The name of the unit's marks.
    pub(crate) name: String,
}

/// The value of `values` at `row`.
pub(crate) fn at<T: Clone + Sync>(values: &ScalarOrArray<T>, row: usize) -> T {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values[row].clone(),
    }
}

/// The values of `values` at `rows`.
fn subset<T: Clone + Sync>(values: &ScalarOrArray<T>, rows: &[u32]) -> ScalarOrArray<T> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => ScalarOrArray::new_scalar(value.clone()),
        ScalarOrArrayValue::Array(values) => ScalarOrArray::new_array(
            rows.iter()
                .map(|row| values[*row as usize].clone())
                .collect(),
        ),
    }
}

pub(crate) fn scalar<T: Clone + Sync>(value: T) -> ScalarOrArray<T> {
    ScalarOrArray::new_scalar(value)
}

fn scale_error(error: impl std::fmt::Display) -> EvalError {
    eval_error("a scale of a mark", error)
}

impl UnitRows<'_> {
    pub(crate) fn len(&self) -> usize {
        self.table.num_rows()
    }

    fn encoding(&self, channel: Channel) -> Option<&proto::Encoding> {
        self.unit
            .encodings
            .iter()
            .find(|encoding| encoding.channel == channel as i32)
    }

    pub(crate) fn has(&self, channel: Channel) -> bool {
        self.encoding(channel).is_some()
    }

    /// Whether the unit places its rows along `x` or `y`: through an encoding of the channel, or
    /// in a map plot, through the projection of its `longitude` and `latitude`.
    fn places(&self, channel: Channel) -> bool {
        self.has(channel)
            || (self.projected.is_some() && matches!(channel, Channel::X | Channel::Y))
    }

    /// The scale that `channel` goes through: its own, or the scale of the first channel of its
    /// pair, when that channel has one.
    pub(crate) fn scale_for(&self, channel: Channel) -> Option<&ConfiguredScale> {
        let first = scale_channel(channel);
        self.encoding(first)?.scale.as_ref()?;
        (self.positions.get(&first).copied()).or_else(|| self.scales.get(&first))
    }

    fn array(&self, source: Source) -> Result<ArrayRef> {
        match source {
            Source::Column(name) => self.table.column_by_name(name).cloned().ok_or_else(|| {
                EvalError::Evaluation(format!("the unit's table has no column `{name}`"))
            }),
            Source::Constant(index) | Source::Scaled(index) => Ok(self.chart.value(index).clone()),
        }
    }

    fn convert<T: Clone + Sync>(
        &self,
        source: Source,
        scale: Option<&ConfiguredScale>,
        convert: Convert<T>,
    ) -> Result<ScalarOrArray<T>> {
        let values = self.array(source)?;
        let scale = match source {
            Source::Constant(_) => None,
            _ => scale,
        };
        let converted = convert(&values, scale)?;
        Ok(match source {
            Source::Column(_) => converted,
            _ => converted.to_scalar_if_len_one(),
        })
    }

    /// The value of `channel` for each row: the value of the first condition that selects the
    /// row, or the encoding's value.
    fn resolve<T: Clone + Sync>(
        &self,
        channel: Channel,
        convert: Convert<T>,
    ) -> Result<Option<ScalarOrArray<T>>> {
        let Some(encoding) = self.encoding(channel) else {
            return Ok(None);
        };
        let Some(value) = &encoding.value else {
            return Ok(None);
        };
        let scale = self.scale_for(channel);
        let base = self.convert(Source::of_encoding(value), scale, convert)?;
        if encoding.conditions.is_empty() {
            return Ok(Some(base));
        }
        let len = self.len();
        let masks = self.masks.get(&channel);
        let alternatives: Vec<Vec<T>> = encoding
            .conditions
            .iter()
            .filter_map(|condition| condition.value.as_ref())
            .map(|value| {
                Ok(self
                    .convert(Source::of_condition(value), scale, convert)?
                    .as_vec(len, None))
            })
            .collect::<Result<_>>()?;
        let base = base.as_vec(len, None);
        let merged = (0..len)
            .map(|row| {
                alternatives
                    .iter()
                    .enumerate()
                    .find(|(index, _)| masks.is_some_and(|masks| masks[*index][row]))
                    .map(|(_, values)| values[row].clone())
                    .unwrap_or_else(|| base[row].clone())
            })
            .collect();
        Ok(Some(ScalarOrArray::new_array(merged)))
    }

    /// The values that `channel` can take for each row: the encoding's value and the value of
    /// each condition.
    pub(crate) fn alternatives<T: Clone + Sync>(
        &self,
        channel: Channel,
        convert: Convert<T>,
    ) -> Result<Vec<ScalarOrArray<T>>> {
        let Some(encoding) = self.encoding(channel) else {
            return Ok(Vec::new());
        };
        let scale = self.scale_for(channel);
        let mut values = Vec::new();
        if let Some(value) = &encoding.value {
            values.push(self.convert(Source::of_encoding(value), scale, convert)?);
        }
        for condition in &encoding.conditions {
            if let Some(value) = &condition.value {
                values.push(self.convert(Source::of_condition(value), scale, convert)?);
            }
        }
        Ok(values)
    }

    pub(crate) fn numbers(&self, channel: Channel) -> Result<Option<ScalarOrArray<f32>>> {
        self.resolve(channel, &numbers)
    }

    fn colors(&self, channel: Channel) -> Result<Option<ScalarOrArray<ColorOrGradient>>> {
        self.resolve(channel, &colors)
    }

    pub(crate) fn strings(&self, channel: Channel) -> Result<Option<ScalarOrArray<String>>> {
        self.resolve(channel, &strings)
    }

    pub(crate) fn dashes(&self, channel: Channel) -> Result<Option<ScalarOrArray<Vec<f32>>>> {
        self.resolve(channel, &dashes)
    }

    pub(crate) fn patterns(
        &self,
        channel: Channel,
    ) -> Result<Option<ScalarOrArray<Option<PatternFill>>>> {
        self.resolve(channel, &patterns)
    }

    /// The pixel offset of a position within its band: half the band of a band scale for a mark
    /// that stands at the center of its band.
    fn band_center(&self, channel: Channel) -> f32 {
        self.scale_for(channel)
            .and_then(bandwidth)
            .map_or(0.0, |width| width / 2.0)
    }

    /// The pixel position of each row along `channel`, at the center of a band, or the center of
    /// the plot when the unit does not encode the channel.
    pub(crate) fn position(&self, channel: Channel) -> Result<ScalarOrArray<f32>> {
        match (&self.projected, channel) {
            (Some(projected), Channel::X) => {
                return Ok(ScalarOrArray::new_array(projected.x.clone()))
            }
            (Some(projected), Channel::Y) => {
                return Ok(ScalarOrArray::new_array(projected.y.clone()))
            }
            _ => {}
        }
        let center = match channel {
            Channel::X | Channel::X2 => self.size[0] / 2.0,
            _ => self.size[1] / 2.0,
        };
        let offset = self.band_center(channel);
        Ok(match self.numbers(channel)? {
            Some(values) if offset != 0.0 => values.map(|value| value + offset),
            Some(values) => values,
            None => scalar(center),
        })
    }

    /// The fill and the stroke of each row: `color` sets the fill of a mark with a fill and the
    /// stroke of a mark without one, `fill` and `stroke` set their own, and the opacities scale
    /// their alpha.
    pub(crate) fn paint(
        &self,
        filled: bool,
    ) -> Result<(
        ScalarOrArray<ColorOrGradient>,
        ScalarOrArray<ColorOrGradient>,
    )> {
        let color = self
            .colors(Channel::Color)?
            .unwrap_or_else(|| scalar(ColorOrGradient::Color(self.theme.mark_color)));
        let transparent = scalar(ColorOrGradient::transparent());
        let fill = match self.colors(Channel::Fill)? {
            Some(fill) => fill,
            None if filled => color.clone(),
            None => transparent.clone(),
        };
        let stroke = match self.colors(Channel::Stroke)? {
            Some(stroke) => stroke,
            None if !filled => color,
            None => transparent,
        };
        let opacity = self.numbers(Channel::Opacity)?;
        let fill = fade(fade(fill, &opacity), &self.numbers(Channel::FillOpacity)?);
        let stroke = fade(
            fade(stroke, &opacity),
            &self.numbers(Channel::StrokeOpacity)?,
        );
        Ok((fill, stroke))
    }

    /// The rows of each series of a line, a trail, or an area, in the order of their first rows:
    /// the rows with one value of each of `detail` and the color channels.
    pub(crate) fn series(&self) -> Result<Vec<Vec<u32>>> {
        let keys: Vec<ArrayRef> = [
            Channel::Color,
            Channel::Fill,
            Channel::Stroke,
            Channel::Detail,
        ]
        .into_iter()
        .filter_map(|channel| match &self.encoding(channel)?.value {
            Some(encoding::Value::Column(name)) => self.table.column_by_name(name).cloned(),
            _ => None,
        })
        .collect();
        let len = self.len() as u32;
        if keys.is_empty() {
            return Ok(if len == 0 {
                Vec::new()
            } else {
                vec![(0..len).collect()]
            });
        }
        let converter = RowConverter::new(
            keys.iter()
                .map(|key| SortField::new(key.data_type().clone()))
                .collect(),
        )?;
        let rows = converter.convert_columns(&keys)?;
        let mut order: Vec<Vec<u8>> = Vec::new();
        let mut series: BTreeMap<Vec<u8>, Vec<u32>> = BTreeMap::new();
        for row in 0..len {
            let key = rows.row(row as usize).as_ref().to_vec();
            series
                .entry(key.clone())
                .or_insert_with(|| {
                    order.push(key);
                    Vec::new()
                })
                .push(row);
        }
        Ok(order
            .into_iter()
            .map(|key| series.remove(&key).unwrap_or_default())
            .collect())
    }

    pub(crate) fn rows(&self, rows: &[u32]) -> Result<RecordBatch> {
        Ok(take_record_batch(
            &self.table,
            &UInt32Array::from(rows.to_vec()),
        )?)
    }

    /// The baseline of a bar or an area along `channel`: zero when the scale's domain holds it,
    /// and otherwise the start of the range.
    pub(crate) fn baseline(&self, channel: Channel) -> Result<f32> {
        let Some(scale) = self.scale_for(channel) else {
            return Ok(match channel {
                Channel::Y => self.size[1],
                _ => 0.0,
            });
        };
        let (start, end) = scale.numeric_interval_range().map_err(scale_error)?;
        let (low, high) = (start.min(end) as f32, start.max(end) as f32);
        let zero = matches!(scale.scale_impl.scale_type(), "linear" | "pow" | "symlog")
            .then(|| {
                scale
                    .scale_to_numeric(&(Arc::new(Float64Array::from(vec![0.0])) as ArrayRef))
                    .ok()
                    .map(|values| at(&values, 0))
            })
            .flatten();
        Ok(match zero {
            Some(zero) if zero.is_finite() => zero.clamp(low, high),
            _ => start as f32,
        })
    }

    /// Whether a bar or an area of the unit is horizontal: when its orient says so, or else when
    /// only its `x` scale is continuous.
    fn horizontal(&self, orient: i32) -> bool {
        let continuous = |channel| {
            self.scale_for(channel).is_some_and(|scale| {
                FitKind::of(scale.scale_impl.scale_type()) == FitKind::Interval
            })
        };
        match proto::Orient::try_from(orient) {
            Ok(proto::Orient::Horizontal) => true,
            Ok(proto::Orient::Vertical) => false,
            _ => continuous(Channel::X) && !continuous(Channel::Y),
        }
    }
}

/// The band of a band or a nested band scale, in pixels.
pub(crate) fn bandwidth(scale: &ConfiguredScale) -> Option<f32> {
    match scale.scale_impl.scale_type() {
        "band" => band::bandwidth(&scale.config)
            .ok()
            .map(|width| width as f32),
        "nested_band" => NestedBandLayout::from_config(&scale.config)
            .ok()
            .map(|layout| layout.leaf_bandwidth() as f32),
        _ => None,
    }
}

fn numbers(values: &ArrayRef, scale: Option<&ConfiguredScale>) -> Result<ScalarOrArray<f32>> {
    match scale {
        Some(scale) => scale.scale_to_numeric(values).map_err(scale_error),
        None => Coercer::default()
            .to_numeric(values, None)
            .map_err(scale_error),
    }
}

fn colors(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<ColorOrGradient>> {
    match scale {
        Some(scale) => scale.scale_to_color(values).map_err(scale_error),
        None => Coercer::default()
            .to_color(values, None)
            .map_err(scale_error),
    }
}

fn strings(values: &ArrayRef, scale: Option<&ConfiguredScale>) -> Result<ScalarOrArray<String>> {
    match scale {
        Some(scale) => scale.scale_to_string(values).map_err(scale_error),
        None => {
            let text = cast(values, &DataType::Utf8)?;
            Ok(ScalarOrArray::new_array(
                text.as_string::<i32>()
                    .iter()
                    .map(|value| value.unwrap_or_default().to_string())
                    .collect(),
            ))
        }
    }
}

fn dashes(values: &ArrayRef, scale: Option<&ConfiguredScale>) -> Result<ScalarOrArray<Vec<f32>>> {
    let values = match scale {
        Some(scale) => scale.scale(values).map_err(scale_error)?,
        None => values.clone(),
    };
    Coercer::default()
        .to_stroke_dash(&values)
        .map_err(scale_error)
}

fn patterns(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<Option<PatternFill>>> {
    match scale {
        Some(scale) => scale.scale_to_pattern(values).map_err(scale_error),
        None => Coercer::default().to_pattern(values).map_err(scale_error),
    }
}

/// `colors` with their alpha times `opacity`.
fn fade(
    colors: ScalarOrArray<ColorOrGradient>,
    opacity: &Option<ScalarOrArray<f32>>,
) -> ScalarOrArray<ColorOrGradient> {
    let Some(opacity) = opacity else {
        return colors;
    };
    match (colors.value(), opacity.value()) {
        (ScalarOrArrayValue::Scalar(color), ScalarOrArrayValue::Scalar(opacity)) => {
            scalar(apply_opacity_to_color(color, *opacity))
        }
        _ => {
            let len = match (colors.value(), opacity.value()) {
                (ScalarOrArrayValue::Array(values), _) => values.len(),
                (_, ScalarOrArrayValue::Array(values)) => values.len(),
                _ => 1,
            };
            ScalarOrArray::new_array(
                (0..len)
                    .map(|row| apply_opacity_to_color(&at(&colors, row), at(opacity, row)))
                    .collect(),
            )
        }
    }
}

/// The patterns of each row as a scene mark takes them.
pub(crate) fn pattern_fills(patterns: Option<ScalarOrArray<Option<PatternFill>>>) -> PatternFills {
    match patterns {
        Some(patterns) => PatternFills(patterns),
        None => PatternFills::default(),
    }
}

/// The distinct shapes of `names`, and the index of each row's shape among them.
pub(crate) fn shapes(
    names: &ScalarOrArray<String>,
) -> Result<(Vec<SymbolShape>, ScalarOrArray<usize>)> {
    let parse = |name: &str| {
        SymbolShape::from_vega_str(name)
            .map_err(|error| EvalError::Evaluation(format!("the shape `{name}`: {error:?}")))
    };
    match names.value() {
        ScalarOrArrayValue::Scalar(name) => Ok((vec![parse(name)?], scalar(0))),
        ScalarOrArrayValue::Array(names) => {
            let mut distinct: Vec<&str> = Vec::new();
            let mut indices = Vec::with_capacity(names.len());
            for name in names.iter() {
                let index = match distinct.iter().position(|known| *known == name) {
                    Some(index) => index,
                    None => {
                        distinct.push(name);
                        distinct.len() - 1
                    }
                };
                indices.push(index);
            }
            let shapes = distinct
                .into_iter()
                .map(parse)
                .collect::<Result<Vec<_>>>()?;
            Ok((shapes, ScalarOrArray::new_array(indices)))
        }
    }
}

/// The scene marks of a unit.
pub(crate) fn marks(rows: &UnitRows) -> Result<Vec<Drawn>> {
    match rows.mark {
        mark::Mark::Symbol(symbol) => symbols(rows, symbol),
        mark::Mark::Line(_) => lines(rows),
        mark::Mark::Trail(_) => trails(rows),
        mark::Mark::Area(area) => areas(rows, area),
        mark::Mark::Bar(bar) => bars(rows, bar),
        mark::Mark::Rect(rect) => rects(rows, rect),
        mark::Mark::Rule(_) => rules(rows),
        mark::Mark::Text(text) => texts(rows, text),
        mark::Mark::Image(image) => images(rows, image),
        mark::Mark::Path(path) => paths(rows, path),
        mark::Mark::Geoshape(_) => Err(EvalError::Evaluation(
            "the evaluator draws geoshapes in map plots".to_string(),
        )),
    }
}

/// Whether a symbol mark fills its symbols, which it does unless it says otherwise.
pub(crate) fn symbol_filled(symbol: &proto::SymbolMark) -> bool {
    symbol.filled.unwrap_or(true)
}

/// The stroke width of a symbol mark's symbols: its own, or one pixel for unfilled symbols.
pub(crate) fn symbol_stroke_width(symbol: &proto::SymbolMark) -> Option<f32> {
    symbol
        .stroke_width
        .or((!symbol_filled(symbol)).then_some(1.0))
}

/// The names of the shapes of each row, `circle` without a shape channel.
pub(crate) fn shape_names(rows: &UnitRows) -> Result<ScalarOrArray<String>> {
    Ok(rows
        .strings(Channel::Shape)?
        .unwrap_or_else(|| scalar("circle".to_string())))
}

fn symbols(rows: &UnitRows, symbol: &proto::SymbolMark) -> Result<Vec<Drawn>> {
    let (fill, stroke) = rows.paint(symbol_filled(symbol))?;
    let (shapes, shape_index) = shapes(&shape_names(rows)?)?;
    let mark = SceneSymbolMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x: rows.position(Channel::X)?,
        y: rows.position(Channel::Y)?,
        fill,
        stroke,
        stroke_width: symbol_stroke_width(symbol),
        size: rows
            .numbers(Channel::Size)?
            .unwrap_or_else(|| scalar(rows.theme.symbol_size)),
        shapes,
        shape_index,
        angle: rows.numbers(Channel::Angle)?.unwrap_or_else(|| scalar(0.0)),
        fill_pattern: pattern_fills(rows.patterns(Channel::Pattern)?),
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

fn lines(rows: &UnitRows) -> Result<Vec<Drawn>> {
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let (_, stroke) = rows.paint(false)?;
    let width = rows
        .numbers(Channel::StrokeWidth)?
        .unwrap_or_else(|| scalar(rows.theme.stroke_width));
    let dash = rows.dashes(Channel::StrokeDash)?;
    rows.series()?
        .into_iter()
        .enumerate()
        .map(|(index, series)| {
            let first = series[0] as usize;
            let mark = SceneLineMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                x: subset(&x, &series),
                y: subset(&y, &series),
                stroke: at(&stroke, first),
                stroke_width: at(&width, first),
                stroke_dash: dash
                    .as_ref()
                    .map(|dash| at(dash, first))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

fn trails(rows: &UnitRows) -> Result<Vec<Drawn>> {
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let (_, stroke) = rows.paint(false)?;
    let size = rows
        .numbers(Channel::Size)?
        .unwrap_or_else(|| scalar(rows.theme.trail_size));
    rows.series()?
        .into_iter()
        .enumerate()
        .map(|(index, series)| {
            let first = series[0] as usize;
            let mark = SceneTrailMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                x: subset(&x, &series),
                y: subset(&y, &series),
                size: subset(&size, &series),
                stroke: at(&stroke, first),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

fn areas(rows: &UnitRows, area: &proto::AreaMark) -> Result<Vec<Drawn>> {
    let horizontal = rows.horizontal(area.orient);
    let (along, across, across2) = if horizontal {
        (Channel::Y, Channel::X, Channel::X2)
    } else {
        (Channel::X, Channel::Y, Channel::Y2)
    };
    let along_values = rows.position(along)?;
    let across_values = rows.position(across)?;
    let base = match rows.has(across2) {
        true => rows.position(across2)?,
        false => scalar(rows.baseline(across)?),
    };
    let (fill, stroke) = rows.paint(true)?;
    let has_stroke = rows.has(Channel::Stroke);
    let width = rows
        .numbers(Channel::StrokeWidth)?
        .unwrap_or_else(|| scalar(if has_stroke { 1.0 } else { 0.0 }));
    let dash = rows.dashes(Channel::StrokeDash)?;
    let pattern = rows.patterns(Channel::Pattern)?;
    rows.series()?
        .into_iter()
        .enumerate()
        .map(|(index, series)| {
            let first = series[0] as usize;
            let (x, y, x2, y2) = if horizontal {
                (
                    subset(&across_values, &series),
                    subset(&along_values, &series),
                    subset(&base, &series),
                    ScalarOrArray::new_scalar(0.0),
                )
            } else {
                (
                    subset(&along_values, &series),
                    subset(&across_values, &series),
                    ScalarOrArray::new_scalar(0.0),
                    subset(&base, &series),
                )
            };
            let mark = SceneAreaMark {
                name: format!("{}/{index}", rows.name),
                len: series.len() as u32,
                orientation: if horizontal {
                    AreaOrientation::Horizontal
                } else {
                    AreaOrientation::Vertical
                },
                x,
                y,
                x2,
                y2,
                fill: at(&fill, first),
                fill_pattern: pattern.as_ref().and_then(|pattern| at(pattern, first)),
                stroke: at(&stroke, first),
                stroke_width: at(&width, first),
                stroke_dash: dash
                    .as_ref()
                    .map(|dash| at(dash, first))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            Ok(Drawn {
                mark: mark.into(),
                rows: rows.rows(&series)?,
            })
        })
        .collect()
}

/// The pixel span of each row along `channel`: from its position to the position of `second`,
/// across its band on a band scale without `second`, a thickness about its position on a
/// continuous key without `second`, and the whole `length` without the channel.
fn span(
    rows: &UnitRows,
    channel: Channel,
    second: Channel,
    thickness: Option<f32>,
    length: f32,
) -> Result<(ScalarOrArray<f32>, ScalarOrArray<f32>)> {
    if !rows.has(channel) {
        return Ok((scalar(0.0), scalar(length)));
    }
    let start = rows.numbers(channel)?.unwrap_or_else(|| scalar(0.0));
    if rows.has(second) {
        let offset = rows.band_center(channel);
        let end = rows.numbers(second)?.unwrap_or_else(|| scalar(0.0));
        return Ok((
            start.map(|value| value + offset),
            end.map(|value| value + offset),
        ));
    }
    if let Some(width) = rows.scale_for(channel).and_then(bandwidth) {
        return Ok((start.clone(), start.map(|value| value + width)));
    }
    let half = thickness.unwrap_or(rows.theme.bar_thickness) / 2.0;
    Ok((
        start.clone().map(|value| value - half),
        start.map(|value| value + half),
    ))
}

/// A rect mark of the spans of each row, with `inset` given up at each side.
fn rect_mark(
    rows: &UnitRows,
    (x, x2): (ScalarOrArray<f32>, ScalarOrArray<f32>),
    (y, y2): (ScalarOrArray<f32>, ScalarOrArray<f32>),
    corner_radius: Option<f32>,
    inset: f32,
) -> Result<Vec<Drawn>> {
    let (fill, stroke) = rows.paint(true)?;
    let has_stroke = rows.has(Channel::Stroke);
    let (x, x2) = inset_spans(x, x2, inset, rows.len());
    let (y, y2) = inset_spans(y, y2, inset, rows.len());
    let mark = SceneRectMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x,
        y,
        x2: Some(x2),
        y2: Some(y2),
        fill,
        stroke,
        stroke_width: rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(if has_stroke { 1.0 } else { 0.0 })),
        corner_radius: scalar(corner_radius.unwrap_or(0.0)),
        fill_pattern: pattern_fills(rows.patterns(Channel::Pattern)?),
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The spans from `start` to `end`, ordered and with `inset` given up at each end, but never
/// past their middle.
fn inset_spans(
    start: ScalarOrArray<f32>,
    end: ScalarOrArray<f32>,
    inset: f32,
    len: usize,
) -> (ScalarOrArray<f32>, ScalarOrArray<f32>) {
    let pair = |a: f32, b: f32| {
        let (low, high) = (a.min(b), a.max(b));
        let inset = inset.min((high - low) / 2.0).max(0.0);
        (low + inset, high - inset)
    };
    match (start.value(), end.value()) {
        (ScalarOrArrayValue::Scalar(a), ScalarOrArrayValue::Scalar(b)) => {
            let (low, high) = pair(*a, *b);
            (scalar(low), scalar(high))
        }
        _ => {
            let (low, high): (Vec<f32>, Vec<f32>) = (0..len)
                .map(|row| pair(at(&start, row), at(&end, row)))
                .unzip();
            (
                ScalarOrArray::new_array(low),
                ScalarOrArray::new_array(high),
            )
        }
    }
}

fn bars(rows: &UnitRows, bar: &proto::BarMark) -> Result<Vec<Drawn>> {
    let horizontal = rows.horizontal(bar.orient);
    let (key, key2, measure, measure2) = if horizontal {
        (Channel::Y, Channel::Y2, Channel::X, Channel::X2)
    } else {
        (Channel::X, Channel::X2, Channel::Y, Channel::Y2)
    };
    let key_length = if horizontal {
        rows.size[1]
    } else {
        rows.size[0]
    };
    let keys = span(rows, key, key2, None, key_length)?;
    let start = rows.numbers(measure)?.unwrap_or_else(|| scalar(0.0));
    let end = match rows.has(measure2) {
        true => rows.numbers(measure2)?.unwrap_or_else(|| scalar(0.0)),
        false => scalar(rows.baseline(measure)?),
    };
    let measures = (start, end);
    let (xs, ys) = if horizontal {
        (measures, keys)
    } else {
        (keys, measures)
    };
    rect_mark(rows, xs, ys, bar.corner_radius, 0.0)
}

fn rects(rows: &UnitRows, rect: &proto::RectMark) -> Result<Vec<Drawn>> {
    let xs = span(rows, Channel::X, Channel::X2, None, rows.size[0])?;
    let ys = span(rows, Channel::Y, Channel::Y2, None, rows.size[1])?;
    rect_mark(rows, xs, ys, rect.corner_radius, rect.inset.unwrap_or(0.0))
}

fn rules(rows: &UnitRows) -> Result<Vec<Drawn>> {
    let line = |channel: Channel, second: Channel, length: f32| -> Result<_> {
        Ok(match (rows.has(channel), rows.has(second)) {
            (false, _) => (scalar(0.0), scalar(length)),
            (true, false) => {
                let position = rows.position(channel)?;
                (position.clone(), position)
            }
            (true, true) => (rows.position(channel)?, rows.position(second)?),
        })
    };
    let (x, x2) = line(Channel::X, Channel::X2, rows.size[0])?;
    let (y, y2) = line(Channel::Y, Channel::Y2, rows.size[1])?;
    let (_, stroke) = rows.paint(false)?;
    let mark = SceneRuleMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        x,
        y,
        x2,
        y2,
        stroke,
        stroke_width: rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(1.0)),
        stroke_dash: rows.dashes(Channel::StrokeDash)?,
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The text of each row as markup or plain text: string values as they are, and other values
/// formatted through `format` with the engine's formatters.
pub(crate) fn text_values(
    rows: &UnitRows,
    text: &proto::TextMark,
) -> Result<(ScalarOrArray<String>, TextSyntaxMode)> {
    let syntax = proto::TextSyntax::try_from(text.syntax).unwrap_or(proto::TextSyntax::Unspecified);
    let engine = rows.engine;
    let format = &text.format;
    let formatted = rows
        .resolve(Channel::Text, &|values, _| {
            let strings = matches!(
                values.data_type(),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            );
            if strings {
                let text = cast(values, &DataType::Utf8)?;
                return Ok(ScalarOrArray::new_array(
                    text.as_string::<i32>()
                        .iter()
                        .map(|value| (value.unwrap_or_default().to_string(), false))
                        .collect(),
                ));
            }
            let values = engine
                .format_array(values, format, PrecisionSharing::PerValue)
                .map_err(|error| eval_error("the text of a text mark", error))?;
            Ok(ScalarOrArray::new_array(
                values
                    .into_iter()
                    .map(|value| match value {
                        LabelValue::Markup(markup) => (markup, true),
                        LabelValue::Str(text) => (text, false),
                        _ => (String::new(), false),
                    })
                    .collect(),
            ))
        })?
        .unwrap_or_else(|| scalar((String::new(), false)));
    if syntax == proto::TextSyntax::Typst {
        return Ok((
            formatted.map(|(text, _)| text.clone()),
            TextSyntaxMode::TypstMarkup,
        ));
    }
    let markup = match formatted.value() {
        ScalarOrArrayValue::Scalar((_, markup)) => *markup,
        ScalarOrArrayValue::Array(values) => values.iter().any(|(_, markup)| *markup),
    };
    if markup {
        return Ok((
            formatted.map(|(text, markup)| {
                if *markup {
                    text.clone()
                } else {
                    escape_text(text)
                }
            }),
            TextSyntaxMode::TypstMarkup,
        ));
    }
    let syntax = match syntax {
        proto::TextSyntax::PlainLines => TextSyntaxMode::PlainLines,
        _ => TextSyntaxMode::Plain,
    };
    Ok((formatted.map(|(text, _)| text.clone()), syntax))
}

/// The alignment and the baseline of a text mark: its own, or the center and the middle.
pub(crate) fn text_anchor(text: &proto::TextMark) -> (TextAlign, TextBaseline) {
    let align = match proto::TextAlign::try_from(text.align) {
        Ok(proto::TextAlign::Left) => TextAlign::Left,
        Ok(proto::TextAlign::Right) => TextAlign::Right,
        _ => TextAlign::Center,
    };
    let baseline = match proto::TextBaseline::try_from(text.baseline) {
        Ok(proto::TextBaseline::Top) => TextBaseline::Top,
        Ok(proto::TextBaseline::Alphabetic) => TextBaseline::Alphabetic,
        Ok(proto::TextBaseline::Bottom) => TextBaseline::Bottom,
        _ => TextBaseline::Middle,
    };
    (align, baseline)
}

/// The scene text mark of a unit's rows at `x` and `y`.
pub(crate) fn text_mark(
    rows: &UnitRows,
    text: &proto::TextMark,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
) -> Result<SceneTextMark> {
    let (values, syntax) = text_values(rows, text)?;
    let (align, baseline) = text_anchor(text);
    let (fill, _) = rows.paint(true)?;
    let color = fill.map(|color| match color {
        ColorOrGradient::Color(color) => *color,
        ColorOrGradient::GradientIndex(_) => rows.theme.mark_color,
    });
    let font = if text.font_family.is_empty() {
        rows.theme.font_family.clone()
    } else {
        text.font_family.clone()
    };
    Ok(SceneTextMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        text: values,
        text_syntax: syntax,
        x,
        y,
        align: scalar(align),
        baseline: scalar(baseline),
        angle: rows.numbers(Channel::Angle)?.unwrap_or_else(|| scalar(0.0)),
        color,
        font: scalar(font),
        font_size: rows
            .numbers(Channel::Size)?
            .unwrap_or_else(|| scalar(rows.theme.font_size)),
        font_weight: scalar(FontWeight(
            text.font_weight
                .map_or(FontWeight::NORMAL.0, |weight| weight as u16),
        )),
        ..Default::default()
    })
}

fn texts(rows: &UnitRows, text: &proto::TextMark) -> Result<Vec<Drawn>> {
    let mark = text_mark(
        rows,
        text,
        rows.position(Channel::X)?,
        rows.position(Channel::Y)?,
    )?;
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The alignment and the baseline of an image mark: its own, or the center and the middle.
pub(crate) fn image_anchor(image: &proto::ImageMark) -> (ImageAlign, ImageBaseline) {
    let align = match proto::ImageAlign::try_from(image.align) {
        Ok(proto::ImageAlign::Left) => ImageAlign::Left,
        Ok(proto::ImageAlign::Right) => ImageAlign::Right,
        _ => ImageAlign::Center,
    };
    let baseline = match proto::ImageBaseline::try_from(image.baseline) {
        Ok(proto::ImageBaseline::Top) => ImageBaseline::Top,
        Ok(proto::ImageBaseline::Bottom) => ImageBaseline::Bottom,
        _ => ImageBaseline::Middle,
    };
    (align, baseline)
}

/// The width and the height of an image mark's images: its own, or the theme's.
pub(crate) fn image_size(image: &proto::ImageMark, theme: &Theme) -> [f32; 2] {
    [
        image.width.unwrap_or(theme.image_size[0]),
        image.height.unwrap_or(theme.image_size[1]),
    ]
}

fn images(rows: &UnitRows, image: &proto::ImageMark) -> Result<Vec<Drawn>> {
    let [width, height] = image_size(image, rows.theme);
    let values = match rows
        .encoding(Channel::Image)
        .and_then(|encoding| encoding.value.as_ref())
    {
        Some(value) => rows.array(Source::of_encoding(value))?,
        None => return Ok(Vec::new()),
    };
    let images: Vec<SceneImageSource> = if matches!(
        values.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) {
        let urls = cast(&values, &DataType::Utf8)?;
        urls.as_string::<i32>()
            .iter()
            .map(|url| {
                SceneImageSource::Resource(SceneImageResource {
                    url: url.unwrap_or_default().to_string(),
                    width: width.round() as u32,
                    height: height.round() as u32,
                    placeholder: None,
                })
            })
            .collect()
    } else {
        Coercer::default()
            .to_image(&values)
            .map_err(scale_error)?
            .as_vec(values.len(), None)
            .into_iter()
            .map(|image| SceneImageSource::Inline(Arc::new(image)))
            .collect()
    };
    let images = if images.len() == 1 && rows.len() != 1 {
        ScalarOrArray::new_scalar(images[0].clone())
    } else {
        ScalarOrArray::new_array(images)
    };
    let (align, baseline) = image_anchor(image);
    let x = rows.position(Channel::X)?;
    let y = rows.position(Channel::Y)?;
    let (x, y, width, height, align, baseline) = if rows.has(Channel::X2) && rows.has(Channel::Y2) {
        let x2 = rows.position(Channel::X2)?;
        let y2 = rows.position(Channel::Y2)?;
        let len = rows.len();
        let (left, right): (Vec<f32>, Vec<f32>) = (0..len)
            .map(|row| (at(&x, row).min(at(&x2, row)), at(&x, row).max(at(&x2, row))))
            .unzip();
        let (top, bottom): (Vec<f32>, Vec<f32>) = (0..len)
            .map(|row| (at(&y, row).min(at(&y2, row)), at(&y, row).max(at(&y2, row))))
            .unzip();
        let widths = left.iter().zip(&right).map(|(l, r)| r - l).collect();
        let heights = top.iter().zip(&bottom).map(|(t, b)| b - t).collect();
        (
            ScalarOrArray::new_array(left),
            ScalarOrArray::new_array(top),
            ScalarOrArray::new_array(widths),
            ScalarOrArray::new_array(heights),
            ImageAlign::Left,
            ImageBaseline::Top,
        )
    } else {
        (x, y, scalar(width), scalar(height), align, baseline)
    };
    let mark = SceneImageMark {
        name: rows.name.clone(),
        len: rows.len() as u32,
        image: images,
        x,
        y,
        width,
        height,
        align: scalar(align),
        baseline: scalar(baseline),
        aspect: image.aspect.unwrap_or(true),
        smooth: image.smooth.unwrap_or(true),
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}

/// The SVG paths of each row.
pub(crate) fn path_values(rows: &UnitRows) -> Result<ScalarOrArray<lyon_path::Path>> {
    let values = match rows
        .encoding(Channel::Path)
        .and_then(|encoding| encoding.value.as_ref())
    {
        Some(value) => rows.array(Source::of_encoding(value))?,
        None => return Ok(scalar(lyon_path::Path::new())),
    };
    let paths = Coercer::default().to_path(&values).map_err(scale_error)?;
    Ok(if values.len() == 1 && rows.len() != 1 {
        paths.to_scalar_if_len_one()
    } else {
        paths
    })
}

fn paths(rows: &UnitRows, path: &proto::PathMark) -> Result<Vec<Drawn>> {
    let x = if rows.places(Channel::X) {
        rows.position(Channel::X)?
    } else {
        scalar(0.0)
    };
    let y = if rows.places(Channel::Y) {
        rows.position(Channel::Y)?
    } else {
        scalar(0.0)
    };
    let angle = rows.numbers(Channel::Angle)?.unwrap_or_else(|| scalar(0.0));
    let len = rows.len();
    let transform = ScalarOrArray::new_array(
        (0..len)
            .map(|row| {
                PathTransform::rotation(Angle::degrees(at(&angle, row)))
                    .then_translate(Vector2D::new(at(&x, row), at(&y, row)))
            })
            .collect(),
    );
    let (fill, stroke) = rows.paint(true)?;
    let mark = ScenePathMark {
        name: rows.name.clone(),
        len: len as u32,
        path: path_values(rows)?,
        fill,
        stroke,
        stroke_width: path.stroke_width,
        transform,
        fill_pattern: pattern_fills(rows.patterns(Channel::Pattern)?),
        ..Default::default()
    };
    Ok(vec![Drawn {
        mark: mark.into(),
        rows: rows.table.clone(),
    }])
}
