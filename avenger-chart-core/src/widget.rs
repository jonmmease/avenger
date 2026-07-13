//! Shared authoring and serialized contracts for chart widgets.

use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD};
use datafusion::{
    arrow::datatypes::{DataType, Field},
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::expr::Placeholder,
    prelude::Expr,
};
use datafusion_proto::protobuf::LogicalExprNode;
use datafusion_proto_common::protobuf_common::ScalarValue as ProtoScalarValue;
use indexmap::IndexMap;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CompiledDataContext, CompiledMark, CompiledParamSpec,
    DefaultLogicalExprNodeExt, IntoExpr, LegendPosition, PixelFrame, ThemeValue, ToolExpansion,
    serialization::{SerializableExpr, SerializableScalar},
};

pub type ChromePosition = LegendPosition;

#[derive(Clone, Debug)]
pub struct WidgetExpansionContext<'a> {
    pub widget_id: &'a str,
}

impl<'a> WidgetExpansionContext<'a> {
    pub const fn new(widget_id: &'a str) -> Self {
        Self { widget_id }
    }

    /// Reference one resolved part style from a widget mark expression.
    pub fn part_style(&self, part: &str, property: WidgetStyleProperty) -> Expr {
        widget_runtime_placeholder(
            &widget_style_input_name(Some(part), property),
            widget_style_input_data_type(property),
        )
    }

    /// Reference one resolved host style from a widget mark expression.
    pub fn host_style(&self, property: WidgetStyleProperty) -> Expr {
        widget_runtime_placeholder(
            &widget_style_input_name(None, property),
            widget_style_input_data_type(property),
        )
    }

    /// Reference the realized widget frame width from a mark expression.
    pub fn frame_width(&self) -> Expr {
        widget_runtime_placeholder(WIDGET_FRAME_WIDTH_INPUT, Some(DataType::Float32))
    }

    /// Reference the realized widget frame height from a mark expression.
    pub fn frame_height(&self) -> Expr {
        widget_runtime_placeholder(WIDGET_FRAME_HEIGHT_INPUT, Some(DataType::Float32))
    }
}

pub const WIDGET_RUNTIME_INPUT_PREFIX: &str = "__widget_";
pub const WIDGET_FRAME_WIDTH_INPUT: &str = "__widget_frame_width";
pub const WIDGET_FRAME_HEIGHT_INPUT: &str = "__widget_frame_height";

fn widget_runtime_placeholder(name: &str, data_type: Option<DataType>) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        format!("${name}"),
        data_type.map(|data_type| Arc::new(Field::new("", data_type, false))),
    ))
}

fn widget_style_input_data_type(property: WidgetStyleProperty) -> Option<DataType> {
    match property.value_type() {
        WidgetStyleValueType::Number | WidgetStyleValueType::Length => Some(DataType::Float32),
        WidgetStyleValueType::Color
        | WidgetStyleValueType::String
        | WidgetStyleValueType::Cursor => Some(DataType::Utf8),
        // Font weights intentionally accept either numeric or keyword values.
        WidgetStyleValueType::FontWeight => None,
    }
}

pub fn widget_style_input_name(part: Option<&str>, property: WidgetStyleProperty) -> String {
    let part = part.unwrap_or("host").replace('-', "_");
    let property = property.name().replace('-', "_");
    format!("__widget_style_{part}_{property}")
}

pub struct WidgetExpansion {
    pub expansion: ToolExpansion<PixelFrame>,
    pub items: Option<WidgetItems>,
    pub measure: WidgetMeasureSpec,
    pub presentation: WidgetPresentationBindings,
}

#[derive(Clone, Debug, Default)]
pub struct WidgetPresentationBindings {
    pub variant: Option<String>,
    pub disabled: Option<Expr>,
    pub checked: Option<Expr>,
    pub selected: Option<Expr>,
    pub orientation: Option<String>,
}

impl WidgetPresentationBindings {
    pub fn variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    pub fn disabled(mut self, disabled: impl IntoExpr) -> Self {
        self.disabled = Some(disabled.into_expr());
        self
    }

    pub fn checked(mut self, checked: impl IntoExpr) -> Self {
        self.checked = Some(checked.into_expr());
        self
    }

    pub fn selected(mut self, selected: impl IntoExpr) -> Self {
        self.selected = Some(selected.into_expr());
        self
    }

    pub fn orientation(mut self, orientation: impl Into<String>) -> Self {
        self.orientation = Some(orientation.into());
        self
    }

    pub fn compile(&self) -> Result<CompiledWidgetPresentationSpec, AvengerChartError> {
        let compile_expr = |expr: &Expr| LogicalExprNode::from_default_expr(expr.clone());
        Ok(CompiledWidgetPresentationSpec {
            variant: self.variant.clone(),
            disabled: self.disabled.as_ref().map(compile_expr).transpose()?,
            checked: self.checked.as_ref().map(compile_expr).transpose()?,
            selected: self.selected.as_ref().map(compile_expr).transpose()?,
            orientation: self.orientation.clone(),
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompiledWidgetPresentationSpec {
    pub variant: Option<String>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub disabled: Option<LogicalExprNode>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub checked: Option<LogicalExprNode>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub selected: Option<LogicalExprNode>,
    pub orientation: Option<String>,
}

pub trait ChartWidget: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn kind(&self) -> &'static str;
    fn expand(&self, ctx: WidgetExpansionContext<'_>)
    -> Result<WidgetExpansion, AvengerChartError>;
}

#[derive(Clone, Debug)]
pub struct WidgetItemRow {
    pub values: IndexMap<String, ScalarValue>,
}

impl WidgetItemRow {
    pub fn new(values: impl IntoIterator<Item = (String, ScalarValue)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

#[derive(Clone)]
// `DataFrame` is intentionally held directly; boxing would alter this public builder input.
#[allow(clippy::large_enum_variant)]
pub enum WidgetItems {
    Static(Vec<WidgetItemRow>),
    DataFrame {
        data: DataFrame,
        order_key: Vec<Expr>,
    },
    Configured {
        source: Box<WidgetItems>,
        value: Option<Expr>,
        label: Option<Expr>,
        identity: Option<WidgetItemIdentityDerivation>,
        validations: Vec<WidgetItemValidation>,
    },
}

/// A serialized, engine-neutral instruction for deriving stable item ids once
/// a widget's canonical item relation has materialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetItemIdentityDerivation {
    pub source_column: String,
    pub output_column: String,
}

/// Type-preserving item identities shared by list widgets and semantic
/// selection updates.
pub struct WidgetItemIdentityCodec;

impl WidgetItemIdentityCodec {
    /// Encode a scalar using DataFusion's protobuf interchange representation.
    /// The versioned prefix reserves room for a future codec migration without
    /// conflating identities produced by two formats.
    pub fn encode(value: &ScalarValue) -> Result<String, AvengerChartError> {
        let proto = ProtoScalarValue::try_from(value).map_err(|error| {
            AvengerChartError::SerializationError(format!(
                "Failed to encode widget item identity: {error}"
            ))
        })?;
        Ok(format!(
            "wii1_{}",
            BASE64_URL_SAFE_NO_PAD.encode(proto.encode_to_vec())
        ))
    }
}

impl WidgetItems {
    /// Project author-facing item expressions to the canonical columns used
    /// by data-encoded widget marks, measurement, validation, and event data.
    pub fn project(self, value: impl IntoExpr, label: impl IntoExpr) -> Self {
        match self {
            Self::Configured {
                source,
                identity,
                validations,
                ..
            } => Self::Configured {
                source,
                value: Some(value.into_expr()),
                label: Some(label.into_expr()),
                identity,
                validations,
            },
            source => Self::Configured {
                source: Box::new(source),
                value: Some(value.into_expr()),
                label: Some(label.into_expr()),
                identity: None,
                validations: Vec::new(),
            },
        }
    }

    /// Derive a stable string identity from one canonical item column after
    /// the relation's single materialization.
    pub fn derive_identity(
        self,
        source_column: impl Into<String>,
        output_column: impl Into<String>,
    ) -> Self {
        let identity = WidgetItemIdentityDerivation {
            source_column: source_column.into(),
            output_column: output_column.into(),
        };
        match self {
            Self::Configured {
                source,
                value,
                label,
                validations,
                ..
            } => Self::Configured {
                source,
                value,
                label,
                identity: Some(identity),
                validations,
            },
            source => Self::Configured {
                source: Box::new(source),
                value: None,
                label: None,
                identity: Some(identity),
                validations: Vec::new(),
            },
        }
    }

    /// Attach a serialized validation to the prepared item relation.
    pub fn validate(self, validation: WidgetItemValidation) -> Self {
        match self {
            Self::Configured {
                source,
                value,
                label,
                identity,
                mut validations,
            } => {
                validations.push(validation);
                Self::Configured {
                    source,
                    value,
                    label,
                    identity,
                    validations,
                }
            }
            source => Self::Configured {
                source: Box::new(source),
                value: None,
                label: None,
                identity: None,
                validations: vec![validation],
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetMeasureSpec {
    pub width: WidgetAxisMeasureSpec,
    pub height: WidgetAxisMeasureSpec,
}

impl WidgetMeasureSpec {
    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            width: WidgetAxisMeasureSpec::Fixed { px: width },
            height: WidgetAxisMeasureSpec::Fixed { px: height },
        }
    }

    /// Conservative seed frame used for style-dependent mark preparation
    /// before intrinsic measurement and the unified chrome solve. Content and
    /// fill axes use their declared minimum; the realized frame replaces this
    /// seed for final rendering.
    #[doc(hidden)]
    pub fn provisional_frame_size(&self) -> (f32, f32) {
        fn axis(spec: &WidgetAxisMeasureSpec) -> f32 {
            match spec {
                WidgetAxisMeasureSpec::Fixed { px } => *px,
                WidgetAxisMeasureSpec::Content { min_px, .. }
                | WidgetAxisMeasureSpec::Fill { min_px, .. } => *min_px,
            }
        }
        (axis(&self.width), axis(&self.height))
    }
}

impl Default for WidgetMeasureSpec {
    fn default() -> Self {
        Self::fixed(0.0, 0.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WidgetAxisMeasureSpec {
    Fixed {
        px: f32,
    },
    Content {
        expr: WidgetMeasureExpr,
        min_px: f32,
        max_px: Option<f32>,
    },
    Fill {
        expr: WidgetMeasureExpr,
        min_px: f32,
        max_px: Option<f32>,
        stretch: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WidgetTextMeasureAxis {
    Width,
    Height,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WidgetMeasureExpr {
    Px(f32),
    StyleLength {
        part: Option<String>,
        property: WidgetStyleProperty,
    },
    TextExtent {
        part: String,
        axis: WidgetTextMeasureAxis,
        data_encoded: bool,
    },
    ItemCount {
        extent: Box<WidgetMeasureExpr>,
        gap: Box<WidgetMeasureExpr>,
    },
    Add(Vec<WidgetMeasureExpr>),
    Max(Vec<WidgetMeasureExpr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedWidgetAxisSize {
    pub min_px: f32,
    pub preferred_px: f32,
    pub stretch: f32,
}

/// Evaluates a widget's symbolic measurement using one already-resolved style
/// snapshot. The callback is the sole text-measurement seam; callers can route
/// it through their durable text cache without allowing measurement to query
/// CSS independently.
pub fn resolve_widget_measure_spec<F>(
    widget_id: &str,
    spec: &WidgetMeasureSpec,
    styles: &ResolvedWidgetStyleSet,
    item_count: usize,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    mut text_extent: F,
) -> Result<(ResolvedWidgetAxisSize, ResolvedWidgetAxisSize), AvengerChartError>
where
    F: FnMut(&str, WidgetTextMeasureAxis, bool) -> Result<f32, AvengerChartError>,
{
    let mut evaluate = |axis: &WidgetAxisMeasureSpec| {
        resolve_widget_axis_measure(
            widget_id,
            axis,
            styles,
            item_count,
            params,
            base_font_size,
            &mut text_extent,
        )
    };
    Ok((evaluate(&spec.width)?, evaluate(&spec.height)?))
}

fn resolve_widget_axis_measure<F>(
    widget_id: &str,
    spec: &WidgetAxisMeasureSpec,
    styles: &ResolvedWidgetStyleSet,
    item_count: usize,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    text_extent: &mut F,
) -> Result<ResolvedWidgetAxisSize, AvengerChartError>
where
    F: FnMut(&str, WidgetTextMeasureAxis, bool) -> Result<f32, AvengerChartError>,
{
    let (min_px, preferred_px, max_px, stretch) = match spec {
        WidgetAxisMeasureSpec::Fixed { px } => (*px, *px, Some(*px), 0.0),
        WidgetAxisMeasureSpec::Content {
            expr,
            min_px,
            max_px,
        } => (
            *min_px,
            evaluate_widget_measure_expr(
                widget_id,
                expr,
                styles,
                item_count,
                params,
                base_font_size,
                text_extent,
            )?,
            *max_px,
            0.0,
        ),
        WidgetAxisMeasureSpec::Fill {
            expr,
            min_px,
            max_px,
            stretch,
        } => (
            *min_px,
            evaluate_widget_measure_expr(
                widget_id,
                expr,
                styles,
                item_count,
                params,
                base_font_size,
                text_extent,
            )?,
            *max_px,
            *stretch,
        ),
    };
    for (name, value) in [
        ("minimum", min_px),
        ("preferred", preferred_px),
        ("stretch", stretch),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(invalid_widget_measure(widget_id, name, value));
        }
    }
    if let Some(max_px) = max_px
        && (!max_px.is_finite() || max_px < min_px)
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Widget '{widget_id}' measurement maximum {max_px} must be finite and at least its minimum {min_px}"
        )));
    }
    Ok(ResolvedWidgetAxisSize {
        min_px,
        preferred_px: preferred_px
            .max(min_px)
            .min(max_px.unwrap_or(f32::INFINITY)),
        stretch,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_widget_measure_expr<F>(
    widget_id: &str,
    expr: &WidgetMeasureExpr,
    styles: &ResolvedWidgetStyleSet,
    item_count: usize,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    text_extent: &mut F,
) -> Result<f32, AvengerChartError>
where
    F: FnMut(&str, WidgetTextMeasureAxis, bool) -> Result<f32, AvengerChartError>,
{
    let value = match expr {
        WidgetMeasureExpr::Px(px) => *px,
        WidgetMeasureExpr::StyleLength { part, property } => {
            let style = if let Some(part) = part {
                styles.parts.get(part).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' measurement references unknown part '{part}'"
                    ))
                })?
            } else {
                &styles.host
            };
            let value = style.values.get(property).ok_or_else(|| {
                AvengerChartError::InvalidWidgetStyle {
                    widget_id: widget_id.to_string(),
                    property: property.name().to_string(),
                    message: "required by measurement but not resolved".to_string(),
                }
            })?;
            value
                .eval_as_length(&crate::theme::eval::EvalContext::new(
                    params,
                    base_font_size,
                ))
                .map_err(|error| AvengerChartError::InvalidWidgetStyle {
                    widget_id: widget_id.to_string(),
                    property: property.name().to_string(),
                    message: error.to_string(),
                })? as f32
        }
        WidgetMeasureExpr::TextExtent {
            part,
            axis,
            data_encoded,
        } => text_extent(part, *axis, *data_encoded)?,
        WidgetMeasureExpr::ItemCount { extent, gap } => {
            if item_count == 0 {
                0.0
            } else {
                let extent = evaluate_widget_measure_expr(
                    widget_id,
                    extent,
                    styles,
                    item_count,
                    params,
                    base_font_size,
                    text_extent,
                )?;
                let gap = evaluate_widget_measure_expr(
                    widget_id,
                    gap,
                    styles,
                    item_count,
                    params,
                    base_font_size,
                    text_extent,
                )?;
                extent * item_count as f32 + gap * item_count.saturating_sub(1) as f32
            }
        }
        WidgetMeasureExpr::Add(values) => {
            let mut total = 0.0;
            for value in values {
                total += evaluate_widget_measure_expr(
                    widget_id,
                    value,
                    styles,
                    item_count,
                    params,
                    base_font_size,
                    text_extent,
                )?;
            }
            total
        }
        WidgetMeasureExpr::Max(values) => {
            let mut maximum = 0.0_f32;
            for value in values {
                maximum = maximum.max(evaluate_widget_measure_expr(
                    widget_id,
                    value,
                    styles,
                    item_count,
                    params,
                    base_font_size,
                    text_extent,
                )?);
            }
            maximum
        }
    };
    if !value.is_finite() || value < 0.0 {
        return Err(invalid_widget_measure(widget_id, "expression", value));
    }
    Ok(value)
}

fn invalid_widget_measure(widget_id: &str, role: &str, value: f32) -> AvengerChartError {
    AvengerChartError::InvalidArgument(format!(
        "Widget '{widget_id}' measurement {role} resolved to invalid value {value}"
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetStyleValueType {
    Color,
    Number,
    String,
    FontWeight,
    Length,
    Cursor,
}

macro_rules! widget_style_properties {
    ($(($variant:ident, $name:literal, $ty:ident)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum WidgetStyleProperty { $($variant),+ }

        impl WidgetStyleProperty {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
            pub const fn value_type(self) -> WidgetStyleValueType {
                match self { $(Self::$variant => WidgetStyleValueType::$ty),+ }
            }
            pub const fn applies_to_host(self) -> bool {
                matches!(self, Self::Width | Self::Height | Self::MinWidth | Self::MinHeight
                    | Self::PaddingInline | Self::PaddingBlock | Self::ItemGap)
            }

            pub fn for_mark_channel(channel: &str) -> Option<Self> {
                match channel {
                    // Text marks expose author-facing `color`/`font` channels,
                    // while widget CSS follows the stable label-part contract.
                    "color" => return Some(Self::Fill),
                    "font" => return Some(Self::FontFamily),
                    _ => {}
                }
                let css_name = channel.replace('_', "-");
                Self::ALL
                    .iter()
                    .copied()
                    .find(|property| property.name() == css_name)
            }
        }
    };
}

widget_style_properties!(
    (Fill, "fill", Color),
    (Stroke, "stroke", Color),
    (Opacity, "opacity", Number),
    (FontFamily, "font-family", String),
    (FontSize, "font-size", Length),
    (FontWeight, "font-weight", FontWeight),
    (StrokeWidth, "stroke-width", Length),
    (FocusRingWidth, "focus-ring-width", Length),
    (CornerRadius, "corner-radius", Length),
    (Width, "width", Length),
    (Height, "height", Length),
    (MinWidth, "min-width", Length),
    (MinHeight, "min-height", Length),
    (PaddingInline, "padding-inline", Length),
    (PaddingBlock, "padding-block", Length),
    (ControlLabelGap, "control-label-gap", Length),
    (VisualLabelGap, "visual-label-gap", Length),
    (ItemGap, "item-gap", Length),
    (FocusGap, "focus-gap", Length),
    (ControlHeight, "control-height", Length),
    (BorderWidth, "border-width", Length),
    (ButtonMinWidth, "button-min-width", Length),
    (ButtonInlinePadding, "button-inline-padding", Length),
    (ButtonLineHeight, "button-line-height", Length),
    (ButtonRadius, "button-radius", Length),
    (ButtonBorderWidth, "button-border-width", Length),
    (ChoiceControlSize, "choice-control-size", Length),
    (
        RadioSelectedBorderWidth,
        "radio-selected-border-width",
        Length
    ),
    (RadioCenterSize, "radio-center-size", Length),
    (SliderMinWidth, "slider-min-width", Length),
    (SliderTrackHeight, "slider-track-height", Length),
    (SliderHandleSize, "slider-handle-size", Length),
    (
        SliderHandleBorderWidth,
        "slider-handle-border-width",
        Length
    ),
    (
        SliderHandlePressedBorderWidth,
        "slider-handle-pressed-border-width",
        Length
    ),
    (SliderValuePadding, "slider-value-padding", Length),
    (InputInlineInset, "input-inline-inset", Length),
    (InputCaretWidth, "input-caret-width", Length),
    (InputPlaceholderColor, "input-placeholder-color", Color),
    (InputSelectionOpacity, "input-selection-opacity", Number),
    (Cursor, "cursor", Cursor),
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetPartManifest {
    pub name: String,
    pub scene_mark_kind: String,
    pub style_properties: Vec<WidgetStyleProperty>,
    pub states: Vec<String>,
    pub interactive: bool,
}

/// Returns whether a conventional widget part is visual-only and must not be
/// used as an event target.
///
/// Built-in widgets use these names consistently so omitted event targets can
/// expand to the complete interactive surface without making focus and text
/// editing decorations independently clickable.
pub fn is_decorative_widget_part(name: &str) -> bool {
    matches!(
        name,
        "focus-ring" | "selection" | "selected-box" | "selected-control" | "caret" | "preedit"
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetThemeProvenance {
    pub widget_kind: String,
    pub widget_id: String,
    pub part: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetPresentationState {
    pub variant: Option<String>,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub orientation: Option<String>,
    pub focus_visible: bool,
    pub hover: bool,
    pub pressed: bool,
}

impl WidgetPresentationState {
    pub fn apply_to_host(&self, mut host: crate::ThemeContext) -> crate::ThemeContext {
        if let Some(variant) = &self.variant {
            host = host.with_attribute("variant", variant);
        }
        host = host
            .with_attribute("disabled", self.disabled.to_string())
            .with_attribute("focus-visible", self.focus_visible.to_string())
            .with_attribute("hover", self.hover.to_string())
            .with_attribute("pressed", self.pressed.to_string());
        if let Some(checked) = self.checked {
            host = host.with_attribute("checked", checked.to_string());
        }
        if let Some(selected) = self.selected {
            host = host.with_attribute("selected", selected.to_string());
        }
        if let Some(orientation) = &self.orientation {
            host = host.with_attribute("orientation", orientation);
        }
        host
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedWidgetPartStyle {
    pub values: IndexMap<WidgetStyleProperty, ThemeValue>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedWidgetStyleSet {
    pub host: ResolvedWidgetPartStyle,
    pub parts: IndexMap<String, ResolvedWidgetPartStyle>,
    pub digest: u64,
}

/// Lower one resolved style snapshot to internal mark-evaluation inputs.
/// These values are evaluation-local and never become document params.
pub fn widget_style_evaluation_inputs(
    theme: &crate::Theme,
    widget_id: &str,
    styles: &ResolvedWidgetStyleSet,
    document_params: &IndexMap<String, ScalarValue>,
) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
    let eval = crate::theme::eval::EvalContext::new(
        document_params,
        theme.get_base_font_size(document_params),
    );
    let mut inputs = IndexMap::new();
    for (property, value) in &styles.host.values {
        inputs.insert(
            widget_style_input_name(None, *property),
            widget_style_scalar(widget_id, *property, value, &eval)?,
        );
    }
    for (part, style) in &styles.parts {
        for (property, value) in &style.values {
            inputs.insert(
                widget_style_input_name(Some(part), *property),
                widget_style_scalar(widget_id, *property, value, &eval)?,
            );
        }
    }
    Ok(inputs)
}

fn widget_style_scalar(
    widget_id: &str,
    property: WidgetStyleProperty,
    value: &ThemeValue,
    eval: &crate::theme::eval::EvalContext<'_>,
) -> Result<ScalarValue, AvengerChartError> {
    let invalid = |message: String| AvengerChartError::InvalidWidgetStyle {
        widget_id: widget_id.to_string(),
        property: property.name().to_string(),
        message,
    };
    Ok(match property.value_type() {
        WidgetStyleValueType::Color => {
            let rgba = value
                .eval_as_color(eval)
                .map_err(|error| invalid(error.to_string()))?;
            let text = if rgba.alpha == 255 {
                format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue)
            } else {
                format!(
                    "rgba({}, {}, {}, {})",
                    rgba.red,
                    rgba.green,
                    rgba.blue,
                    rgba.alpha as f32 / 255.0
                )
            };
            ScalarValue::Utf8(Some(text))
        }
        WidgetStyleValueType::Number => ScalarValue::Float32(Some(
            value
                .eval_as_number(eval)
                .map_err(|error| invalid(error.to_string()))? as f32,
        )),
        WidgetStyleValueType::Length => ScalarValue::Float32(Some(
            value
                .eval_as_length(eval)
                .map_err(|error| invalid(error.to_string()))? as f32,
        )),
        WidgetStyleValueType::String | WidgetStyleValueType::Cursor => ScalarValue::Utf8(Some(
            value
                .eval_as_string(eval)
                .map_err(|error| invalid(error.to_string()))?,
        )),
        WidgetStyleValueType::FontWeight => {
            if let Ok(number) = value.eval_as_number(eval) {
                ScalarValue::Float32(Some(number as f32))
            } else {
                ScalarValue::Utf8(Some(
                    value
                        .eval_as_string(eval)
                        .map_err(|error| invalid(error.to_string()))?,
                ))
            }
        }
    })
}

pub fn resolve_widget_style_set(
    theme: &crate::Theme,
    widget_kind: &str,
    widget_id: &str,
    parts: &[WidgetPartManifest],
    presentation: &WidgetPresentationState,
    params: &IndexMap<String, ScalarValue>,
) -> Result<ResolvedWidgetStyleSet, AvengerChartError> {
    let host = presentation
        .apply_to_host(crate::ThemeContext::new(widget_kind, params.clone()).with_id(widget_id));
    let mut resolved = ResolvedWidgetStyleSet::default();
    for property in WidgetStyleProperty::ALL
        .iter()
        .copied()
        .filter(|property| property.applies_to_host())
    {
        if let Some(value) = theme.query(&host, property.name()) {
            validate_widget_style_value(theme, widget_id, property, &value, params)?;
            resolved.host.values.insert(property, value);
        }
    }
    for part in parts {
        let context = crate::ThemeContext::new("mark", params.clone())
            .with_subtype(&part.scene_mark_kind)
            .with_part(&part.name, host.clone());
        let mut style = ResolvedWidgetPartStyle::default();
        for property in &part.style_properties {
            if let Some(value) = theme.query_widget_part(&context, property.name()) {
                validate_widget_style_value(theme, widget_id, *property, &value, params)?;
                style.values.insert(*property, value);
            }
        }
        resolved.parts.insert(part.name.clone(), style);
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    widget_kind.hash(&mut hasher);
    widget_id.hash(&mut hasher);
    theme.default_color_scheme.hash(&mut hasher);
    for source in &theme.css_sources {
        source.hash(&mut hasher);
    }
    let eval = crate::theme::eval::EvalContext::new(params, theme.get_base_font_size(params));
    for (property, value) in &resolved.host.values {
        property.hash(&mut hasher);
        format!(
            "{:?}",
            widget_style_scalar(widget_id, *property, value, &eval)?
        )
        .hash(&mut hasher);
    }
    for (part, style) in &resolved.parts {
        part.hash(&mut hasher);
        for (property, value) in &style.values {
            property.hash(&mut hasher);
            format!(
                "{:?}",
                widget_style_scalar(widget_id, *property, value, &eval)?
            )
            .hash(&mut hasher);
        }
    }
    resolved.digest = hasher.finish();
    Ok(resolved)
}

fn validate_widget_style_value(
    theme: &crate::Theme,
    widget_id: &str,
    property: WidgetStyleProperty,
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
) -> Result<(), AvengerChartError> {
    use crate::theme::eval::EvalContext;

    let eval = EvalContext::new(params, theme.get_base_font_size(params));
    let invalid = |message: String| AvengerChartError::InvalidWidgetStyle {
        widget_id: widget_id.to_string(),
        property: property.name().to_string(),
        message,
    };
    match property.value_type() {
        WidgetStyleValueType::Color => value
            .eval_as_color(&eval)
            .map(|_| ())
            .map_err(|error| invalid(error.to_string())),
        WidgetStyleValueType::Number => {
            let number = value
                .eval_as_number(&eval)
                .map_err(|error| invalid(error.to_string()))?;
            if !number.is_finite()
                || (matches!(
                    property,
                    WidgetStyleProperty::Opacity | WidgetStyleProperty::InputSelectionOpacity
                ) && !(0.0..=1.0).contains(&number))
            {
                return Err(invalid(format!("resolved to invalid number {number}")));
            }
            Ok(())
        }
        WidgetStyleValueType::Length => {
            let length = value
                .eval_as_length(&eval)
                .map_err(|error| invalid(error.to_string()))?;
            if !length.is_finite() || length < 0.0 {
                return Err(invalid(format!(
                    "resolved to non-finite or negative length {length}"
                )));
            }
            Ok(())
        }
        WidgetStyleValueType::String => value
            .eval_as_string(&eval)
            .map(|_| ())
            .map_err(|error| invalid(error.to_string())),
        WidgetStyleValueType::FontWeight => {
            if value.eval_as_number(&eval).is_ok() || value.eval_as_string(&eval).is_ok() {
                Ok(())
            } else {
                Err(invalid(
                    "expected a numeric or keyword font weight".to_string(),
                ))
            }
        }
        WidgetStyleValueType::Cursor => {
            let cursor = value
                .eval_as_string(&eval)
                .map_err(|error| invalid(error.to_string()))?;
            if matches!(
                cursor.as_str(),
                "default"
                    | "pointer"
                    | "text"
                    | "not-allowed"
                    | "crosshair"
                    | "grab"
                    | "grabbing"
                    | "ew-resize"
                    | "ns-resize"
                    | "nwse-resize"
                    | "nesw-resize"
            ) {
                Ok(())
            } else {
                Err(invalid(format!("unsupported cursor keyword '{cursor}'")))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NativeWidgetMeasureSpec {
    Declarative(WidgetMeasureSpec),
    Registry,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeWidgetStateSpec {
    params: Vec<CompiledParamSpec>,
}

impl<'de> Deserialize<'de> for NativeWidgetStateSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedState {
            params: Vec<CompiledParamSpec>,
        }

        let state = SerializedState::deserialize(deserializer)?;
        Self::try_new(state.params).map_err(serde::de::Error::custom)
    }
}

impl NativeWidgetStateSpec {
    pub fn try_new(params: Vec<CompiledParamSpec>) -> Result<Self, AvengerChartError> {
        let mut names = std::collections::HashSet::new();
        for param in &params {
            if !names.insert(param.name.clone()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate native widget state parameter '{}'",
                    param.name
                )));
            }
        }
        Ok(Self { params })
    }

    pub fn params(&self) -> &[CompiledParamSpec] {
        &self.params
    }
}

pub trait NativeWidget: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn kind(&self) -> &'static str;
    fn schema_version(&self) -> u32;
    fn payload(&self) -> serde_json::Value;
    fn measure(&self) -> NativeWidgetMeasureSpec;
    fn state(&self) -> NativeWidgetStateSpec;
}

#[derive(Clone)]
enum WidgetSourceInner {
    Composed(Arc<dyn ChartWidget>),
    Native(Arc<dyn NativeWidget>),
}

#[derive(Clone)]
pub struct WidgetSource(WidgetSourceInner);

impl WidgetSource {
    pub fn composed(widget: impl ChartWidget) -> Self {
        Self(WidgetSourceInner::Composed(Arc::new(widget)))
    }

    pub fn native(widget: impl NativeWidget) -> Self {
        Self(WidgetSourceInner::Native(Arc::new(widget)))
    }

    #[doc(hidden)]
    pub fn composed_widget(&self) -> Option<&dyn ChartWidget> {
        match &self.0 {
            WidgetSourceInner::Composed(widget) => Some(widget.as_ref()),
            WidgetSourceInner::Native(_) => None,
        }
    }

    #[doc(hidden)]
    pub fn native_widget(&self) -> Option<&dyn NativeWidget> {
        match &self.0 {
            WidgetSourceInner::Native(widget) => Some(widget.as_ref()),
            WidgetSourceInner::Composed(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetPlacement {
    Guide(ChromePosition),
    ExplicitFrame,
}

#[derive(Clone)]
pub struct WidgetAttachment {
    pub source: WidgetSource,
    pub placement: WidgetPlacement,
}

impl WidgetAttachment {
    pub fn composed<W: ChartWidget>(positioned: PositionedChartWidget<W>) -> Self {
        Self {
            source: WidgetSource::composed(positioned.widget),
            placement: WidgetPlacement::Guide(positioned.position),
        }
    }

    pub fn native<N: NativeWidget>(positioned: PositionedNativeWidget<N>) -> Self {
        Self {
            source: WidgetSource::native(positioned.widget),
            placement: WidgetPlacement::Guide(positioned.position),
        }
    }
}

pub struct PositionedChartWidget<W> {
    pub widget: W,
    pub position: ChromePosition,
}

pub trait ChartWidgetPlacementExt: ChartWidget + Sized {
    fn position(self, position: ChromePosition) -> PositionedChartWidget<Self> {
        PositionedChartWidget {
            widget: self,
            position,
        }
    }
}

impl<W: ChartWidget> ChartWidgetPlacementExt for W {}

pub struct PositionedNativeWidget<N> {
    pub widget: N,
    pub position: ChromePosition,
}

pub trait NativeWidgetPlacementExt: NativeWidget + Sized {
    fn position(self, position: ChromePosition) -> PositionedNativeWidget<Self> {
        PositionedNativeWidget {
            widget: self,
            position,
        }
    }
}

impl<N: NativeWidget> NativeWidgetPlacementExt for N {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonicalJson(String);

impl CanonicalJson {
    pub fn from_value(value: serde_json::Value) -> Result<Self, AvengerChartError> {
        fn canonicalize(value: serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Array(values) => {
                    serde_json::Value::Array(values.into_iter().map(canonicalize).collect())
                }
                serde_json::Value::Object(values) => {
                    let sorted = values
                        .into_iter()
                        .map(|(key, value)| (key, canonicalize(value)))
                        .collect::<BTreeMap<_, _>>();
                    serde_json::Value::Object(sorted.into_iter().collect())
                }
                value => value,
            }
        }

        serde_json::to_string(&canonicalize(value))
            .map(Self)
            .map_err(|error| AvengerChartError::SerializationError(error.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse_for(
        &self,
        widget_id: &str,
        kind: &str,
    ) -> Result<serde_json::Value, AvengerChartError> {
        serde_json::from_str(&self.0).map_err(|error| {
            AvengerChartError::MalformedNativeWidgetPayload {
                widget_id: widget_id.to_string(),
                kind: kind.to_string(),
                message: error.to_string(),
            }
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WidgetItemValidation {
    NonNullUnique {
        columns: Vec<String>,
        role: String,
    },
    ContainsScalar {
        column: String,
        #[serde_as(as = "FromInto<SerializableScalar>")]
        value: ScalarValue,
        role: String,
    },
    ContainsParam {
        column: String,
        param_name: String,
        role: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledWidgetItemPlan {
    pub data: CompiledDataContext,
    pub order_column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<WidgetItemIdentityDerivation>,
    pub validations: Vec<WidgetItemValidation>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledComposedWidget {
    pub id: String,
    pub kind: String,
    pub marks: Vec<Arc<dyn CompiledMark>>,
    pub relative_target_paths: BTreeMap<String, Vec<Vec<usize>>>,
    pub measure: WidgetMeasureSpec,
    pub items: Option<CompiledWidgetItemPlan>,
    #[serde(default)]
    pub presentation: CompiledWidgetPresentationSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledNativeWidgetSpec {
    pub id: String,
    pub kind: String,
    pub schema_version: u32,
    pub payload: CanonicalJson,
    pub measure: NativeWidgetMeasureSpec,
    pub state: NativeWidgetStateSpec,
}

#[derive(Clone, Serialize, Deserialize)]
// Boxing would change both the public variant API and the compiled artifact shape.
#[allow(clippy::large_enum_variant)]
pub enum CompiledWidget {
    Composed(CompiledComposedWidget),
    Native(CompiledNativeWidgetSpec),
}

impl CompiledWidget {
    pub fn id(&self) -> &str {
        match self {
            Self::Composed(widget) => &widget.id,
            Self::Native(widget) => &widget.id,
        }
    }

    pub fn kind(&self) -> &str {
        match self {
            Self::Composed(widget) => &widget.kind,
            Self::Native(widget) => &widget.kind,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledWidgetAttachment {
    pub widget: CompiledWidget,
    pub placement: WidgetPlacement,
    pub declaration_order: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_nested_objects_and_round_trips_bincode() {
        let payload = serde_json::json!({
            "z": null,
            "a": {"two": 2, "one": 1},
            "values": [true, "x", 1.5]
        });
        let canonical = CanonicalJson::from_value(payload).unwrap();
        assert_eq!(
            canonical.as_str(),
            r#"{"a":{"one":1,"two":2},"values":[true,"x",1.5],"z":null}"#
        );
        let bytes = bincode::serialize(&canonical).unwrap();
        let decoded: CanonicalJson = bincode::deserialize(&bytes).unwrap();
        assert_eq!(canonical, decoded);
    }

    #[test]
    fn malformed_canonical_json_is_a_structured_error() {
        let malformed: CanonicalJson = serde_json::from_str(r#""{""#).unwrap();
        assert!(matches!(
            malformed.parse_for("search", "text-input"),
            Err(AvengerChartError::MalformedNativeWidgetPayload { widget_id, kind, .. })
                if widget_id == "search" && kind == "text-input"
        ));
    }

    #[test]
    fn native_state_names_are_unique() {
        let param = crate::Param::new("value", 1_i64);
        let spec = CompiledParamSpec::shared(&param);
        assert!(NativeWidgetStateSpec::try_new(vec![spec.clone(), spec]).is_err());
    }

    #[test]
    fn native_state_deserialization_rejects_duplicate_names() {
        #[derive(Serialize)]
        struct UncheckedState {
            params: Vec<CompiledParamSpec>,
        }

        let spec = CompiledParamSpec::shared(&crate::Param::new("value", 1_i64));
        let bytes = bincode::serialize(&UncheckedState {
            params: vec![spec.clone(), spec],
        })
        .unwrap();
        assert!(bincode::deserialize::<NativeWidgetStateSpec>(&bytes).is_err());
    }

    #[test]
    fn style_property_table_has_unique_names_and_types() {
        let mut names = std::collections::HashSet::new();
        for property in WidgetStyleProperty::ALL {
            assert!(names.insert(property.name()));
            let _ = property.value_type();
        }
    }

    #[test]
    fn text_mark_channels_map_to_widget_css_properties() {
        assert_eq!(
            WidgetStyleProperty::for_mark_channel("color"),
            Some(WidgetStyleProperty::Fill)
        );
        assert_eq!(
            WidgetStyleProperty::for_mark_channel("font"),
            Some(WidgetStyleProperty::FontFamily)
        );
        assert_eq!(
            WidgetStyleProperty::for_mark_channel("font_size"),
            Some(WidgetStyleProperty::FontSize)
        );
    }

    #[test]
    fn widget_part_selector_uses_shadow_host_and_beats_mark_fallback() {
        let mut theme = crate::Theme::light();
        theme
            .append_css(
                r#"
                mark[type="rect"] { fill: #cc0000; }
                checkbox::part(box) { fill: #0072B2; }
                "#,
            )
            .unwrap();
        let host = crate::ThemeContext::new("checkbox", IndexMap::new()).with_id("choice");
        let part = crate::ThemeContext::new("mark", IndexMap::new())
            .with_subtype("rect")
            .with_part("box", host);
        let fill = theme.query_widget_part(&part, "fill").unwrap();
        assert!(
            matches!(fill, ThemeValue::Color(color) if color.blue == 178),
            "unexpected part fill: {fill:?}"
        );
    }

    #[test]
    fn resolved_style_uses_host_state_and_rejects_intrinsic_percentage() {
        let manifest = WidgetPartManifest {
            name: "box".to_string(),
            scene_mark_kind: "rect".to_string(),
            style_properties: vec![WidgetStyleProperty::Width],
            states: vec!["checked".to_string()],
            interactive: true,
        };
        let mut theme = crate::Theme::light();
        theme
            .append_css("checkbox[checked=true]::part(box) { width: 32px; }")
            .unwrap();
        let styles = resolve_widget_style_set(
            &theme,
            "checkbox",
            "choice",
            std::slice::from_ref(&manifest),
            &WidgetPresentationState {
                checked: Some(true),
                ..Default::default()
            },
            &IndexMap::new(),
        )
        .unwrap();
        assert!(matches!(
            styles.parts["box"].values[&WidgetStyleProperty::Width],
            ThemeValue::Length(32.0, _)
        ));

        theme
            .append_css("checkbox::part(box) { width: 50%; }")
            .unwrap();
        assert!(matches!(
            resolve_widget_style_set(
                &theme,
                "checkbox",
                "choice",
                &[manifest],
                &WidgetPresentationState::default(),
                &IndexMap::new(),
            ),
            Err(AvengerChartError::InvalidWidgetStyle { property, .. }) if property == "width"
        ));
    }

    #[test]
    fn resolved_style_digest_tracks_environment_and_ignores_unreferenced_variables() {
        let manifest = WidgetPartManifest {
            name: "box".to_string(),
            scene_mark_kind: "rect".to_string(),
            style_properties: vec![
                WidgetStyleProperty::Width,
                WidgetStyleProperty::Fill,
                WidgetStyleProperty::Stroke,
            ],
            states: Vec::new(),
            interactive: true,
        };
        let mut theme = crate::Theme::light();
        theme
            .append_css(
                r#"
                :root {
                    --digest-used: #0072b2;
                    --digest-unused: #999999;
                }
                digest-widget::part(box) {
                    width: 1rem;
                    fill: var(--digest-used);
                    stroke: light-dark(#000000, #ffffff);
                }
                @media (width >= 600px) {
                    digest-widget::part(box) { width: 2rem; }
                }
                "#,
            )
            .unwrap();
        let resolve = |theme: &crate::Theme, params: &IndexMap<String, ScalarValue>| {
            resolve_widget_style_set(
                theme,
                "digest-widget",
                "digest",
                std::slice::from_ref(&manifest),
                &WidgetPresentationState::default(),
                params,
            )
            .unwrap()
            .digest
        };
        let base = IndexMap::from([
            (
                "--base-font-size".to_string(),
                ScalarValue::Utf8(Some("10px".to_string())),
            ),
            (
                "color-scheme".to_string(),
                ScalarValue::Utf8(Some("light".to_string())),
            ),
            ("width".to_string(), ScalarValue::Float32(Some(500.0))),
        ]);
        let base_digest = resolve(&theme, &base);

        let mut base_font = base.clone();
        base_font.insert(
            "--base-font-size".to_string(),
            ScalarValue::Utf8(Some("20px".to_string())),
        );
        assert_ne!(base_digest, resolve(&theme, &base_font));

        let mut dark = base.clone();
        dark.insert(
            "color-scheme".to_string(),
            ScalarValue::Utf8(Some("dark".to_string())),
        );
        assert_ne!(base_digest, resolve(&theme, &dark));

        let mut used = base.clone();
        used.insert(
            "--digest-used".to_string(),
            ScalarValue::Utf8(Some("#d55e00".to_string())),
        );
        assert_ne!(base_digest, resolve(&theme, &used));

        let mut unused = base.clone();
        unused.insert(
            "--digest-unused".to_string(),
            ScalarValue::Utf8(Some("#d55e00".to_string())),
        );
        assert_eq!(base_digest, resolve(&theme, &unused));

        let mut wide = base.clone();
        wide.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
        assert_ne!(base_digest, resolve(&theme, &wide));

        let mut revised_source = theme.clone();
        revised_source
            .append_css("digest-widget::part(box) { width: 1rem; }")
            .unwrap();
        assert_ne!(base_digest, resolve(&revised_source, &base));
    }

    #[test]
    fn default_hybrid_button_style_resolves_geometry_accent_and_focus() {
        let parts = vec![
            WidgetPartManifest {
                name: "box".to_string(),
                scene_mark_kind: "rect".to_string(),
                style_properties: vec![
                    WidgetStyleProperty::Fill,
                    WidgetStyleProperty::StrokeWidth,
                    WidgetStyleProperty::ButtonRadius,
                ],
                states: Vec::new(),
                interactive: true,
            },
            WidgetPartManifest {
                name: "label".to_string(),
                scene_mark_kind: "text".to_string(),
                style_properties: vec![WidgetStyleProperty::Fill],
                states: Vec::new(),
                interactive: true,
            },
            WidgetPartManifest {
                name: "focus-ring".to_string(),
                scene_mark_kind: "rect".to_string(),
                style_properties: vec![
                    WidgetStyleProperty::Stroke,
                    WidgetStyleProperty::StrokeWidth,
                    WidgetStyleProperty::Opacity,
                ],
                states: Vec::new(),
                interactive: false,
            },
        ];
        let styles = resolve_widget_style_set(
            &crate::Theme::light(),
            "button",
            "clear",
            &parts,
            &WidgetPresentationState {
                variant: Some("accent".to_string()),
                focus_visible: true,
                ..Default::default()
            },
            &IndexMap::new(),
        )
        .unwrap();

        assert!(matches!(
            styles.host.values[&WidgetStyleProperty::Height],
            ThemeValue::Length(32.0, _)
        ));
        assert!(matches!(
            styles.parts["box"].values[&WidgetStyleProperty::Fill],
            ThemeValue::Color(color) if (color.red, color.green, color.blue) == (0, 114, 178)
        ));
        assert!(matches!(
            styles.parts["box"].values[&WidgetStyleProperty::ButtonRadius],
            ThemeValue::Length(4.0, _)
        ));
        assert!(matches!(
            styles.parts["label"].values[&WidgetStyleProperty::Fill],
            ThemeValue::Color(color) if (color.red, color.green, color.blue) == (255, 255, 255)
        ));
        assert_eq!(
            styles.parts["focus-ring"].values[&WidgetStyleProperty::Opacity],
            ThemeValue::Number(1.0)
        );

        let disabled_neutral = resolve_widget_style_set(
            &crate::Theme::light(),
            "button",
            "clear",
            &parts,
            &WidgetPresentationState {
                variant: Some("neutral".to_string()),
                disabled: true,
                ..Default::default()
            },
            &IndexMap::new(),
        )
        .unwrap();
        assert!(matches!(
            disabled_neutral.parts["box"].values[&WidgetStyleProperty::Fill],
            ThemeValue::Color(color) if (color.red, color.green, color.blue) == (244, 244, 244)
        ));
        assert!(matches!(
            disabled_neutral.parts["label"].values[&WidgetStyleProperty::Fill],
            ThemeValue::Color(color) if (color.red, color.green, color.blue) == (138, 138, 138)
        ));
    }

    #[test]
    fn symbolic_widget_measurement_uses_one_style_snapshot_and_item_count() {
        let mut styles = ResolvedWidgetStyleSet::default();
        styles.parts.insert(
            "row".to_string(),
            ResolvedWidgetPartStyle {
                values: IndexMap::from([(
                    WidgetStyleProperty::Height,
                    ThemeValue::Length(20.0, crate::theme::LengthUnit::Px),
                )]),
            },
        );
        let spec = WidgetMeasureSpec {
            width: WidgetAxisMeasureSpec::Content {
                expr: WidgetMeasureExpr::Add(vec![
                    WidgetMeasureExpr::TextExtent {
                        part: "label".to_string(),
                        axis: WidgetTextMeasureAxis::Width,
                        data_encoded: true,
                    },
                    WidgetMeasureExpr::Px(8.0),
                ]),
                min_px: 24.0,
                max_px: Some(80.0),
            },
            height: WidgetAxisMeasureSpec::Content {
                expr: WidgetMeasureExpr::ItemCount {
                    extent: Box::new(WidgetMeasureExpr::StyleLength {
                        part: Some("row".to_string()),
                        property: WidgetStyleProperty::Height,
                    }),
                    gap: Box::new(WidgetMeasureExpr::Px(4.0)),
                },
                min_px: 0.0,
                max_px: None,
            },
        };
        let mut calls = 0;
        let (width, height) = resolve_widget_measure_spec(
            "choices",
            &spec,
            &styles,
            3,
            &IndexMap::new(),
            12.0,
            |part, axis, data_encoded| {
                calls += 1;
                assert_eq!(part, "label");
                assert_eq!(axis, WidgetTextMeasureAxis::Width);
                assert!(data_encoded);
                Ok(100.0)
            },
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(width.min_px, 24.0);
        assert_eq!(width.preferred_px, 80.0);
        assert_eq!(height.preferred_px, 68.0);
    }

    #[test]
    fn symbolic_widget_measurement_rejects_invalid_bounds() {
        let result = resolve_widget_measure_spec(
            "broken",
            &WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::Px(20.0),
                    min_px: 30.0,
                    max_px: Some(10.0),
                },
                height: WidgetAxisMeasureSpec::Fixed { px: 10.0 },
            },
            &ResolvedWidgetStyleSet::default(),
            0,
            &IndexMap::new(),
            12.0,
            |_part, _axis, _data_encoded| Ok(0.0),
        );
        assert!(matches!(
            result,
            Err(AvengerChartError::InvalidArgument(message))
                if message.contains("maximum 10") && message.contains("minimum 30")
        ));
    }

    #[test]
    fn widget_item_identity_codec_is_stable_and_type_preserving() {
        let values = [
            ScalarValue::Int64(Some(1)),
            ScalarValue::UInt64(Some(1)),
            ScalarValue::Float64(Some(1.0)),
            ScalarValue::Boolean(Some(true)),
            ScalarValue::Utf8(Some("1".to_string())),
            ScalarValue::LargeUtf8(Some("1".to_string())),
        ];
        let encoded = values
            .iter()
            .map(WidgetItemIdentityCodec::encode)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            encoded.len(),
            encoded
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
        for (value, identity) in values.iter().zip(&encoded) {
            assert_eq!(WidgetItemIdentityCodec::encode(value).unwrap(), *identity);
            assert!(identity.starts_with("wii1_"));
        }
    }
}
