//! The drawn rows of a layer and the values of its channels: through the channel's scale, from a
//! constant, or, without an encoding, from the computed property of the same name of the layer's
//! `mark` node, with the first conditional value that a row's predicate selects.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use arrow::{
    array::{new_null_array, Array, ArrayRef, AsArray, Float64Array, RecordBatch, UInt32Array},
    compute::{cast, take_record_batch},
    datatypes::DataType,
    row::{RowConverter, SortField},
};
use avenger_chart::{
    layer::{LayerRef, MarkMessage},
    proto::{self, conditional_value, encoding, Channel},
    resolve::scale_channel,
    Chart, FitKind,
};
use avenger_color::{apply_opacity_to_color, ColorOrGradient};
use avenger_common::{
    pattern::PatternFill,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{
    band, coerce::Coercer, nested_band::NestedBandLayout, ConfiguredScale,
};
use avenger_selection::{Condition, SelectionState};
use avenger_style::{Computed, ComputedStyle, ComputedStyles, NodeId, Paint, Property};
use avenger_typst_label::LabelEngine;

use crate::{convert::selection::predicate, EvalError, EvalWarning};

pub(crate) type Result<T> = std::result::Result<T, EvalError>;

/// The rows of `table` that draw an instance: those whose encoded position columns hold values.
pub(crate) fn drawn_rows(mark: &dyn MarkMessage, table: &RecordBatch) -> Result<RecordBatch> {
    let positions = mark.kind().pairs().concat();
    let columns: Vec<ArrayRef> = mark
        .encodings()
        .filter(|(channel, _)| positions.contains(channel))
        .filter_map(|(channel, encoding)| match &encoding.kind {
            Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)) => {
                let column = table.column_by_name(name)?;
                Some(crate::lumps::channel_values(column, channel))
            }
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

/// Where a value of an encoding or of a conditional value comes from, and whether it goes through
/// the channel's scale.
#[derive(Clone, Copy)]
enum Source<'a> {
    Column(&'a str),
    ScaledColumn(&'a str),
    Constant(&'a proto::ValueRef),
    Scaled(&'a proto::ValueRef),
}

impl<'a> Source<'a> {
    fn of_encoding(value: &'a encoding::Kind) -> Self {
        match value {
            encoding::Kind::DirectField(name) => Self::Column(name),
            encoding::Kind::ScaledField(name) => Self::ScaledColumn(name),
            encoding::Kind::DirectValue(value) => Self::Constant(value),
            encoding::Kind::ScaledValue(value) => Self::Scaled(value),
        }
    }

    fn of_conditional(value: &'a conditional_value::Kind) -> Self {
        match value {
            conditional_value::Kind::DirectField(name) => Self::Column(name),
            conditional_value::Kind::ScaledField(name) => Self::ScaledColumn(name),
            conditional_value::Kind::DirectValue(value) => Self::Constant(value),
            conditional_value::Kind::ScaledValue(value) => Self::Scaled(value),
        }
    }
}

/// A converter of the values of a channel, through its scale when it has one.
pub(crate) type Convert<'c, T> =
    &'c dyn Fn(&ArrayRef, Option<&ConfiguredScale>) -> Result<ScalarOrArray<T>>;

/// The value of `values` at `row`.
pub(crate) fn at<T: Clone + Sync>(values: &ScalarOrArray<T>, row: usize) -> T {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values[row].clone(),
    }
}

/// The values of `values` at `rows`.
pub(crate) fn subset<T: Clone + Sync>(values: &ScalarOrArray<T>, rows: &[u32]) -> ScalarOrArray<T> {
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

/// The drawn rows of a layer, with what encodes them: the scales of its channels, the masks of its
/// conditional values, and the computed style of its `mark` node.
pub(crate) struct LayerRows<'a> {
    pub(crate) chart: &'a Chart,
    /// The layer, whose `mark` oneof holds the options of its mark.
    pub(crate) layer: LayerRef<'a>,
    pub(crate) mark: &'a dyn MarkMessage,
    /// The computed style of the layer's `mark` node.
    pub(crate) style: &'a ComputedStyle,
    /// The computed styles of the chart, with the layer's `mark` node, which resolve a paint of
    /// `currentColor` against the color of each row.
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) node: NodeId,
    /// The rows that draw an instance, in order.
    pub(crate) table: RecordBatch,
    /// The partition values of the layer, by column.
    pub(crate) partition: &'a BTreeMap<String, ArrayRef>,
    /// The scales of the layer's encodings, by channel: the position scales at the plot's size,
    /// and the non-position scales.
    pub(crate) scales: BTreeMap<Channel, &'a ConfiguredScale>,
    /// Whether each conditional value of each channel selects each row.
    pub(crate) masks: BTreeMap<Channel, Vec<Vec<bool>>>,
    /// The kind of fit of the scale of each channel that the layer encodes through a scale.
    pub(crate) kinds: BTreeMap<Channel, FitKind>,
    pub(crate) engine: &'a LabelEngine,
    /// The size of the plot.
    pub(crate) size: [f32; 2],
    /// The name of the layer's marks.
    pub(crate) name: String,
}

impl<'a> LayerRows<'a> {
    pub(crate) fn len(&self) -> usize {
        self.table.num_rows()
    }

    pub(crate) fn encoding(&self, channel: Channel) -> Option<&proto::Encoding> {
        self.mark.encoding(channel)
    }

    pub(crate) fn has(&self, channel: Channel) -> bool {
        self.encoding(channel).is_some()
    }

    /// The scale that `channel` goes through: its own, or the scale of the first channel of its
    /// pair, when the channel's encoding has a scale.
    pub(crate) fn scale_for(&self, channel: Channel) -> Option<&ConfiguredScale> {
        let first = scale_channel(channel);
        self.encoding(first)?.scale.as_ref()?;
        self.scales.get(&first).copied()
    }

    /// The values of `column`: the table's, or the layer's partition value on every row.
    pub(crate) fn column(&self, name: &str) -> Option<ArrayRef> {
        if let Some(values) = self.table.column_by_name(name) {
            return Some(values.clone());
        }
        let value = self.partition.get(name)?;
        arrow::compute::take(value, &UInt32Array::from(vec![0; self.len()]), None).ok()
    }

    /// The values of `source` on `channel`, which takes the starts or the ends of a column of
    /// spans.
    fn array(&self, channel: Channel, source: Source) -> Result<ArrayRef> {
        match source {
            Source::Column(name) | Source::ScaledColumn(name) => {
                let column = (self.column(name))
                    .expect("a valid layer's table or partition has its columns");
                Ok(crate::lumps::channel_values(&column, channel))
            }
            Source::Constant(value) | Source::Scaled(value) => Ok(self.chart.value(value).clone()),
        }
    }

    fn convert<T: Clone + Sync>(
        &self,
        channel: Channel,
        source: Source,
        scale: Option<&ConfiguredScale>,
        convert: Convert<T>,
    ) -> Result<ScalarOrArray<T>> {
        let values = self.array(channel, source)?;
        // A plain value is drawn as it is, even in an encoding that has a scale.
        let scale = match source {
            Source::Column(_) | Source::Constant(_) => None,
            Source::ScaledColumn(_) | Source::Scaled(_) => scale,
        };
        let converted = convert(&values, scale)?;
        Ok(match source {
            Source::Column(_) | Source::ScaledColumn(_) => converted,
            _ => converted.to_scalar_if_len_one(),
        })
    }

    /// The value of `channel` for each row: the value of the first conditional value that selects
    /// the row, or the encoding's value, or `None` without an encoding.
    pub(crate) fn resolve<T: Clone + Sync>(
        &self,
        channel: Channel,
        convert: Convert<T>,
    ) -> Result<Option<ScalarOrArray<T>>> {
        let Some(encoding) = self.encoding(channel) else {
            return Ok(None);
        };
        let value = encoding
            .kind
            .as_ref()
            .expect("a valid encoding has a value");
        let scale = self.scale_for(channel);
        let base = self.convert(channel, Source::of_encoding(value), scale, convert)?;
        if encoding.conditional.is_empty() {
            return Ok(Some(base));
        }
        let len = self.len();
        let masks = self.masks.get(&channel);
        let alternatives: Vec<Vec<T>> = (encoding.conditional.iter())
            .map(|conditional| {
                let value = conditional
                    .kind
                    .as_ref()
                    .expect("a valid condition has a value");
                Ok(self
                    .convert(channel, Source::of_conditional(value), scale, convert)?
                    .as_vec(len, None))
            })
            .collect::<Result<_>>()?;
        let base = base.as_vec(len, None);
        let merged = (0..len)
            .map(|row| {
                (alternatives.iter().enumerate())
                    .find(|(index, _)| masks.is_some_and(|masks| masks[*index][row]))
                    .map(|(_, values)| values[row].clone())
                    .unwrap_or_else(|| base[row].clone())
            })
            .collect();
        Ok(Some(ScalarOrArray::new_array(merged)))
    }

    /// These rows with every row taking the chosen value of each channel of `choices`: 0 for the
    /// encoding's value, and `k` for its `k`th conditional value.
    pub(crate) fn choosing(&self, choices: &[(Channel, usize)]) -> LayerRows<'a> {
        let mut masks = self.masks.clone();
        for (channel, choice) in choices {
            let Some(encoding) = self.encoding(*channel) else {
                continue;
            };
            let chosen = (0..encoding.conditional.len())
                .map(|index| vec![index + 1 == *choice; self.len()])
                .collect();
            masks.insert(*channel, chosen);
        }
        LayerRows {
            table: self.table.clone(),
            scales: self.scales.clone(),
            masks,
            kinds: self.kinds.clone(),
            name: self.name.clone(),
            ..*self
        }
    }

    /// Every value that `channel` can take for each row: the encoding's value and the value of
    /// each conditional value, which bound the reach of a mark at every selection.
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
        if let Some(value) = &encoding.kind {
            values.push(self.convert(channel, Source::of_encoding(value), scale, convert)?);
        }
        for conditional in &encoding.conditional {
            if let Some(value) = &conditional.kind {
                values.push(self.convert(
                    channel,
                    Source::of_conditional(value),
                    scale,
                    convert,
                )?);
            }
        }
        Ok(values)
    }

    /// The numbers of `channel`. A direct position of a Cartesian mark is a fraction of the plot
    /// along its dimension, from the left to the right for `x` and `x2`, and from the bottom to the
    /// top for `y` and `y2`, which the numbers give in pixels from the plot's top left corner.
    pub(crate) fn numbers(&self, channel: Channel) -> Result<Option<ScalarOrArray<f32>>> {
        let fraction = match (self.layer, channel) {
            (LayerRef::Cartesian(_), Channel::X | Channel::X2) => Some((self.size[0], false)),
            (LayerRef::Cartesian(_), Channel::Y | Channel::Y2) => Some((self.size[1], true)),
            _ => None,
        };
        let Some((length, upward)) = fraction else {
            return self.resolve(channel, &numbers);
        };
        self.resolve(channel, &|values, scale| {
            let values = numbers(values, scale)?;
            Ok(match (scale, upward) {
                (Some(_), _) => values,
                (None, false) => values.map(|fraction| fraction * length),
                (None, true) => values.map(|fraction| (1.0 - fraction) * length),
            })
        })
    }

    pub(crate) fn colors(
        &self,
        channel: Channel,
    ) -> Result<Option<ScalarOrArray<ColorOrGradient>>> {
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

    /// The numbers of `channel`, or the computed `property` of the mark node without an encoding.
    pub(crate) fn numbers_or(
        &self,
        channel: Channel,
        property: Property,
    ) -> Result<ScalarOrArray<f32>> {
        Ok(match self.numbers(channel)? {
            Some(values) => values,
            // A percentage `size` that no band divides draws at the initial `size`.
            None => scalar(
                (self.style.number(property))
                    .unwrap_or_else(|| property.initial().trim().parse().unwrap_or(0.0)),
            ),
        })
    }

    /// The computed `size` of the mark node as a fraction of its band, when it is a percentage and
    /// the layer does not encode `size`.
    pub(crate) fn band_size(&self) -> Option<f32> {
        match (self.has(Channel::Size), self.style.get(Property::Size)) {
            (false, Computed::Percentage(fraction)) => Some(*fraction),
            _ => None,
        }
    }

    /// The bands that a percentage `size` divides: of `x`, or else of `y`.
    pub(crate) fn size_bands(&self) -> Result<Option<ScalarOrArray<(f32, f32)>>> {
        Ok(match self.bands(Channel::X)? {
            Some(bands) => Some(bands),
            None => self.bands(Channel::Y)?,
        })
    }

    /// Why a percentage `size` draws at the initial `size`: a mark other than a symbol, a bar,
    /// and a rect, or one without a band, has no band that it divides.
    pub(crate) fn size_warning(&self) -> Result<Option<String>> {
        use avenger_chart::layer::{CoordKind, MarkType};
        if self.band_size().is_none() {
            return Ok(None);
        }
        let divides = self.mark.kind() == CoordKind::Cartesian
            && matches!(
                self.mark.mark_type(),
                MarkType::Symbol | MarkType::Bar | MarkType::Rect
            )
            && self.size_bands()?.is_some();
        Ok((!divides).then(|| {
            "a percentage `size` is a fraction of the band of the mark's position, which it \
             does not have, so it draws at the initial `size`"
                .to_string()
        }))
    }

    /// The dashes of `stroke_dash`, or the computed `stroke-dash` of the mark node: no dash for
    /// `none`.
    pub(crate) fn dashes_or_style(&self) -> Result<Option<ScalarOrArray<Vec<f32>>>> {
        if let Some(dashes) = self.dashes(Channel::StrokeDash)? {
            return Ok(Some(dashes));
        }
        Ok(match self.style.get(Property::StrokeDash) {
            Computed::Sequence(parts) | Computed::List(parts) => {
                let lengths: Vec<f32> = (parts.iter())
                    .filter_map(|part| match part {
                        Computed::Number(value) => Some(*value),
                        Computed::Integer(value) => Some(*value as f32),
                        _ => None,
                    })
                    .collect();
                (!lengths.is_empty()).then(|| scalar(lengths))
            }
            Computed::Number(value) => Some(scalar(vec![*value])),
            _ => None,
        })
    }

    /// The band of each row along `channel` on a band or a nested band scale: its start, the edge
    /// nearer the range start, and its width, in pixels, or `None` on another scale. On a nested
    /// band scale, the band is at the encoding's `band_level`, the leaf without one, so the bands
    /// of an outer level differ in width, and a row whose group the domain does not hold has a
    /// band at NaN, which draws nothing. The second channel of a pair takes the level of the
    /// first.
    pub(crate) fn bands(&self, channel: Channel) -> Result<Option<ScalarOrArray<(f32, f32)>>> {
        let Some(scale) = self.scale_for(channel) else {
            return Ok(None);
        };
        let nested = match scale.scale_impl.scale_type() {
            "band" => false,
            "nested_band" => true,
            _ => return Ok(None),
        };
        let level =
            (self.encoding(scale_channel(channel))).and_then(|encoding| encoding.band_level);
        let convert = |values: &ArrayRef, scale: Option<&ConfiguredScale>| {
            let scale = scale.expect("a band channel goes through its scale");
            if !nested {
                let width = band::bandwidth(&scale.config)? as f32;
                return Ok(numbers(values, Some(scale))?.map(|start| (*start, width)));
            }
            let layout = NestedBandLayout::from_config(&scale.config)?;
            let level = level.map_or(layout.leaf_level(), |level| level as usize);
            let bands = (layout.bands_at(values, level)?.into_iter())
                .map(|band| match band {
                    Some((start, end)) => (start.min(end) as f32, (end - start).abs() as f32),
                    None => (f32::NAN, f32::NAN),
                })
                .collect();
            Ok(ScalarOrArray::new_array(bands))
        };
        self.resolve(channel, &convert)
    }

    /// Where the mark of each row stands in its band along `channel`: the encoding's
    /// `band_position`, or the middle of the band without one.
    pub(crate) fn band_position(&self, channel: Channel) -> f32 {
        (self.encoding(channel))
            .and_then(|encoding| encoding.band_position)
            .map_or(0.5, |position| position as f32)
    }

    /// The pixel position of each row along `channel`: at its band position in its band on a band
    /// or a nested band scale, or the center of the plot when the layer does not encode the
    /// channel.
    pub(crate) fn position(&self, channel: Channel) -> Result<ScalarOrArray<f32>> {
        if let Some(bands) = self.bands(channel)? {
            let position = self.band_position(channel);
            return Ok(bands.map(|(start, width)| start + width * position));
        }
        let center = match channel {
            Channel::X | Channel::X2 => self.size[0] / 2.0,
            _ => self.size[1] / 2.0,
        };
        Ok(self.numbers(channel)?.unwrap_or_else(|| scalar(center)))
    }

    /// The paint of `property`, `fill` or `stroke`, for each row: the channel of the same name, or
    /// the mark node's computed paint, where a paint that reads `currentColor`, `auto` included,
    /// resolves against the `color` channel of each row, as the CSS `color` of the row. A gradient
    /// of a row takes the place of the paint as it is.
    fn paint_of(
        &self,
        property: Property,
        channel: Channel,
    ) -> Result<ScalarOrArray<ColorOrGradient>> {
        if let Some(values) = self.colors(channel)? {
            return Ok(values);
        }
        let painted = |paint: Paint| match paint {
            Paint::None => ColorOrGradient::transparent(),
            Paint::Color(color) | Paint::Contrast { otherwise: color } => {
                ColorOrGradient::Color(color)
            }
        };
        let reads_color = matches!(
            self.style.get(property),
            Computed::Keyword("auto") | Computed::CurrentColor { .. }
        );
        let colors = match reads_color {
            true => self.colors(Channel::Color)?,
            false => None,
        };
        let Some(colors) = colors else {
            return Ok(scalar(painted(self.style.paint(property))));
        };
        let mut resolved: HashMap<[u32; 4], ColorOrGradient> = HashMap::new();
        let mut resolve = |color: &ColorOrGradient| match color {
            ColorOrGradient::Color(color) => resolved
                .entry(color.map(f32::to_bits))
                .or_insert_with(|| painted(self.styles.paint_with(self.node, property, *color)))
                .clone(),
            gradient => gradient.clone(),
        };
        Ok(match colors.value() {
            ScalarOrArrayValue::Scalar(color) => scalar(resolve(color)),
            ScalarOrArrayValue::Array(colors) => {
                ScalarOrArray::new_array(colors.iter().map(resolve).collect())
            }
        })
    }

    /// The fill and the stroke of each row, with their alphas times the opacities.
    pub(crate) fn paint(
        &self,
    ) -> Result<(
        ScalarOrArray<ColorOrGradient>,
        ScalarOrArray<ColorOrGradient>,
    )> {
        let fill = self.paint_of(Property::Fill, Channel::Fill)?;
        let stroke = self.paint_of(Property::Stroke, Channel::Stroke)?;
        let opacity = self.numbers_or(Channel::Opacity, Property::Opacity)?;
        let fill = fade(
            fade(fill, &opacity),
            &self.numbers_or(Channel::FillOpacity, Property::FillOpacity)?,
        );
        let stroke = fade(
            fade(stroke, &opacity),
            &self.numbers_or(Channel::StrokeOpacity, Property::StrokeOpacity)?,
        );
        Ok((fill, stroke))
    }

    /// Whether the mark of each row strokes: its stroke is not transparent.
    pub(crate) fn strokes(&self) -> Result<bool> {
        let (_, stroke) = self.paint()?;
        let visible = |color: &ColorOrGradient| match color {
            ColorOrGradient::Color(color) => color[3] > 0.0,
            ColorOrGradient::GradientIndex(_) => true,
        };
        Ok(match stroke.value() {
            ScalarOrArrayValue::Scalar(color) => visible(color),
            ScalarOrArrayValue::Array(colors) => colors.iter().any(visible),
        })
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
        .filter_map(|channel| match &self.encoding(channel)?.kind {
            Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)) => {
                self.column(name)
            }
            _ => None,
        })
        .collect();
        let len = self.len() as u32;
        if keys.is_empty() {
            return Ok(match len {
                0 => Vec::new(),
                _ => vec![(0..len).collect()],
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

    /// The baseline of a bar or an area along `channel`: zero when the scale's range holds it,
    /// and otherwise the start of the range.
    pub(crate) fn baseline(&self, channel: Channel) -> Result<f32> {
        let Some(scale) = self.scale_for(channel) else {
            return Ok(match channel {
                Channel::Y => self.size[1],
                _ => 0.0,
            });
        };
        let (start, end) = scale.numeric_interval_range()?;
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

    /// The kind of fit of the scale of `channel`, when the layer encodes it through a scale.
    pub(crate) fn kind(&self, channel: Channel) -> Option<FitKind> {
        self.kinds.get(&scale_channel(channel)).copied()
    }

    /// Whether a bar or an area of the layer is horizontal: when its orient says so, or else when
    /// only the scale of its first position, `x` or `theta`, is continuous.
    pub(crate) fn horizontal(&self, orient: i32) -> bool {
        let continuous = |channel| self.kind(channel) == Some(FitKind::Interval);
        let [first, second] = [0, 1].map(|index| self.mark.kind().positions()[index]);
        match proto::Orient::try_from(orient).unwrap_or_default() {
            proto::Orient::Horizontal => true,
            proto::Orient::Vertical => false,
            proto::Orient::Unspecified => continuous(first) && !continuous(second),
        }
    }
}

pub(crate) fn numbers(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<f32>> {
    Ok(match scale {
        Some(scale) => scale.scale_to_numeric(values)?,
        None => Coercer::default().to_numeric(values, None)?,
    })
}

fn colors(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<ColorOrGradient>> {
    Ok(match scale {
        Some(scale) => scale.scale_to_color(values)?,
        None => Coercer::default().to_color(values, None)?,
    })
}

pub(crate) fn strings(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<String>> {
    match scale {
        Some(scale) => Ok(scale.scale_to_string(values)?),
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
        Some(scale) => scale.scale(values)?,
        None => values.clone(),
    };
    Ok(Coercer::default().to_stroke_dash(&values)?)
}

fn patterns(
    values: &ArrayRef,
    scale: Option<&ConfiguredScale>,
) -> Result<ScalarOrArray<Option<PatternFill>>> {
    Ok(match scale {
        Some(scale) => scale.scale_to_pattern(values)?,
        None => Coercer::default().to_pattern(values)?,
    })
}

/// `colors` with their alpha times `opacity`.
pub(crate) fn fade(
    colors: ScalarOrArray<ColorOrGradient>,
    opacity: &ScalarOrArray<f32>,
) -> ScalarOrArray<ColorOrGradient> {
    match (colors.value(), opacity.value()) {
        (_, ScalarOrArrayValue::Scalar(opacity)) if *opacity == 1.0 => colors,
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

/// The masks of the conditional values of each channel of a layer: whether each predicate
/// selects each drawn row, over the selection states. A condition of a column that neither the
/// layer's table nor its partition has selects no row, with a warning.
pub(crate) fn masks(
    rows: &LayerRows,
    states: &BTreeMap<String, SelectionState>,
    owners: &[String],
    path: &str,
    warnings: &mut Vec<EvalWarning>,
) -> Result<BTreeMap<Channel, Vec<Vec<bool>>>> {
    let mark = rows.mark;
    let faceted = !rows.partition.is_empty();
    let len = rows.len();
    let mut masks = BTreeMap::new();
    for (channel, encoding) in mark.encodings() {
        if encoding.conditional.is_empty() {
            continue;
        }
        let mut channel_masks = Vec::with_capacity(encoding.conditional.len());
        for conditional in &encoding.conditional {
            let predicate_message = conditional
                .predicate
                .as_ref()
                .expect("a valid condition has a predicate");
            let missing = missing_columns(rows, states, &tested(predicate_message), path, warnings);
            let types = |name: &str| {
                rows.column(name)
                    .map(|values| values.data_type().clone())
                    .or_else(|| missing.get(name).cloned())
            };
            let test = predicate(predicate_message, owners, faceted).prepare(states, types)?;
            let column = |name: &str| {
                rows.column(name).or_else(|| {
                    missing
                        .get(name)
                        .map(|data_type| new_null_array(data_type, len))
                })
            };
            let mask = test.mask(len, column, |name: &str| rows.column(name))?;
            channel_masks.push(mask.values().iter().collect());
        }
        masks.insert(channel, channel_masks);
    }
    Ok(masks)
}

/// The names of the selections that `predicate` tests.
fn tested(predicate: &proto::Predicate) -> Vec<&str> {
    use proto::predicate::Predicate as Kind;
    match predicate.predicate.as_ref() {
        Some(Kind::Selected(selected)) => vec![selected.selection.as_str()],
        Some(Kind::All(list) | Kind::Any(list)) => {
            list.predicates.iter().flat_map(tested).collect()
        }
        Some(Kind::Negate(inner)) => tested(inner),
        None => Vec::new(),
    }
}

/// The columns that the clauses of the selections `names` check and the layer lacks, with the
/// types of their values, each with a warning that its condition holds for no row.
fn missing_columns(
    rows: &LayerRows,
    states: &BTreeMap<String, SelectionState>,
    names: &[&str],
    path: &str,
    warnings: &mut Vec<EvalWarning>,
) -> BTreeMap<String, DataType> {
    let mut missing = BTreeMap::new();
    for (name, state) in states
        .iter()
        .filter(|(name, _)| names.contains(&name.as_str()))
    {
        for condition in state.clauses.iter().flat_map(|clause| &clause.conditions) {
            for (column, data_type) in value_types(condition) {
                if rows.column(column).is_some() || missing.contains_key(column) {
                    continue;
                }
                let warning = EvalWarning::NotDrawn {
                    path: path.to_string(),
                    reason: format!(
                        "the selection `{name}` checks the column `{column}`, which the layer's \
                         table and partition do not have, so its condition holds for no row"
                    ),
                };
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
                missing.insert(column.to_string(), data_type);
            }
        }
    }
    missing
}

/// The columns that a condition checks, with the types of its values.
fn value_types(condition: &Condition) -> Vec<(&str, DataType)> {
    match condition {
        Condition::Set {
            columns, values, ..
        } => (columns.iter())
            .zip(values)
            .map(|(column, values)| (column.as_str(), values.data_type().clone()))
            .collect(),
        Condition::Range { column, ends, .. } => vec![(column.as_str(), ends.data_type().clone())],
        Condition::Polygon {
            x_column,
            y_column,
            polygons,
            ..
        } => match polygons.first() {
            Some((x, y)) => vec![
                (x_column.as_str(), x.data_type().clone()),
                (y_column.as_str(), y.data_type().clone()),
            ],
            None => Vec::new(),
        },
        Condition::CyclicRange { column, .. } => vec![(column.as_str(), DataType::Float64)],
    }
}

#[cfg(test)]
mod tests;
