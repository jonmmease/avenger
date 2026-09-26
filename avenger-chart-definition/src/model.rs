use crate::{dataflow::*, *};
use dataflow::datafusion::common::ScalarValue;
use std::sync::atomic::{AtomicU64, Ordering};

/// Immutable native dataflow and the visual templates bound to it.
#[derive(Clone)]
pub struct ChartDefinition {
    pub(crate) dataflow: Dataflow,
    pub(crate) root: Group,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) background: Option<String>,
}
impl ChartDefinition {
    /// Start direct construction around an already finished dataflow.
    pub fn builder(dataflow: Dataflow) -> ChartBuilder {
        ChartBuilder::new(dataflow)
    }
    /// Read the optional canvas background color.
    pub fn background(&self) -> Option<&str> {
        self.background.as_deref()
    }
    /// Return the native dataflow without preparing it.
    pub fn dataflow(&self) -> &Dataflow {
        &self.dataflow
    }
    /// Read the root composition template.
    pub fn root(&self) -> &Group {
        &self.root
    }
    /// Read the chart's public scalar-input bindings.
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
}

/// A named scalar input and its optional chart-level initial value.
#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub input: ScalarInput,
    pub initial: Option<ScalarValue>,
}

/// An ordered container of visual templates.
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub arrangement: Arrangement,
    pub title: Option<Text>,
    pub children: Vec<Node>,
    pub discovery: Option<ScalarOutput>,
    pub key_order: KeyOrder,
}

/// One plot, a fixed group, or a repeated scope template.
#[derive(Clone, Debug)]
pub enum Node {
    Plot(Plot),
    Group(Group),
    Facet {
        name: String,
        scope: ScopeHandle,
        arrangement: Arrangement,
        template: Group,
    },
}
impl Node {
    /// Return the name within the containing group.
    pub fn name(&self) -> &str {
        match self {
            Self::Plot(p) => &p.name,
            Self::Group(g) => &g.name,
            Self::Facet { name, .. } => name,
        }
    }
}

/// Typed facet-key ordering, independent of result-map iteration order.
#[derive(Clone, Debug, Default)]
pub enum KeyOrder {
    #[default]
    Ascending,
    Descending,
    Explicit(Vec<PartitionKey>),
}

/// Placement of a group's direct children.
#[derive(Clone, Debug)]
pub enum ArrangementKind {
    Row,
    Column,
    Wrap(usize),
    Grid {
        rows: usize,
        columns: usize,
        slots: Vec<(String, GridSlot)>,
    },
}
/// Portable layout settings. Sizes and gaps are logical pixels.
#[derive(Clone, Debug)]
pub struct Arrangement {
    pub kind: ArrangementKind,
    pub gap: f32,
    pub margin: f32,
    pub uniform_columns: bool,
    pub uniform_rows: bool,
    pub share: Option<String>,
    pub columns: Vec<TrackSize>,
    pub rows: Vec<TrackSize>,
}
impl Default for Arrangement {
    fn default() -> Self {
        Self::column()
    }
}
impl Arrangement {
    fn new(kind: ArrangementKind) -> Self {
        Self {
            kind,
            gap: 16.0,
            margin: 0.0,
            uniform_columns: false,
            uniform_rows: false,
            share: None,
            columns: vec![],
            rows: vec![],
        }
    }
    /// Place children horizontally in insertion order.
    pub fn row() -> Self {
        Self::new(ArrangementKind::Row)
    }
    /// Place children vertically in insertion order.
    pub fn column() -> Self {
        Self::new(ArrangementKind::Column)
    }
    /// Wrap children at the specified positive column count.
    pub fn wrap(columns: usize) -> Self {
        Self::new(ArrangementKind::Wrap(columns))
    }
    /// Place named direct children in explicit grid slots.
    pub fn grid(rows: usize, columns: usize, slots: Vec<(String, GridSlot)>) -> Self {
        Self::new(ArrangementKind::Grid {
            rows,
            columns,
            slots,
        })
    }
    /// Set the minimum gap between adjacent children.
    pub fn gap(mut self, value: f32) -> Self {
        self.gap = value;
        self
    }
    /// Set exterior space around this group.
    pub fn margin(mut self, value: f32) -> Self {
        self.margin = value;
        self
    }
    /// Coordinate column widths, including rows with different child counts.
    pub fn uniform_columns(mut self) -> Self {
        self.uniform_columns = true;
        self
    }
    /// Coordinate row heights.
    pub fn uniform_rows(mut self) -> Self {
        self.uniform_rows = true;
        self
    }
    /// Join a layout track-sharing group.
    pub fn share(mut self, key: impl Into<String>) -> Self {
        self.share = Some(key.into());
        self
    }
    /// Set explicit column tracks.
    pub fn columns(mut self, tracks: Vec<TrackSize>) -> Self {
        self.columns = tracks;
        self
    }
    /// Set explicit row tracks.
    pub fn rows(mut self, tracks: Vec<TrackSize>) -> Self {
        self.rows = tracks;
        self
    }
}

/// A literal, local facet key, or scalar output used as a group title.
#[derive(Clone, Debug)]
pub enum Text {
    Literal(String),
    Key(String),
    Scalar(ScalarOutput),
}
impl Text {
    /// Read one field of the current facet key.
    pub fn key(name: impl Into<String>) -> Self {
        Self::Key(name.into())
    }
    /// Format a scalar output as text.
    pub fn scalar(output: &ScalarOutput) -> Self {
        Self::Scalar(*output)
    }
}
impl From<&str> for Text {
    fn from(v: &str) -> Self {
        Self::Literal(v.into())
    }
}
impl From<String> for Text {
    fn from(v: String) -> Self {
        Self::Literal(v)
    }
}

/// A single content rectangle with local scales and mark layers.
#[derive(Clone, Debug)]
pub struct Plot {
    pub(crate) identity: u64,
    pub name: String,
    pub size: Size,
    pub width_step: Option<StepDimension>,
    pub height_step: Option<StepDimension>,
    pub clip: bool,
    pub scales: Vec<(String, Scale)>,
    pub marks: Vec<Mark>,
    pub axes: Vec<Axis>,
    pub guide_reservations: Option<Edges<f32>>,
}
pub(crate) fn plot_identity() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Content size derived from a discrete scale's domain and padding.
#[derive(Clone, Debug)]
pub struct StepDimension {
    pub scale: ScaleHandle,
    pub step: f32,
}

/// A scale local to a particular plot template.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScaleHandle {
    pub(crate) owner: u64,
    pub(crate) name: String,
}
impl ScaleHandle {
    /// Return its local name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Map a column of the mark's bound table.
    pub fn field(&self, field: impl Into<String>) -> Value {
        Value::Scaled(self.clone(), Box::new(Value::Field(field.into())))
    }
    /// Map a numeric constant through this scale.
    pub fn constant(&self, value: f64) -> Value {
        Value::Scaled(self.clone(), Box::new(Value::Constant(value)))
    }
    /// Map a scalar output through this scale.
    pub fn scalar(&self, output: &ScalarOutput) -> Value {
        Value::Scaled(self.clone(), Box::new(Value::Scalar(*output)))
    }
    /// Map a field at a fractional position within its band.
    pub fn band_position(&self, field: impl Into<String>, fraction: f32) -> Value {
        Value::BandPosition(self.clone(), Box::new(Value::Field(field.into())), fraction)
    }
    /// Map zero, clamped to this linear scale's domain.
    pub fn baseline(&self) -> Value {
        Value::Baseline(self.clone())
    }
    /// Read the configured band width in pixels.
    pub fn bandwidth(&self) -> Value {
        Value::Bandwidth(self.clone())
    }
}

/// Source of a scale domain.
#[derive(Clone, Debug)]
pub enum Domain {
    Column(TableOutput, String),
    Extent(ScalarOutput),
    Bounds(ScalarOutput, ScalarOutput),
    Values(Vec<ScalarValue>),
}
impl Domain {
    /// Ordered distinct categories from a published column.
    pub fn column(table: &TableOutput, column: impl Into<String>) -> Self {
        Self::Column(*table, column.into())
    }
    /// Read the nullable min/max struct returned by the extent transform.
    pub fn extent(output: &ScalarOutput) -> Self {
        Self::Extent(*output)
    }
    /// Read the lower and upper bound outputs.
    pub fn bounds(min: &ScalarOutput, max: &ScalarOutput) -> Self {
        Self::Bounds(*min, *max)
    }
    /// Use explicit typed domain values.
    pub fn values(values: impl IntoIterator<Item = ScalarValue>) -> Self {
        Self::Values(values.into_iter().collect())
    }
    /// Use two literal numeric bounds.
    pub fn numeric(min: f64, max: f64) -> Self {
        Self::Values(vec![min.into(), max.into()])
    }
}
/// Mapping from data values to local plot coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleKind {
    Linear,
    Band,
    Point,
}
/// Scale range resolved against each plot's content rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Range {
    PlotWidth,
    PlotHeightReversed,
    /// Content height in top-to-bottom order, for categorical positions.
    PlotHeight,
    Fixed(f32, f32),
}
/// Declarative scale configuration without live kernels or formatter objects.
#[derive(Clone, Debug)]
pub struct Scale {
    pub kind: ScaleKind,
    pub domain: Domain,
    pub range: Range,
    pub zero: bool,
    pub nice: bool,
    pub clamp: bool,
    pub padding_inner: f32,
    pub padding_outer: f32,
    pub include_null: bool,
    /// Logical-pixel domain padding applied before numeric nice rounding.
    pub pixel_padding: f32,
    pub empty_domain: [f64; 2],
    pub sharing: Option<(String, PanelScope)>,
}
impl Scale {
    fn new(kind: ScaleKind, domain: Domain, range: Range) -> Self {
        Self {
            kind,
            domain,
            range,
            zero: false,
            nice: false,
            clamp: false,
            padding_inner: 0.0,
            padding_outer: 0.0,
            include_null: false,
            pixel_padding: 0.0,
            empty_domain: [0.0, 1.0],
            sharing: None,
        }
    }
    /// Define a numeric linear mapping.
    pub fn linear(domain: Domain, range: Range) -> Self {
        Self::new(ScaleKind::Linear, domain, range)
    }
    /// Define ordered categorical bands.
    pub fn band(domain: Domain, range: Range) -> Self {
        Self::new(ScaleKind::Band, domain, range)
    }
    /// Define evenly spaced categorical positions.
    pub fn point(domain: Domain, range: Range) -> Self {
        Self::new(ScaleKind::Point, domain, range)
    }
    /// Retain null as a distinct categorical value.
    pub fn include_null(mut self, enabled: bool) -> Self {
        self.include_null = enabled;
        self
    }
    /// Expand a numeric domain to leave this many logical pixels at each end.
    pub fn pixel_padding(mut self, pixels: f32) -> Self {
        self.pixel_padding = pixels;
        self
    }
    /// Include zero in a numeric domain.
    pub fn zero(mut self, value: bool) -> Self {
        self.zero = value;
        self
    }
    /// Expand a numeric domain to rounded tick boundaries.
    pub fn nice(mut self, value: bool) -> Self {
        self.nice = value;
        self
    }
    /// Clamp values to the domain interval.
    pub fn clamp(mut self, value: bool) -> Self {
        self.clamp = value;
        self
    }
    /// Set the fraction of each band reserved between bands.
    pub fn padding_inner(mut self, value: f32) -> Self {
        self.padding_inner = value;
        self
    }
    /// Set exterior band padding.
    pub fn padding_outer(mut self, value: f32) -> Self {
        self.padding_outer = value;
        self
    }
    /// Supply bounds when every contributing extent is absent.
    pub fn empty_domain(mut self, min: f64, max: f64) -> Self {
        self.empty_domain = [min, max];
        self
    }
    /// Combine compatible domain contributions within a panel scope.
    pub fn share_domain(mut self, name: impl Into<String>, scope: PanelScope) -> Self {
        self.sharing = Some((name.into(), scope));
        self
    }
}

/// Small declarative property binding evaluated against a mark's rows.
#[derive(Clone, Debug)]
pub enum Value {
    Constant(f64),
    Field(String),
    Scalar(ScalarOutput),
    Scaled(ScaleHandle, Box<Value>),
    Bandwidth(ScaleHandle),
    BandPosition(ScaleHandle, Box<Value>, f32),
    Baseline(ScaleHandle),
    PlotWidth,
    PlotHeight,
}
impl Value {
    /// Use an unscaled numeric column.
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field(name.into())
    }
    /// Broadcast an unscaled numeric scalar output.
    pub fn scalar(output: &ScalarOutput) -> Self {
        Self::Scalar(*output)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Constant(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Self::Constant(v as f64)
    }
}

/// Pixel adjustments applied after scaling and ordering a rectangle's endpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpanAdjustment {
    pub spacing: f32,
    pub minimum: f32,
    pub offset: f32,
}

/// Rectangle position, size, and constant styling bindings.
#[derive(Clone, Debug)]
pub struct RectEncoding {
    pub x: Option<Value>,
    pub y: Option<Value>,
    pub x2: Option<Value>,
    pub y2: Option<Value>,
    pub xc: Option<Value>,
    pub yc: Option<Value>,
    pub x_span: SpanAdjustment,
    pub y_span: SpanAdjustment,
    pub width: Option<Value>,
    pub height: Option<Value>,
    pub fill: String,
    pub interactive: bool,
}
impl Default for RectEncoding {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            x2: None,
            y2: None,
            xc: None,
            yc: None,
            x_span: SpanAdjustment::default(),
            y_span: SpanAdjustment::default(),
            width: None,
            height: None,
            fill: "#4c78a8".into(),
            interactive: false,
        }
    }
}
macro_rules! property {
    ($name:ident) => {
        #[doc = concat!("Bind the `", stringify!($name), "` property.")]
        pub fn $name(mut self, value: impl Into<Value>) -> Self {
            self.$name = Some(value.into());
            self
        }
    };
}
impl RectEncoding {
    /// Start a rectangle encoding with no geometry bindings.
    pub fn new() -> Self {
        Self::default()
    }
    property!(x);
    property!(y);
    property!(x2);
    property!(y2);
    property!(xc);
    property!(yc);
    property!(width);
    property!(height);
    /// Set a CSS fill color.
    pub fn fill(mut self, color: impl Into<String>) -> Self {
        self.fill = color.into();
        self
    }
    /// Adjust the rectangle's horizontal pixel interval.
    pub fn x_span(mut self, span: SpanAdjustment) -> Self {
        self.x_span = span;
        self
    }
    /// Adjust the rectangle's vertical pixel interval.
    pub fn y_span(mut self, span: SpanAdjustment) -> Self {
        self.y_span = span;
        self
    }
    /// Include the layer in app hit testing.
    pub fn interactive(mut self, enabled: bool) -> Self {
        self.interactive = enabled;
        self
    }
}
/// Symbol positions and size, with constant shape and fill.
#[derive(Clone, Debug)]
pub struct SymbolEncoding {
    pub x: Option<Value>,
    pub y: Option<Value>,
    pub size: Value,
    pub fill: String,
    pub interactive: bool,
}
impl Default for SymbolEncoding {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            size: Value::Constant(20.0),
            fill: "#4c78a8".into(),
            interactive: false,
        }
    }
}
impl SymbolEncoding {
    /// Start a circle encoding.
    pub fn new() -> Self {
        Self::default()
    }
    property!(x);
    property!(y);
    /// Bind scenegraph symbol size, which remains constant in screen space during pan/zoom.
    pub fn size(mut self, value: impl Into<Value>) -> Self {
        self.size = value.into();
        self
    }
    /// Set a CSS fill color.
    pub fn fill(mut self, color: impl Into<String>) -> Self {
        self.fill = color.into();
        self
    }
    /// Include the layer in app hit testing.
    pub fn interactive(mut self, enabled: bool) -> Self {
        self.interactive = enabled;
        self
    }
}
/// One named mark layer bound to a published table.
#[derive(Clone, Debug)]
pub struct Mark {
    pub name: String,
    pub table: TableOutput,
    pub encoding: Encoding,
}
/// Supported mark descriptor variants.
#[derive(Clone, Debug)]
pub enum Encoding {
    Rect(Box<RectEncoding>),
    Symbol(SymbolEncoding),
}

/// A Cartesian axis referencing a local plot scale.
#[derive(Clone, Debug)]
pub struct Axis {
    pub scale: ScaleHandle,
    pub side: Side,
    pub title: String,
    pub format: Option<String>,
    pub tick_count: f32,
    pub label_angle: Option<f32>,
    pub grid: bool,
    pub labels: LabelVisibility,
    pub sharing: PanelScope,
    pub shared_title: bool,
}
impl Axis {
    fn new(scale: &ScaleHandle, side: Side) -> Self {
        Self {
            scale: scale.clone(),
            side,
            title: String::new(),
            format: None,
            tick_count: 5.0,
            label_angle: None,
            grid: false,
            labels: LabelVisibility::All,
            sharing: PanelScope::Root,
            shared_title: false,
        }
    }
    /// Place an axis below the plot.
    pub fn bottom(scale: &ScaleHandle) -> Self {
        Self::new(scale, Side::Bottom)
    }
    /// Place an axis above the plot.
    pub fn top(scale: &ScaleHandle) -> Self {
        Self::new(scale, Side::Top)
    }
    /// Place an axis to the left of the plot.
    pub fn left(scale: &ScaleHandle) -> Self {
        Self::new(scale, Side::Left)
    }
    /// Place an axis to the right of the plot.
    pub fn right(scale: &ScaleHandle) -> Self {
        Self::new(scale, Side::Right)
    }
    /// Set the axis title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
    /// Set a numeric tick-label format.
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }
    /// Set the approximate tick count.
    pub fn tick_count(mut self, count: f32) -> Self {
        self.tick_count = count;
        self
    }
    /// Rotate tick labels in degrees.
    pub fn label_angle(mut self, angle: f32) -> Self {
        self.label_angle = Some(angle);
        self
    }
    /// Draw grid lines across the plot.
    pub fn grid(mut self, enabled: bool) -> Self {
        self.grid = enabled;
        self
    }
    /// Set label visibility using the panel guide planner.
    pub fn labels(mut self, labels: LabelVisibility) -> Self {
        self.labels = labels;
        self
    }
    /// Resolve guide sharing within a logical panel scope.
    pub fn share_within(mut self, scope: PanelScope) -> Self {
        self.sharing = scope;
        self
    }
    /// Place one title per compatible sharing group.
    pub fn shared_title(mut self, enabled: bool) -> Self {
        self.shared_title = enabled;
        self
    }
}
