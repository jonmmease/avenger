//! The builder of charts: a unit of one mark over one table, and the combinations of charts in
//! overlays, rows, columns, and wraps. It writes the messages of the format, with the scale types
//! that it infers, and it checks each chart that it returns. The renderer applies the defaults of
//! guides and their titles, because they depend on the scales that the combined tree shares.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, RecordBatch, StructArray},
    datatypes::DataType,
};
use avenger_scales::{scalar::Scalar, scales::domain_summary::DomainSummary};

use crate::{
    payload,
    proto::{
        self, axis, category_fit, condition, coord, domain, domain_fit, encoding, guide, mark,
        node::Node, range, Channel,
    },
    tree,
    validate::{dictionary_values, is_position, takes_scale, FitKind},
    Chart, ChartError, FORMAT_VERSION,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The value of an encoding or of a condition.
#[derive(Clone, Debug)]
pub enum Value {
    /// The name of a column of the unit's table.
    Column(String),
    /// A value that the mark draws as it is, in the form that the comment of the channel gives,
    /// such as a CSS color.
    Constant(Scalar),
    /// A data value that goes through the scale of the channel, such as the position of a
    /// reference line.
    ScaledConstant(Scalar),
}

impl Value {
    fn encoding(&self, values: &mut Vec<ArrayRef>) -> encoding::Value {
        match self {
            Self::Column(name) => encoding::Value::Column(name.clone()),
            Self::Constant(value) => encoding::Value::Constant(push(values, value.0.clone())),
            Self::ScaledConstant(value) => {
                encoding::Value::ScaledConstant(push(values, value.0.clone()))
            }
        }
    }

    fn condition(&self, values: &mut Vec<ArrayRef>) -> condition::Value {
        match self.encoding(values) {
            encoding::Value::Column(name) => condition::Value::Column(name),
            encoding::Value::Constant(index) => condition::Value::Constant(index),
            encoding::Value::ScaledConstant(index) => condition::Value::ScaledConstant(index),
        }
    }
}

/// Adds `value` to the values of a chart, and returns the index that a field refers to it by.
fn push(values: &mut Vec<ArrayRef>, value: ArrayRef) -> u32 {
    values.push(value);
    (values.len() - 1) as u32
}

impl From<&str> for Value {
    fn from(column: &str) -> Self {
        Self::Column(column.to_string())
    }
}

/// The binding of a channel: its value, the scale that the value goes through, and the values
/// of the rows that predicates select.
#[derive(Clone, Debug)]
pub struct Encode {
    value: Value,
    scaling: Scaling,
    conditions: Vec<(proto::Predicate, Value)>,
    selection_field: String,
    title: Option<String>,
}

#[derive(Clone, Debug)]
enum Scaling {
    /// A scale of the type that the builder infers, for a channel that takes a scale.
    Inferred,
    /// No scale: the mark draws the values as they are.
    Raw,
    Spec(Box<Scale>),
}

impl Encode {
    /// A column through the scale of the channel, whose type the builder infers from the column
    /// and the mark. A channel that takes no scale, such as `x2` or `text`, draws the column
    /// without one.
    pub fn column(name: impl Into<String>) -> Self {
        Self::with(Value::Column(name.into()), Scaling::Inferred)
    }

    /// A column of values that the mark draws as they are, without a scale, in the form that the
    /// comment of the channel gives, such as CSS colors for `color`.
    pub fn values(name: impl Into<String>) -> Self {
        Self::with(Value::Column(name.into()), Scaling::Raw)
    }

    /// A value that the mark draws as it is for every row.
    pub fn constant(value: impl Into<Scalar>) -> Self {
        Self::with(Value::Constant(value.into()), Scaling::Raw)
    }

    /// A data value through the scale of the channel for every row.
    pub fn scaled_constant(value: impl Into<Scalar>) -> Self {
        Self::with(Value::ScaledConstant(value.into()), Scaling::Inferred)
    }

    fn with(value: Value, scaling: Scaling) -> Self {
        Self {
            value,
            scaling,
            conditions: Vec::new(),
            selection_field: String::new(),
            title: None,
        }
    }

    /// The scale of the channel. A scale without a type takes the type that the builder infers.
    pub fn scale(mut self, scale: Scale) -> Self {
        self.scaling = Scaling::Spec(Box::new(scale));
        self
    }

    /// A value for the rows that `predicate` selects. A row takes the value of the first
    /// condition whose predicate selects it.
    pub fn when(mut self, predicate: proto::Predicate, value: impl Into<Value>) -> Self {
        self.conditions.push((predicate, value.into()));
        self
    }

    /// The column that a selection through this channel records, when the encoded column
    /// derives from it.
    pub fn selection_field(mut self, column: impl Into<String>) -> Self {
        self.selection_field = column.into();
        self
    }

    /// The title of the values in the guide of the channel's scale, in place of the name of the
    /// column. A guide without a title of its own joins the titles of the encodings of its scale.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// No title from this encoding in the guide of the channel's scale.
    pub fn untitled(mut self) -> Self {
        self.title = Some(String::new());
        self
    }
}

impl From<&str> for Encode {
    fn from(column: &str) -> Self {
        Self::column(column)
    }
}

impl From<String> for Encode {
    fn from(column: String) -> Self {
        Self::column(column)
    }
}

/// The scale of a channel. Each part that the caller does not set takes its default: the type
/// that the builder infers, the unit's data as the domain, the fit of the type's kind, the range
/// of the theme or the plot, and the default guide of the channel, which the renderer gives.
#[derive(Clone, Debug, Default)]
pub struct Scale {
    scale_type: Option<String>,
    options: BTreeMap<String, Scalar>,
    domain: DomainSource,
    selection: Option<(String, Channel)>,
    zero: Option<bool>,
    nice: Option<f64>,
    empty: Option<ArrayRef>,
    order: Order,
    range: Option<RangeSource>,
    reverse: bool,
    family: String,
    guide: Option<proto::Guide>,
    guide_values: Option<ArrayRef>,
}

#[derive(Clone, Debug, Default)]
enum DomainSource {
    #[default]
    Data,
    Explicit(ArrayRef),
    Summary(DomainSummary),
}

#[derive(Clone, Debug)]
enum RangeSource {
    Values(ArrayRef),
    Palette(String),
}

#[derive(Clone, Debug, Default)]
enum Order {
    #[default]
    Ascending,
    Descending,
    Preferred(ArrayRef),
}

impl Scale {
    /// A scale of the type that the builder infers.
    pub fn new() -> Self {
        Self::default()
    }

    /// A scale of `scale_type`, the typetag name of an avenger-scales scale type, such as `log`.
    pub fn of(scale_type: impl Into<String>) -> Self {
        Self {
            scale_type: Some(scale_type.into()),
            ..Self::default()
        }
    }

    /// An option of the scale type, such as `clamp` or `padding_inner`.
    pub fn option(mut self, name: impl Into<String>, value: impl Into<Scalar>) -> Self {
        self.options.insert(name.into(), value.into());
        self
    }

    /// An explicit domain: the two ends of an interval scale, the categories of a band, point,
    /// or ordinal scale, or the thresholds of a threshold scale, as one Arrow column.
    pub fn domain(mut self, values: ArrayRef) -> Self {
        self.domain = DomainSource::Explicit(values);
        self
    }

    /// A summary of the data that the domain covers, from the producer of the chart.
    pub fn summary(mut self, summary: DomainSummary) -> Self {
        self.domain = DomainSource::Summary(summary);
        self
    }

    /// A domain from the values of `selection` for `channel`, which falls back to the explicit,
    /// summary, or data domain of the scale when the selection gives none.
    pub fn selection(mut self, selection: impl Into<String>, channel: Channel) -> Self {
        self.selection = Some((selection.into(), channel));
        self
    }

    /// Whether the fitted domain of an interval scale contains zero. By default, it does on the
    /// measure channel of a bar or an area.
    pub fn zero(mut self, zero: bool) -> Self {
        self.zero = Some(zero);
        self
    }

    /// A nice fitted domain of an interval scale, whose ends are round values for about
    /// `tick_count` ticks.
    pub fn nice(mut self, tick_count: f64) -> Self {
        self.nice = Some(tick_count);
        self
    }

    /// The domain of an interval scale when there is no data, as an explicit domain holds it.
    pub fn empty(mut self, domain: ArrayRef) -> Self {
        self.empty = Some(domain);
        self
    }

    /// The categories of the data in descending order, rather than ascending.
    pub fn descending(mut self) -> Self {
        self.order = Order::Descending;
        self
    }

    /// These categories first, in this order, and then the other categories of the data,
    /// ascending.
    pub fn preferred(mut self, categories: ArrayRef) -> Self {
        self.order = Order::Preferred(categories);
        self
    }

    /// The values of the range, such as CSS colors or sizes, as one Arrow column.
    pub fn range(mut self, values: ArrayRef) -> Self {
        self.range = Some(RangeSource::Values(values));
        self
    }

    /// A palette of the theme as the range, such as `tableau10`.
    pub fn palette(mut self, name: impl Into<String>) -> Self {
        self.range = Some(RangeSource::Palette(name.into()));
        self
    }

    /// The range from its end to its start.
    pub fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }

    /// The share family: units whose specs name one family for a channel share one scale.
    pub fn family(mut self, family: impl Into<String>) -> Self {
        self.family = family.into();
        self
    }

    /// The guide, in place of the default guide of the channel.
    pub fn guide(mut self, guide: impl Into<proto::Guide>) -> Self {
        self.guide = Some(guide.into());
        self
    }

    /// The ticks of the axis or of the colorbar, or the entries of the legend, in place of the
    /// ticks of the scale, as one Arrow column in the type of the domain. The guide sets the
    /// kind, so the scale has a guide.
    pub fn guide_values(mut self, values: ArrayRef) -> Self {
        self.guide_values = Some(values);
        self
    }

    /// The spec of a scale of `scale_type` at `path`, with its values added to `values`.
    /// `measure` gives the fit of an interval scale `zero` by default.
    fn spec(
        &self,
        scale_type: &str,
        measure: bool,
        values: &mut Vec<ArrayRef>,
        path: &str,
    ) -> Result<proto::ScaleSpec> {
        let options = self
            .options
            .iter()
            .map(|(name, value)| {
                Ok(proto::NamedOption {
                    name: name.clone(),
                    value: Some(payload::encode_option(value)?),
                })
            })
            .collect::<Result<_>>()?;
        let mut domain = match &self.domain {
            DomainSource::Data => domain::Domain::Data(proto::DataDomain {}),
            DomainSource::Explicit(domain) => {
                domain::Domain::Explicit(push(values, payload::list(domain.clone())))
            }
            DomainSource::Summary(summary) => {
                domain::Domain::Summary(push(values, Arc::new(summary.to_arrow())))
            }
        };
        let fitted = !matches!(self.domain, DomainSource::Explicit(_));
        if let Some((selection, selection_channel)) = &self.selection {
            domain = domain::Domain::Selection(Box::new(proto::SelectionDomain {
                selection: selection.clone(),
                channel: *selection_channel as i32,
                fallback: Some(Box::new(proto::Domain {
                    domain: Some(domain),
                })),
            }));
        }
        let fit = match FitKind::of(scale_type) {
            FitKind::Interval if fitted => Some(domain_fit::Fit::Interval(proto::IntervalFit {
                zero: self.zero.unwrap_or(measure),
                nice: self.nice,
                empty: self
                    .empty
                    .as_ref()
                    .map(|empty| push(values, payload::list(empty.clone()))),
            })),
            FitKind::Categories if fitted => {
                let order = match &self.order {
                    Order::Ascending => {
                        category_fit::Order::Sort(proto::CategorySort::Ascending as i32)
                    }
                    Order::Descending => {
                        category_fit::Order::Sort(proto::CategorySort::Descending as i32)
                    }
                    Order::Preferred(categories) => category_fit::Order::Preferred(push(
                        values,
                        payload::list(categories.clone()),
                    )),
                };
                Some(domain_fit::Fit::Categories(proto::CategoryFit {
                    order: Some(order),
                }))
            }
            _ => None,
        };
        let range = match &self.range {
            Some(RangeSource::Values(range)) => Some(range::Range::Values(push(
                values,
                payload::list(range.clone()),
            ))),
            Some(RangeSource::Palette(name)) => Some(range::Range::Palette(name.clone())),
            None => None,
        };
        let range = (range.is_some() || self.reverse).then_some(proto::Range {
            range,
            reverse: self.reverse,
        });
        let mut guide = self.guide.clone();
        if let Some(entries) = &self.guide_values {
            let index = push(values, payload::list(entries.clone()));
            let ticks = Some(axis::Ticks::TickValues(index));
            match guide.as_mut().and_then(|guide| guide.guide.as_mut()) {
                Some(guide::Guide::Axis(axis)) => axis.ticks = ticks,
                Some(guide::Guide::Legend(legend)) => legend.values = Some(index),
                Some(guide::Guide::Colorbar(colorbar)) => {
                    colorbar.axis.get_or_insert_default().ticks = ticks
                }
                _ => {
                    return Err(ChartError::invalid(
                        format!("{path}/guide"),
                        "guide values belong to an axis, a legend, or a colorbar that the scale \
                         sets",
                    ))
                }
            }
        }
        Ok(proto::ScaleSpec {
            r#type: scale_type.to_string(),
            options,
            domain: Some(proto::Domain {
                domain: Some(domain),
            }),
            range,
            family: self.family.clone(),
            guide,
            fit: fit.map(|fit| proto::DomainFit { fit: Some(fit) }),
        })
    }
}

/// The scale type of `channel` over values of `data_type` on `mark`: linear for numbers and
/// time for dates and times, except on the channels of discrete values, and otherwise a band
/// scale on a bar or a rect, a point scale on other marks, a nested band scale for paths of
/// categories, and an ordinal scale on the channels other than the positions.
fn infer(channel: Channel, data_type: &DataType, mark: &mark::Mark) -> &'static str {
    let data_type = dictionary_values(data_type);
    let discrete = matches!(
        channel,
        Channel::Shape | Channel::Pattern | Channel::StrokeDash
    );
    if !discrete && data_type.is_numeric() {
        "linear"
    } else if !discrete && data_type.is_temporal() {
        "time"
    } else if !is_position(channel) {
        "ordinal"
    } else if matches!(data_type, DataType::Struct(_)) {
        "nested_band"
    } else if matches!(mark, mark::Mark::Bar(_) | mark::Mark::Rect(_)) {
        "band"
    } else {
        "point"
    }
}

/// The measure channel of a bar or an area: `y`, or `radius` in a polar plot, for a vertical
/// mark, and `x`, or `theta`, for a horizontal one, which is a mark whose orient says so, or
/// whose only continuous position scale is the first.
fn measure(mark: &mark::Mark, types: &BTreeMap<Channel, String>) -> Option<Channel> {
    let orient = match mark {
        mark::Mark::Bar(bar) => bar.orient,
        mark::Mark::Area(area) => area.orient,
        _ => return None,
    };
    let (first, second) =
        if types.contains_key(&Channel::Theta) || types.contains_key(&Channel::Radius) {
            (Channel::Theta, Channel::Radius)
        } else {
            (Channel::X, Channel::Y)
        };
    let continuous = |channel| {
        types
            .get(&channel)
            .is_some_and(|scale_type| FitKind::of(scale_type) == FitKind::Interval)
    };
    let horizontal = match proto::Orient::try_from(orient) {
        Ok(proto::Orient::Horizontal) => true,
        Ok(proto::Orient::Vertical) => false,
        _ => continuous(first) && !continuous(second),
    };
    Some(if horizontal { first } else { second })
}

/// The parts of a node that every builder sets.
#[derive(Default)]
struct Common {
    id: String,
    title: Option<proto::Title>,
    partition: BTreeMap<String, Scalar>,
    selections: Vec<Selection>,
}

/// The partition values of `partition`, in the order of their columns, with their values added
/// to `values`.
fn partition_values(
    partition: &BTreeMap<String, Scalar>,
    values: &mut Vec<ArrayRef>,
) -> Vec<proto::PartitionValue> {
    partition
        .iter()
        .map(|(column, value)| proto::PartitionValue {
            column: column.clone(),
            value: push(values, value.0.clone()),
        })
        .collect()
}

macro_rules! common_methods {
    () => {
        /// The id of the node, which, with its partition, identifies it.
        pub fn id(mut self, id: impl Into<String>) -> Self {
            self.common.id = id.into();
            self
        }

        /// The title of the node.
        pub fn title(mut self, title: impl Into<proto::Title>) -> Self {
            self.common.title = Some(title.into());
            self
        }

        /// A partition value of the node: the value of `column` that selects its rows from the
        /// data that a facet grid splits.
        pub fn partition(mut self, column: impl Into<String>, value: impl Into<Scalar>) -> Self {
            self.common.partition.insert(column.into(), value.into());
            self
        }

        /// A selection of the chart, which the interactions of its plots write.
        pub fn selection(mut self, selection: Selection) -> Self {
            self.common.selections.push(selection);
            self
        }
    };
}

/// The builder of a unit: one mark over one table.
#[must_use]
pub struct UnitBuilder {
    table: RecordBatch,
    mark: mark::Mark,
    encodings: BTreeMap<Channel, Encode>,
    coord: Option<proto::Coord>,
    interactions: Vec<proto::Interaction>,
    attribution: Option<proto::Attribution>,
    common: Common,
}

impl UnitBuilder {
    common_methods!();

    /// Binds `channel`, in place of an earlier binding of it.
    pub fn encode(mut self, channel: Channel, encode: impl Into<Encode>) -> Self {
        self.encodings.insert(channel, encode.into());
        self
    }

    /// The coordinate system of the plot.
    pub fn coord(mut self, coord: impl Into<proto::Coord>) -> Self {
        self.coord = Some(coord.into());
        self
    }

    /// An interaction of the plot, after the earlier ones.
    pub fn interaction(mut self, interaction: proto::Interaction) -> Self {
        self.interactions.push(interaction);
        self
    }

    /// The credits of the plot's sources.
    pub fn attribution(mut self, attribution: proto::Attribution) -> Self {
        self.attribution = Some(attribution);
        self
    }

    /// The chart of the unit.
    pub fn build(self) -> Result<Chart> {
        let Self {
            table,
            mark,
            encodings,
            coord,
            interactions,
            attribution,
            common,
        } = self;
        let scale_of =
            |channel: Channel, encode: &Encode| match (&encode.scaling, takes_scale(channel)) {
                (Scaling::Raw, _) | (Scaling::Inferred, false) => None,
                (Scaling::Inferred, true) => Some(Scale::default()),
                (Scaling::Spec(scale), _) => Some(scale.as_ref().clone()),
            };

        let mut scales = BTreeMap::new();
        let mut types = BTreeMap::new();
        for (index, (channel, encode)) in encodings.iter().enumerate() {
            let Some(scale) = scale_of(*channel, encode) else {
                continue;
            };
            let data_type = match &encode.value {
                Value::Column(name) => table
                    .column_by_name(name)
                    .ok_or_else(|| {
                        ChartError::invalid(
                            format!("root/encodings[{index}]"),
                            format!("the unit's table has no column `{name}`"),
                        )
                    })?
                    .data_type()
                    .clone(),
                Value::Constant(value) | Value::ScaledConstant(value) => {
                    value.0.data_type().clone()
                }
            };
            let scale_type = scale
                .scale_type
                .clone()
                .unwrap_or_else(|| infer(*channel, &data_type, &mark).to_string());
            types.insert(*channel, scale_type);
            scales.insert(*channel, scale);
        }
        let measure = measure(&mark, &types);

        let mut parts = Parts::default();
        let values = &mut parts.values;
        let encodings = encodings
            .into_iter()
            .enumerate()
            .map(|(index, (channel, encode))| {
                let scale = scales
                    .get(&channel)
                    .map(|scale| {
                        scale.spec(
                            &types[&channel],
                            measure == Some(channel),
                            values,
                            &format!("root/encodings[{index}]/scale"),
                        )
                    })
                    .transpose()?;
                let conditions = encode
                    .conditions
                    .iter()
                    .map(|(predicate, value)| proto::Condition {
                        predicate: Some(predicate.clone()),
                        value: Some(value.condition(values)),
                    })
                    .collect();
                Ok(proto::Encoding {
                    channel: channel as i32,
                    value: Some(encode.value.encoding(values)),
                    scale,
                    conditions,
                    selection_field: encode.selection_field,
                    title: encode.title,
                })
            })
            .collect::<Result<_>>()?;

        let unit = proto::Unit {
            id: common.id.clone(),
            table: 0,
            mark: Some(proto::Mark { mark: Some(mark) }),
            encodings,
            coord,
            title: common.title.clone(),
            interactions,
            partition: partition_values(&common.partition, &mut parts.values),
            attribution,
        };
        parts.tables.push(table);
        finish(Node::Unit(unit), parts, common.selections)
    }
}

macro_rules! channels {
    ($($(#[$doc:meta])* $method:ident $(, $values:ident)? => $channel:ident;)*) => {
        impl UnitBuilder {
            $(
                $(#[$doc])*
                pub fn $method(self, encode: impl Into<Encode>) -> Self {
                    self.encode(Channel::$channel, encode)
                }

                $(
                    #[doc = concat!(
                        "`", stringify!($method), "` from a column of values that the mark draws \
                         as they are, without a scale."
                    )]
                    pub fn $values(self, column: impl Into<String>) -> Self {
                        self.encode(Channel::$channel, Encode::values(column))
                    }
                )?
            )*
        }
    };
}

channels! {
    /// The x position, through a scale.
    x => X;
    /// The y position, through a scale.
    y => Y;
    /// The second x position, through the scale of `x`.
    x2 => X2;
    /// The second y position, through the scale of `y`.
    y2 => Y2;
    /// The longitude of each row in degrees, without a scale.
    longitude => Longitude;
    /// The latitude of each row in degrees, without a scale.
    latitude => Latitude;
    /// The second longitude, without a scale.
    longitude2 => Longitude2;
    /// The second latitude, without a scale.
    latitude2 => Latitude2;
    /// The geometry of each row, from a GeoArrow column, without a scale.
    geometry => Geometry;
    /// The fill of a mark with a fill, and the stroke of a mark without one.
    color, color_values => Color;
    /// The fill.
    fill, fill_values => Fill;
    /// The stroke.
    stroke, stroke_values => Stroke;
    /// A factor of the alpha of the fill and the stroke.
    opacity, opacity_values => Opacity;
    /// A factor of the alpha of the fill.
    fill_opacity, fill_opacity_values => FillOpacity;
    /// A factor of the alpha of the stroke.
    stroke_opacity, stroke_opacity_values => StrokeOpacity;
    /// The width of a stroke.
    stroke_width, stroke_width_values => StrokeWidth;
    /// The dash of a stroke.
    stroke_dash, stroke_dash_values => StrokeDash;
    /// The area of a symbol, the font size of a text, or the width of a trail.
    size, size_values => Size;
    /// The shape of a symbol.
    shape, shape_values => Shape;
    /// The rotation of a symbol, a text, or a path.
    angle, angle_values => Angle;
    /// A pattern over the fill.
    pattern, pattern_values => Pattern;
    /// The text of each row, without a scale.
    text => Text;
    /// The image of each row, without a scale.
    image => Image;
    /// The SVG path of each row, without a scale.
    path => Path;
    /// Splits the rows of a line, a trail, or an area into series, without a scale.
    detail => Detail;
    /// The angle of each row in a polar plot.
    theta, theta_values => Theta;
    /// The second angle, through the scale of `theta`.
    theta2 => Theta2;
    /// The distance of each row from the center of a polar plot.
    radius, radius_values => Radius;
    /// The second radius, through the scale of `radius`.
    radius2 => Radius2;
}

/// A child of a row, a column, or a wrap: a chart, or an empty cell.
pub enum Child {
    Chart(Box<Chart>),
    Empty(Empty),
}

impl From<Chart> for Child {
    fn from(chart: Chart) -> Self {
        Self::Chart(Box::new(chart))
    }
}

impl From<Empty> for Child {
    fn from(empty: Empty) -> Self {
        Self::Empty(empty)
    }
}

/// An empty cell of a row, a column, or a wrap, which keeps its position and draws nothing, such
/// as the cell of a facet without data.
#[must_use]
#[derive(Default)]
pub struct Empty {
    partition: BTreeMap<String, Scalar>,
}

impl Empty {
    /// A partition value of the cell, such as the value of the facet that it stands for.
    pub fn partition(mut self, column: impl Into<String>, value: impl Into<Scalar>) -> Self {
        self.partition.insert(column.into(), value.into());
        self
    }
}

/// A selection of a chart: its name, how its contributions combine, and the contributions that
/// it holds before any interaction.
#[must_use]
#[derive(Clone, Debug)]
pub struct Selection {
    name: String,
    resolution: proto::SelectionResolution,
    initial: Vec<Contribution>,
}

impl Selection {
    /// A selection without initial contributions.
    pub fn new(name: impl Into<String>, resolution: proto::SelectionResolution) -> Self {
        Self {
            name: name.into(),
            resolution,
            initial: Vec::new(),
        }
    }

    /// An initial contribution, after the contributions of the owners before its owner in tree
    /// order.
    pub fn initial(mut self, contribution: Contribution) -> Self {
        self.initial.push(contribution);
        self
    }

    fn encode(&self, values: &mut Vec<ArrayRef>) -> Result<proto::Selection> {
        Ok(proto::Selection {
            name: self.name.clone(),
            resolution: self.resolution as i32,
            initial: self
                .initial
                .iter()
                .map(|contribution| contribution.encode(values))
                .collect::<Result<_>>()?,
        })
    }
}

/// The part of a selection that one plot wrote: alternatives, of which a row matches any, each a
/// tuple of tests, of which a row matches all.
#[must_use]
#[derive(Clone, Debug)]
pub struct Contribution {
    /// The id of the unit or the overlay that wrote it.
    pub owner: String,
    /// The partition values of the owner, its own and those of its ancestors, by column.
    pub partition: BTreeMap<String, Scalar>,
    /// The alternatives, each a tuple of tests.
    pub tuples: Vec<Vec<TupleTest>>,
}

impl Contribution {
    /// A contribution of the unit or the overlay with the id `owner`.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            partition: BTreeMap::new(),
            tuples: Vec::new(),
        }
    }

    /// A partition value of the owner: its own values and those of its ancestors name it, with
    /// its id.
    pub fn partition(mut self, column: impl Into<String>, value: impl Into<Scalar>) -> Self {
        self.partition.insert(column.into(), value.into());
        self
    }

    /// An alternative: a tuple of tests that a row matches when it matches every test.
    pub fn tuple(mut self, tests: impl IntoIterator<Item = TupleTest>) -> Self {
        self.tuples.push(tests.into_iter().collect());
        self
    }

    fn encode(&self, values: &mut Vec<ArrayRef>) -> Result<proto::Contribution> {
        let tuples = self
            .tuples
            .iter()
            .map(|tests| {
                Ok(proto::Tuple {
                    fields: tests
                        .iter()
                        .map(|test| {
                            Ok(proto::TupleField {
                                test: Some(test.encode(values)?),
                            })
                        })
                        .collect::<Result<_>>()?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(proto::Contribution {
            owner: self.owner.clone(),
            tuples,
            partition: partition_values(&self.partition, values),
        })
    }
}

/// The test of one or two columns of a row in a tuple of a contribution.
#[derive(Clone, Debug)]
pub enum TupleTest {
    /// A row matches when its value of `column` is in `values`. `channel` is the channel that
    /// the selection projected through, or `Channel::Unspecified`.
    Set {
        column: String,
        channel: Channel,
        values: ArrayRef,
    },
    /// A row matches when its value of `column` is between the two `ends`, in ascending order.
    Range {
        column: String,
        channel: Channel,
        ends: ArrayRef,
        exclude_end: bool,
    },
    /// A row matches when the point of its values of `x_column` and `y_column` is inside the
    /// polygon of the vertices `x` and `y`, in order around it.
    Polygon {
        x_column: String,
        y_column: String,
        x: ArrayRef,
        y: ArrayRef,
    },
    /// A row matches when its value of `column` is from `start` forward to `end`, in a dimension
    /// whose values wrap after `period`, such as longitudes in degrees with a period of 360. When
    /// `end` is less than `start`, the range crosses the wrap.
    CyclicRange {
        column: String,
        channel: Channel,
        start: f64,
        end: f64,
        period: f64,
    },
}

impl TupleTest {
    fn encode(&self, values: &mut Vec<ArrayRef>) -> Result<proto::tuple_field::Test> {
        Ok(match self {
            Self::Set {
                column,
                channel,
                values: set,
            } => proto::tuple_field::Test::Set(proto::ValueSet {
                column: column.clone(),
                channel: *channel as i32,
                values: push(values, payload::list(set.clone())),
            }),
            Self::Range {
                column,
                channel,
                ends,
                exclude_end,
            } => proto::tuple_field::Test::Range(proto::ValueRange {
                column: column.clone(),
                channel: *channel as i32,
                ends: push(values, payload::list(ends.clone())),
                exclude_end: *exclude_end,
            }),
            Self::Polygon {
                x_column,
                y_column,
                x,
                y,
            } => {
                let vertices = StructArray::try_from(vec![("x", x.clone()), ("y", y.clone())])?;
                proto::tuple_field::Test::Polygon(proto::Polygon {
                    x_column: x_column.clone(),
                    y_column: y_column.clone(),
                    vertices: push(values, payload::list(Arc::new(vertices))),
                })
            }
            Self::CyclicRange {
                column,
                channel,
                start,
                end,
                period,
            } => proto::tuple_field::Test::CyclicRange(proto::CyclicRange {
                column: column.clone(),
                channel: *channel as i32,
                start: *start,
                end: *end,
                period: *period,
            }),
        })
    }
}

/// The builder of an overlay: charts of units and overlays that draw into one plot.
#[must_use]
pub struct OverlayBuilder {
    children: Vec<Chart>,
    coord: Option<proto::Coord>,
    resolve: BTreeMap<Channel, proto::Sharing>,
    interactions: Vec<proto::Interaction>,
    attribution: Option<proto::Attribution>,
    common: Common,
}

impl OverlayBuilder {
    common_methods!();

    /// The coordinate system of the plot.
    pub fn coord(mut self, coord: impl Into<proto::Coord>) -> Self {
        self.coord = Some(coord.into());
        self
    }

    /// One scale of `channel` for every unit below the overlay, which is the default.
    pub fn share(mut self, channel: Channel) -> Self {
        self.resolve.insert(channel, proto::Sharing::Shared);
        self
    }

    /// One scale of `channel` for each child, as for a second axis.
    pub fn independent(mut self, channel: Channel) -> Self {
        self.resolve.insert(channel, proto::Sharing::Independent);
        self
    }

    /// An interaction of the plot, after the earlier ones.
    pub fn interaction(mut self, interaction: proto::Interaction) -> Self {
        self.interactions.push(interaction);
        self
    }

    /// The credits of the plot's sources, before the credits of the children.
    pub fn attribution(mut self, attribution: proto::Attribution) -> Self {
        self.attribution = Some(attribution);
        self
    }

    /// The chart of the overlay. It drops the titles of the children, because the overlay draws
    /// one plot, and moves their credits to its attribution, in order and each credit once.
    pub fn build(self) -> Result<Chart> {
        let mut parts = Parts::default();
        let explicit = partition_values(&self.common.partition, &mut parts.values);
        let children = self.children.into_iter().map(Child::from).collect();
        combine(&mut parts, children, &explicit)?;
        let partition = hoist(&mut parts, None, explicit);
        let mut attribution = self.attribution;
        for node in &mut parts.nodes {
            let (title, credits) = match &mut node.node {
                Some(Node::Unit(unit)) => (&mut unit.title, &mut unit.attribution),
                Some(Node::Overlay(overlay)) => (&mut overlay.title, &mut overlay.attribution),
                _ => continue,
            };
            *title = None;
            for credit in credits
                .take()
                .into_iter()
                .flat_map(|credits| credits.credits)
            {
                let attribution = attribution.get_or_insert_default();
                if !attribution.credits.contains(&credit) {
                    attribution.credits.push(credit);
                }
            }
        }
        let overlay = proto::Overlay {
            id: self.common.id,
            children: std::mem::take(&mut parts.nodes),
            coord: self.coord,
            resolve: resolve(self.resolve),
            title: self.common.title,
            interactions: self.interactions,
            partition,
            attribution,
        };
        finish(Node::Overlay(overlay), parts, self.common.selections)
    }
}

/// The facet of a row, a column, or a wrap, as its builder holds it.
#[derive(Default)]
struct FacetParts {
    column: Option<String>,
    header: Option<proto::facet::Header>,
}

impl FacetParts {
    fn build(self) -> Option<proto::Facet> {
        Some(proto::Facet {
            column: self.column?,
            header: self.header,
        })
    }
}

macro_rules! grid_methods {
    () => {
        /// One scale of `channel` for every unit below the node.
        pub fn share(mut self, channel: Channel) -> Self {
            self.resolve.insert(channel, proto::Sharing::Shared);
            self
        }

        /// One scale of `channel` for each child.
        pub fn independent(mut self, channel: Channel) -> Self {
            self.resolve.insert(channel, proto::Sharing::Independent);
            self
        }

        /// The sharing of the projections of the map plots below the node.
        pub fn projection(mut self, sharing: proto::Sharing) -> Self {
            self.projection = sharing;
            self
        }

        /// The partition column that the children split, so each child declares a value of it,
        /// with a header that labels each child with its value.
        pub fn facet(mut self, column: impl Into<String>) -> Self {
            self.facet.column = Some(column.into());
            self
        }

        /// The options of the header of the facet. Its title is the facet's column when it does
        /// not set one.
        pub fn facet_header(mut self, header: proto::FacetHeader) -> Self {
            self.facet.header = Some(proto::facet::Header::Shown(header));
            self
        }

        /// No header for the facet.
        pub fn hide_facet_header(mut self) -> Self {
            self.facet.header = Some(proto::facet::Header::Hidden(proto::NoHeader {}));
            self
        }
    };
}

/// The builder of a row or a column of charts.
#[must_use]
pub struct ConcatBuilder {
    direction: proto::Direction,
    children: Vec<Child>,
    resolve: BTreeMap<Channel, proto::Sharing>,
    layout: Option<proto::ConcatLayout>,
    projection: proto::Sharing,
    facet: FacetParts,
    common: Common,
}

impl ConcatBuilder {
    common_methods!();
    grid_methods!();

    /// The layout options of the row or the column.
    pub fn layout(mut self, layout: proto::ConcatLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The chart of the row or the column.
    pub fn build(self) -> Result<Chart> {
        let mut parts = Parts::default();
        let explicit = partition_values(&self.common.partition, &mut parts.values);
        combine(&mut parts, self.children, &explicit)?;
        let facet = self.facet.build();
        let partition = hoist(
            &mut parts,
            facet.as_ref().map(|facet| facet.column.as_str()),
            explicit,
        );
        let concat = proto::Concat {
            id: self.common.id,
            direction: self.direction as i32,
            children: std::mem::take(&mut parts.nodes),
            resolve: resolve(self.resolve),
            layout: self.layout,
            title: self.common.title,
            projection: self.projection as i32,
            partition,
            facet,
        };
        finish(Node::Concat(concat), parts, self.common.selections)
    }
}

/// The builder of a wrap: charts in rows of equal columns, as many as fit its width.
#[must_use]
pub struct WrapBuilder {
    children: Vec<Child>,
    resolve: BTreeMap<Channel, proto::Sharing>,
    layout: Option<proto::WrapLayout>,
    projection: proto::Sharing,
    facet: FacetParts,
    common: Common,
}

impl WrapBuilder {
    common_methods!();
    grid_methods!();

    /// The layout options of the wrap.
    pub fn layout(mut self, layout: proto::WrapLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The chart of the wrap.
    pub fn build(self) -> Result<Chart> {
        let mut parts = Parts::default();
        let explicit = partition_values(&self.common.partition, &mut parts.values);
        combine(&mut parts, self.children, &explicit)?;
        let facet = self.facet.build();
        let partition = hoist(
            &mut parts,
            facet.as_ref().map(|facet| facet.column.as_str()),
            explicit,
        );
        let wrap = proto::Wrap {
            id: self.common.id,
            children: std::mem::take(&mut parts.nodes),
            resolve: resolve(self.resolve),
            layout: self.layout,
            title: self.common.title,
            projection: self.projection as i32,
            partition,
            facet,
        };
        finish(Node::Wrap(wrap), parts, self.common.selections)
    }
}

impl Chart {
    /// The builder of a unit of `mark` over `table`.
    pub fn unit(table: RecordBatch, mark: impl Into<mark::Mark>) -> UnitBuilder {
        UnitBuilder {
            table,
            mark: mark.into(),
            encodings: BTreeMap::new(),
            coord: None,
            interactions: Vec::new(),
            attribution: None,
            common: Common::default(),
        }
    }

    /// The builder of an overlay of `children`, charts of units and overlays, which shares every
    /// channel by default.
    pub fn overlay(children: impl IntoIterator<Item = Chart>) -> OverlayBuilder {
        OverlayBuilder {
            children: children.into_iter().collect(),
            coord: None,
            resolve: BTreeMap::new(),
            interactions: Vec::new(),
            attribution: None,
            common: Common::default(),
        }
    }

    /// The builder of a row of `children`.
    pub fn row(children: impl IntoIterator<Item = impl Into<Child>>) -> ConcatBuilder {
        Self::concat(proto::Direction::Row, children)
    }

    /// The builder of a column of `children`.
    pub fn column(children: impl IntoIterator<Item = impl Into<Child>>) -> ConcatBuilder {
        Self::concat(proto::Direction::Column, children)
    }

    fn concat(
        direction: proto::Direction,
        children: impl IntoIterator<Item = impl Into<Child>>,
    ) -> ConcatBuilder {
        ConcatBuilder {
            direction,
            children: children.into_iter().map(Into::into).collect(),
            resolve: BTreeMap::new(),
            layout: None,
            projection: proto::Sharing::Unspecified,
            facet: FacetParts::default(),
            common: Common::default(),
        }
    }

    /// The builder of a wrap of `children`.
    pub fn wrap(children: impl IntoIterator<Item = impl Into<Child>>) -> WrapBuilder {
        WrapBuilder {
            children: children.into_iter().map(Into::into).collect(),
            resolve: BTreeMap::new(),
            layout: None,
            projection: proto::Sharing::Unspecified,
            facet: FacetParts::default(),
            common: Common::default(),
        }
    }

    /// An empty cell of a row, a column, or a wrap.
    pub fn empty() -> Empty {
        Empty::default()
    }
}

/// The nodes, the tables, the values, and the selections of a new chart.
#[derive(Default)]
struct Parts {
    nodes: Vec<proto::Node>,
    tables: Vec<RecordBatch>,
    values: Vec<ArrayRef>,
    selections: BTreeMap<String, proto::Selection>,
}

/// Adds `children` to `parts`, with the tables and the values of each child after those before
/// it and the selections of one name as one. `partition` is the partition that the new node
/// declares, which joins the owner partition of each contribution of the children.
fn combine(
    parts: &mut Parts,
    children: Vec<Child>,
    partition: &[proto::PartitionValue],
) -> Result<()> {
    for child in children {
        match child {
            Child::Chart(chart) => {
                let (mut message, tables, values) = chart.into_parts();
                let value_offset = parts.values.len() as u32;
                tree::for_each_value_mut(&mut message, &mut |index| *index += value_offset);
                let table_offset = parts.tables.len() as u32;
                let mut root = message.root.expect("a valid chart has a root");
                tree::for_each_unit_mut(&mut root, &mut |unit| unit.table += table_offset);
                parts.tables.extend(tables);
                parts.values.extend(values);
                parts.nodes.push(root);
                for mut selection in message.selections {
                    for contribution in &mut selection.initial {
                        contribution.partition.extend_from_slice(partition);
                        contribution
                            .partition
                            .sort_by(|a, b| a.column.cmp(&b.column));
                    }
                    join_selection(&mut parts.selections, selection)?;
                }
            }
            Child::Empty(empty) => {
                let partition = partition_values(&empty.partition, &mut parts.values);
                parts.nodes.push(proto::Node {
                    node: Some(Node::Empty(proto::Empty { partition })),
                })
            }
        }
    }
    Ok(())
}

/// Adds `selection` to `selections`, as one selection with another of its name, whose
/// resolution must be equal, and whose initial contributions come first.
fn join_selection(
    selections: &mut BTreeMap<String, proto::Selection>,
    selection: proto::Selection,
) -> Result<()> {
    match selections.get_mut(&selection.name) {
        Some(existing) => {
            if existing.resolution != selection.resolution {
                return Err(ChartError::invalid(
                    "selections",
                    format!(
                        "the selection `{}` has two resolutions in the combined charts",
                        selection.name
                    ),
                ));
            }
            existing.initial.extend(selection.initial);
        }
        None => {
            selections.insert(selection.name.clone(), selection);
        }
    }
    Ok(())
}

/// The partition of a new node over the nodes of `parts`: `explicit`, with each value that every
/// node declares, other than a value of the facet's column, moved from the nodes.
fn hoist(
    parts: &mut Parts,
    facet: Option<&str>,
    explicit: Vec<proto::PartitionValue>,
) -> Vec<proto::PartitionValue> {
    let values = &parts.values;
    let same = |a: &proto::PartitionValue, b: &proto::PartitionValue| {
        a.column == b.column
            && values[a.value as usize].as_ref() == values[b.value as usize].as_ref()
    };
    let shared: Vec<proto::PartitionValue> = match parts.nodes.split_first() {
        Some((first, rest)) => tree::partition(first)
            .iter()
            .filter(|value| Some(value.column.as_str()) != facet)
            .filter(|value| {
                rest.iter()
                    .all(|node| tree::partition(node).iter().any(|other| same(value, other)))
            })
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    for node in parts.nodes.iter_mut() {
        if let Some(partition) = tree::partition_mut(node) {
            partition.retain(|value| !shared.iter().any(|moved| same(moved, value)));
        }
    }
    let mut partition = explicit;
    for value in shared {
        if !partition.iter().any(|declared| same(declared, &value)) {
            partition.push(value);
        }
    }
    partition.sort_by(|a, b| a.column.cmp(&b.column));
    partition
}

fn resolve(rules: BTreeMap<Channel, proto::Sharing>) -> Vec<proto::Resolve> {
    rules
        .into_iter()
        .map(|(channel, sharing)| proto::Resolve {
            channel: channel as i32,
            sharing: sharing as i32,
        })
        .collect()
}

/// The chart of `root` over the tables and the values of `parts`, with the selections of the
/// children and `selections`, checked.
fn finish(root: Node, mut parts: Parts, selections: Vec<Selection>) -> Result<Chart> {
    let mut joined = std::mem::take(&mut parts.selections);
    for selection in selections {
        join_selection(&mut joined, selection.encode(&mut parts.values)?)?;
    }
    let message = proto::Chart {
        version: FORMAT_VERSION,
        tables: Vec::new(),
        root: Some(proto::Node { node: Some(root) }),
        selections: joined.into_values().collect(),
        values: None,
    };
    Chart::new(message, parts.tables, parts.values)
}

macro_rules! from_variants {
    ($target:ty, $field:ident: $oneof:ty { $($message:ident => $variant:ident),* $(,)? }) => {
        $(
            impl From<proto::$message> for $target {
                fn from(message: proto::$message) -> Self {
                    Self { $field: Some(<$oneof>::$variant(message)) }
                }
            }
        )*
    };
}

macro_rules! mark_from {
    ($($message:ident => $variant:ident),* $(,)?) => {
        $(
            impl From<proto::$message> for mark::Mark {
                fn from(message: proto::$message) -> Self {
                    Self::$variant(message)
                }
            }
        )*
    };
}

mark_from! {
    SymbolMark => Symbol,
    LineMark => Line,
    TrailMark => Trail,
    AreaMark => Area,
    BarMark => Bar,
    RectMark => Rect,
    RuleMark => Rule,
    TextMark => Text,
    ImageMark => Image,
    PathMark => Path,
    GeoshapeMark => Geoshape,
}

from_variants!(proto::Coord, coord: coord::Coord {
    Cartesian => Cartesian,
    Geo => Geo,
    Polar => Polar,
});

from_variants!(proto::Guide, guide: guide::Guide {
    Axis => Axis,
    Legend => Legend,
    Colorbar => Colorbar,
    NoGuide => None,
});

impl From<&str> for proto::Title {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
            ..Default::default()
        }
    }
}
