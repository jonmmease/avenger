//! Shared authoring and serialized contracts for chart widgets.

use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::Expr};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CompiledDataContext, CompiledMark, CompiledParamSpec, LegendPosition,
    PixelFrame, ThemeValue, ToolExpansion, serialization::SerializableScalar,
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
}

pub struct WidgetExpansion {
    pub expansion: ToolExpansion<PixelFrame>,
    pub items: Option<WidgetItems>,
    pub measure: WidgetMeasureSpec,
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
pub enum WidgetItems {
    Static(Vec<WidgetItemRow>),
    DataFrame {
        data: DataFrame,
        order_key: Vec<Expr>,
    },
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

    /// Conservative pre-measurement frame used while the W1.4 host solver is
    /// not yet active. Content/fill axes use their declared minimum.
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
    matches!(name, "focus-ring" | "selection" | "caret" | "preedit")
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
    for (property, value) in &resolved.host.values {
        property.hash(&mut hasher);
        serde_json::to_string(value)
            .map_err(|error| AvengerChartError::SerializationError(error.to_string()))?
            .hash(&mut hasher);
    }
    for (part, style) in &resolved.parts {
        part.hash(&mut hasher);
        for (property, value) in &style.values {
            property.hash(&mut hasher);
            serde_json::to_string(value)
                .map_err(|error| AvengerChartError::SerializationError(error.to_string()))?
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
    }
}
