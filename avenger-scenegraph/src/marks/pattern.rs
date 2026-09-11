use std::hash::{Hash, Hasher};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

pub type Px = f32;
pub type Deg = f32;
pub type Alpha = f32;
pub type Rgba = [f32; 4];
pub type SymbolShapeSpec = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub struct PatternFill {
    #[serde(default)]
    pub anchor: PatternAnchor,

    #[serde(default)]
    pub ink: PatternInk,

    #[serde(default)]
    pub layers: Vec<PatternLayer>,
}

impl Hash for PatternFill {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.anchor.hash(state);
        self.ink.hash(state);
        self.layers.hash(state);
    }
}

impl PatternFill {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        self.ink.validate()?;

        for layer in &self.layers {
            layer.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternAnchor {
    #[default]
    Plot,
    Mark,
    Chart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternInk {
    AutoContrast {
        #[serde(default = "default_pattern_opacity")]
        opacity: Alpha,
    },
    Solid {
        color: Rgba,
        #[serde(default = "default_pattern_opacity")]
        opacity: Alpha,
    },
}

impl Default for PatternInk {
    fn default() -> Self {
        Self::AutoContrast {
            opacity: default_pattern_opacity(),
        }
    }
}

impl Hash for PatternInk {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::AutoContrast { opacity } => {
                0_u8.hash(state);
                hash_f32(*opacity, state);
            }
            Self::Solid { color, opacity } => {
                1_u8.hash(state);
                hash_rgba(*color, state);
                hash_f32(*opacity, state);
            }
        }
    }
}

impl PatternInk {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        match self {
            Self::AutoContrast { opacity } | Self::Solid { opacity, .. } => {
                validate_alpha(*opacity, "ink.opacity")?;
            }
        }

        if let Self::Solid { color, .. } = self {
            for component in color {
                if !component.is_finite() {
                    return Err(PatternValidationError::NonFinite("ink.color"));
                }
            }
        }

        Ok(())
    }
}

pub fn default_pattern_opacity() -> Alpha {
    0.18
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternLayerOperation {
    #[default]
    Add,
    Subtract,
    Xor,
}

fn is_add_operation(operation: &PatternLayerOperation) -> bool {
    matches!(operation, PatternLayerOperation::Add)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternLayer {
    Stripe(StripePatternLayer),
    Symbol(SymbolPatternLayer),
}

impl Hash for PatternLayer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Stripe(layer) => {
                0_u8.hash(state);
                layer.hash(state);
            }
            Self::Symbol(layer) => {
                1_u8.hash(state);
                layer.hash(state);
            }
        }
    }
}

impl PatternLayer {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        match self {
            Self::Stripe(layer) => layer.validate(),
            Self::Symbol(layer) => layer.validate(),
        }
    }

    pub fn operation(&self) -> PatternLayerOperation {
        match self {
            Self::Stripe(layer) => layer.operation,
            Self::Symbol(layer) => layer.operation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StripePatternLayer {
    #[serde(default, skip_serializing_if = "is_add_operation")]
    pub operation: PatternLayerOperation,

    pub angle: Deg,
    pub spacing: Px,
    pub stroke_width: Px,

    #[serde(default)]
    pub phase: Px,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<StripeDash>,
}

impl Hash for StripePatternLayer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.operation.hash(state);
        hash_f32(self.angle, state);
        hash_f32(self.spacing, state);
        hash_f32(self.stroke_width, state);
        hash_f32(self.phase, state);
        self.dash.hash(state);
    }
}

impl StripePatternLayer {
    pub fn new(angle: Deg, spacing: Px, stroke_width: Px) -> Self {
        Self {
            operation: PatternLayerOperation::Add,
            angle,
            spacing,
            stroke_width,
            phase: 0.0,
            dash: None,
        }
    }

    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_finite(self.angle, "stripe.angle")?;
        validate_positive(self.spacing, "stripe.spacing")?;
        validate_positive(self.stroke_width, "stripe.stroke_width")?;
        validate_finite(self.phase, "stripe.phase")?;

        if let Some(dash) = &self.dash {
            dash.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StripeDash {
    pub length: Px,
    pub gap: Px,

    #[serde(default)]
    pub phase: Px,
}

impl Hash for StripeDash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32(self.length, state);
        hash_f32(self.gap, state);
        hash_f32(self.phase, state);
    }
}

impl StripeDash {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_positive(self.length, "dash.length")?;
        validate_positive(self.gap, "dash.gap")?;
        validate_finite(self.phase, "dash.phase")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolPatternLayer {
    #[serde(default, skip_serializing_if = "is_add_operation")]
    pub operation: PatternLayerOperation,

    pub lattice: SymbolLattice2d,
    pub symbol: PatternSymbol,
    pub paint: SymbolPaint,
}

impl Hash for SymbolPatternLayer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.operation.hash(state);
        self.lattice.hash(state);
        self.symbol.hash(state);
        self.paint.hash(state);
    }
}

impl SymbolPatternLayer {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        self.lattice.validate()?;
        self.symbol.validate()?;
        self.paint.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolLattice2d {
    pub u_spacing: Px,
    pub u_angle: Deg,
    pub v_spacing: Px,
    pub v_angle: Deg,

    #[serde(default)]
    pub u_phase: Px,

    #[serde(default)]
    pub v_phase: Px,
}

impl Hash for SymbolLattice2d {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32(self.u_spacing, state);
        hash_f32(self.u_angle, state);
        hash_f32(self.v_spacing, state);
        hash_f32(self.v_angle, state);
        hash_f32(self.u_phase, state);
        hash_f32(self.v_phase, state);
    }
}

impl SymbolLattice2d {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_positive(self.u_spacing, "symbol.lattice.u_spacing")?;
        validate_finite(self.u_angle, "symbol.lattice.u_angle")?;
        validate_positive(self.v_spacing, "symbol.lattice.v_spacing")?;
        validate_finite(self.v_angle, "symbol.lattice.v_angle")?;
        validate_finite(self.u_phase, "symbol.lattice.u_phase")?;
        validate_finite(self.v_phase, "symbol.lattice.v_phase")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternSymbol {
    pub shape: SymbolShapeSpec,
    pub size: Px,

    #[serde(default)]
    pub rotation: Deg,
}

impl Hash for PatternSymbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.shape.hash(state);
        hash_f32(self.size, state);
        hash_f32(self.rotation, state);
    }
}

impl PatternSymbol {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        if self.shape.is_empty() {
            return Err(PatternValidationError::EmptyString("symbol.shape"));
        }
        validate_positive(self.size, "symbol.size")?;
        validate_finite(self.rotation, "symbol.rotation")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SymbolPaint {
    Filled,
    Open { stroke_width: Px },
}

impl Hash for SymbolPaint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Filled => 0_u8.hash(state),
            Self::Open { stroke_width } => {
                1_u8.hash(state);
                hash_f32(*stroke_width, state);
            }
        }
    }
}

impl SymbolPaint {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        match self {
            Self::Filled => Ok(()),
            Self::Open { stroke_width } => {
                validate_positive(*stroke_width, "symbol.paint.stroke_width")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternReferenceFrame {
    pub x: Px,
    pub y: Px,
    pub width: Px,
    pub height: Px,
}

impl Hash for PatternReferenceFrame {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32(self.x, state);
        hash_f32(self.y, state);
        hash_f32(self.width, state);
        hash_f32(self.height, state);
    }
}

impl PatternReferenceFrame {
    pub fn translated(&self, x: Px, y: Px) -> Self {
        Self {
            x: self.x + x,
            y: self.y + y,
            width: self.width,
            height: self.height,
        }
    }

    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_finite(self.x, "pattern_reference_frame.x")?;
        validate_finite(self.y, "pattern_reference_frame.y")?;
        validate_positive(self.width, "pattern_reference_frame.width")?;
        validate_positive(self.height, "pattern_reference_frame.height")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternValidationError {
    NonFinite(&'static str),
    NonPositive(&'static str),
    InvalidAlpha(&'static str),
    EmptyString(&'static str),
}

pub fn default_no_fill_pattern() -> ScalarOrArray<Option<PatternFill>> {
    ScalarOrArray::new_scalar(None)
}

pub fn is_no_fill_pattern(value: &ScalarOrArray<Option<PatternFill>>) -> bool {
    matches!(value.value(), ScalarOrArrayValue::Scalar(None))
}

pub fn hash_fill_pattern_scalar_or_array<H: Hasher>(
    value: &ScalarOrArray<Option<PatternFill>>,
    state: &mut H,
) {
    match value.value() {
        ScalarOrArrayValue::Scalar(value) => {
            0_u8.hash(state);
            value.hash(state);
        }
        ScalarOrArrayValue::Array(values) => {
            1_u8.hash(state);
            for value in values.iter() {
                value.hash(state);
            }
        }
    }
}

fn validate_finite(value: f32, field: &'static str) -> Result<(), PatternValidationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(PatternValidationError::NonFinite(field))
    }
}

fn validate_positive(value: f32, field: &'static str) -> Result<(), PatternValidationError> {
    validate_finite(value, field)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(PatternValidationError::NonPositive(field))
    }
}

fn validate_alpha(value: f32, field: &'static str) -> Result<(), PatternValidationError> {
    validate_finite(value, field)?;
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(PatternValidationError::InvalidAlpha(field))
    }
}

fn hash_rgba<H: Hasher>(color: Rgba, state: &mut H) {
    for value in color {
        hash_f32(value, state);
    }
}

fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
    OrderedFloat(value).hash(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    #[test]
    fn stripe_layer_serializes_with_internal_type_tag() {
        let layer = PatternLayer::Stripe(StripePatternLayer::new(45.0, 16.0, 1.25));
        let value = serde_json::to_value(layer).unwrap();

        assert_eq!(value["type"], "stripe");
        assert_eq!(value["angle"], 45.0);
        assert_eq!(value["spacing"], 16.0);
        assert_eq!(value["stroke-width"], 1.25);
        assert!(value.get("operation").is_none());
    }

    #[test]
    fn pattern_layer_operation_round_trips_explicit_values() {
        for (operation, expected) in [
            (PatternLayerOperation::Add, "add"),
            (PatternLayerOperation::Subtract, "subtract"),
            (PatternLayerOperation::Xor, "xor"),
        ] {
            let mut layer = StripePatternLayer::new(45.0, 16.0, 1.25);
            layer.operation = operation;
            let value = serde_json::to_value(PatternLayer::Stripe(layer)).unwrap();

            if operation == PatternLayerOperation::Add {
                assert!(value.get("operation").is_none());
            } else {
                assert_eq!(value["operation"], expected);
            }

            let mut value = value;
            value["operation"] = serde_json::Value::String(expected.to_string());
            let restored: PatternLayer = serde_json::from_value(value).unwrap();
            assert_eq!(restored.operation(), operation);
        }
    }

    #[test]
    fn omitted_pattern_layer_operation_deserializes_to_add() {
        let value = serde_json::json!({
            "type": "stripe",
            "angle": 45.0,
            "spacing": 16.0,
            "stroke-width": 1.25
        });
        let layer: PatternLayer = serde_json::from_value(value).unwrap();

        assert_eq!(layer.operation(), PatternLayerOperation::Add);
    }

    #[test]
    fn layer_hash_changes_when_operation_changes() {
        let add = StripePatternLayer::new(45.0, 16.0, 1.25);
        let mut subtract = add.clone();
        subtract.operation = PatternLayerOperation::Subtract;

        let mut add_hasher = DefaultHasher::new();
        add.hash(&mut add_hasher);
        let mut subtract_hasher = DefaultHasher::new();
        subtract.hash(&mut subtract_hasher);

        assert_ne!(add_hasher.finish(), subtract_hasher.finish());
    }

    #[test]
    fn default_pattern_fill_uses_plot_anchor_and_auto_contrast() {
        let fill = PatternFill::default();

        assert_eq!(fill.anchor, PatternAnchor::Plot);
        assert!(matches!(
            fill.ink,
            PatternInk::AutoContrast { opacity } if opacity == default_pattern_opacity()
        ));
        assert!(fill.layers.is_empty());
    }

    #[test]
    fn pattern_validation_rejects_invalid_stripe_spacing() {
        let fill = PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
                45.0, 0.0, 1.0,
            ))],
            ..Default::default()
        };

        assert_eq!(
            fill.validate(),
            Err(PatternValidationError::NonPositive("stripe.spacing"))
        );
    }
}
