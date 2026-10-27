//! Pattern fills: layers of stripes and repeated symbols over the fill of a mark.
//!
//! # Text
//!
//! A [`PatternFill`] reads its text with [`FromStr`](std::str::FromStr) and writes it with
//! [`Display`](std::fmt::Display).
//!
//! ```text
//! pattern  = ( "none" | layers ) { ";" option }
//! layers   = [ op ] layer { op layer }
//! op       = "+" | "-" | "^"
//! layer    = stripes | symbols
//!
//! stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ]
//! symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice
//! lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE )
//!            [ "at" ANGLE ] [ "offset" PHASE PHASE ]
//!
//! option   = "ink" ( "auto" | COLOR ) [ OPACITY ]
//!          | "anchor" ( "group" | "mark" | "scene" )
//! ```
//!
//! Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding
//! square in square pixels, as for symbol marks, and `SHAPE` is the name of a
//! [`PatternSymbolShape`], such as `circle` or `triangle-up`. `COLOR` is a CSS color. Spaces
//! separate the tokens, so an operator has a space on each side and `-45` is a number, and the
//! spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.
//!
//! The operators combine each layer with the coverage of the layers before it: `+` adds, `-`
//! subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in
//! any order, each at most once.
//!
//! `grid` and `hex` write a lattice with steps at 0° and 90°, and with equal steps at 0° and
//! 60°, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A
//! pattern that leaves out an option takes its default: `ink auto 0.18` and `anchor group`. `none`
//! is a pattern without layers.
//!
//! [`Display`](std::fmt::Display) writes the canonical text, without the defaults, so that equal patterns have equal
//! text. Reading the text that it writes gives back the same pattern, for patterns whose ink
//! colors a CSS color can write: an opaque ink color as `#rrggbb`, and another as `rgba(…)`.
//!
//! ```
//! use avenger_common::pattern::PatternFill;
//!
//! let crosshatch: PatternFill = "stripes 45 8 1.5 + stripes -45 8 1.5".parse().unwrap();
//! assert_eq!(crosshatch.layers.len(), 2);
//!
//! let dots: PatternFill = "circle 12 on grid 10 10 at 0; ink auto 0.18".parse().unwrap();
//! assert_eq!(dots.to_string(), "circle 12 on grid 10");
//! ```

use crate::hash::hash_floats;
use crate::types::SymbolShape;
use crate::value::{hash_values, ScalarOrArray, ScalarOrArrayValue};
use educe::Educe;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod grammar;

pub use grammar::PatternParseError;

/// A fill drawn over a mark's own fill: layers of stripes and repeated symbols, painted in one ink.
#[derive(Debug, Clone, Default, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternFill {
    #[serde(default)]
    pub anchor: PatternAnchor,

    #[serde(default)]
    pub ink: PatternInk,

    #[serde(default)]
    pub layers: Vec<PatternLayer>,
}

impl PatternFill {
    /// The most layers a pattern has, so that renderers can store every pattern in a record of
    /// one size.
    pub const MAX_LAYERS: usize = 8;

    pub fn validate(&self) -> Result<(), PatternValidationError> {
        if self.layers.len() > Self::MAX_LAYERS {
            return Err(PatternValidationError::TooManyLayers(self.layers.len()));
        }
        self.ink.validate()?;

        for layer in &self.layers {
            layer.validate()?;
        }

        Ok(())
    }
}

/// Where a pattern's stripes and lattices start, so that patterns line up across marks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternAnchor {
    /// The pattern origin of the nearest enclosing group that sets one, or the scene's top-left
    /// corner.
    #[default]
    Group,
    /// The top-left corner of each mark item's bounds, which for a symbol is its nominal square.
    Mark,
    /// The scene's top-left corner.
    Scene,
}

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternInk {
    /// Black or white, whichever contrasts with the mark's fill, at an opacity from 0 to 1.
    AutoContrast {
        #[serde(default = "default_pattern_opacity")]
        #[educe(Hash(method(hash_floats)))]
        opacity: f32,
    },
    /// An RGBA color, whose alpha is multiplied by an opacity from 0 to 1.
    Solid {
        #[educe(Hash(method(hash_floats)))]
        color: [f32; 4],
        #[serde(default = "default_pattern_opacity")]
        #[educe(Hash(method(hash_floats)))]
        opacity: f32,
    },
}

impl Default for PatternInk {
    fn default() -> Self {
        Self::AutoContrast {
            opacity: default_pattern_opacity(),
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

pub fn default_pattern_opacity() -> f32 {
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

/// One layer of a pattern: coverage of one kind, and how it combines with the coverage of the
/// layers before it.
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternLayer {
    #[serde(default, skip_serializing_if = "is_add_operation")]
    pub operation: PatternLayerOperation,

    #[serde(flatten)]
    pub kind: PatternLayerKind,
}

impl PatternLayer {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        match &self.kind {
            PatternLayerKind::Stripe(layer) => layer.validate(),
            PatternLayerKind::Symbol(layer) => layer.validate(),
        }
    }

    /// The layer with `operation`.
    pub fn with_operation(self, operation: PatternLayerOperation) -> Self {
        Self { operation, ..self }
    }
}

/// The coverage of a layer. Each kind has a closed-form distance to its edge, so renderers can
/// compute coverage per pixel.
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternLayerKind {
    Stripe(StripeLayer),
    Symbol(SymbolLayer),
}

impl From<StripeLayer> for PatternLayer {
    fn from(layer: StripeLayer) -> Self {
        Self {
            operation: PatternLayerOperation::Add,
            kind: PatternLayerKind::Stripe(layer),
        }
    }
}

impl From<SymbolLayer> for PatternLayer {
    fn from(layer: SymbolLayer) -> Self {
        Self {
            operation: PatternLayerOperation::Add,
            kind: PatternLayerKind::Symbol(layer),
        }
    }
}

/// Parallel stripes. Lengths are in pixels and angles in degrees.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct StripeLayer {
    #[educe(Hash(method(hash_floats)))]
    pub angle: f32,
    #[educe(Hash(method(hash_floats)))]
    pub spacing: f32,
    #[educe(Hash(method(hash_floats)))]
    pub stroke_width: f32,

    #[serde(default)]
    #[educe(Hash(method(hash_floats)))]
    pub phase: f32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<StripeDash>,
}

impl StripeLayer {
    pub fn new(angle: f32, spacing: f32, stroke_width: f32) -> Self {
        Self {
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

/// Dashes along each stripe, in pixels.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct StripeDash {
    #[educe(Hash(method(hash_floats)))]
    pub length: f32,
    #[educe(Hash(method(hash_floats)))]
    pub gap: f32,

    #[serde(default)]
    #[educe(Hash(method(hash_floats)))]
    pub phase: f32,
}

impl StripeDash {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_positive(self.length, "dash.length")?;
        validate_positive(self.gap, "dash.gap")?;
        validate_finite(self.phase, "dash.phase")
    }
}

/// A symbol repeated on a lattice.
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolLayer {
    pub lattice: SymbolLattice2d,
    pub symbol: PatternSymbol,
    pub paint: SymbolPaint,
}

impl SymbolLayer {
    /// How many of its lattice's shortest steps a symbol can span, with its stroke. Each pixel
    /// looks for the symbols that reach it, so the bound limits that search.
    pub const MAX_SPAN: f32 = 4.0;

    pub fn validate(&self) -> Result<(), PatternValidationError> {
        self.lattice.validate()?;
        self.symbol.validate()?;
        self.paint.validate()?;
        let stroke_width = match self.paint {
            SymbolPaint::Filled => 0.0,
            SymbolPaint::Open { stroke_width } => stroke_width,
        };
        let (shortest, _) = self.lattice.reduced_steps();
        if self.symbol.size.sqrt() + stroke_width > Self::MAX_SPAN * shortest[0].hypot(shortest[1])
        {
            return Err(PatternValidationError::SymbolOverlap);
        }
        Ok(())
    }
}

/// How far below this the area of a lattice cell is too small for symbols to repeat on, in square
/// pixels.
const SYMBOL_LATTICE_MIN_AREA: f32 = 1e-4;

/// A lattice of symbol positions along two directions. Spacings and phases are in pixels, and
/// angles are in degrees.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolLattice2d {
    #[educe(Hash(method(hash_floats)))]
    pub u_spacing: f32,
    #[educe(Hash(method(hash_floats)))]
    pub u_angle: f32,
    #[educe(Hash(method(hash_floats)))]
    pub v_spacing: f32,
    #[educe(Hash(method(hash_floats)))]
    pub v_angle: f32,

    #[serde(default)]
    #[educe(Hash(method(hash_floats)))]
    pub u_phase: f32,

    #[serde(default)]
    #[educe(Hash(method(hash_floats)))]
    pub v_phase: f32,
}

impl SymbolLattice2d {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_positive(self.u_spacing, "symbol.lattice.u_spacing")?;
        validate_finite(self.u_angle, "symbol.lattice.u_angle")?;
        validate_positive(self.v_spacing, "symbol.lattice.v_spacing")?;
        validate_finite(self.v_angle, "symbol.lattice.v_angle")?;
        validate_finite(self.u_phase, "symbol.lattice.u_phase")?;
        validate_finite(self.v_phase, "symbol.lattice.v_phase")?;
        if self.cell_area().abs() < SYMBOL_LATTICE_MIN_AREA {
            return Err(PatternValidationError::ParallelLattice);
        }
        Ok(())
    }

    /// The step along `u` and the step along `v`, as vectors in pixels.
    pub fn steps(&self) -> ([f32; 2], [f32; 2]) {
        let step = |spacing: f32, angle: f32| {
            let theta = angle.to_radians();
            [theta.cos() * spacing, theta.sin() * spacing]
        };
        (
            step(self.u_spacing, self.u_angle),
            step(self.v_spacing, self.v_angle),
        )
    }

    /// The signed area of a lattice cell, in square pixels.
    pub fn cell_area(&self) -> f32 {
        let (u, v) = self.steps();
        u[0] * v[1] - u[1] * v[0]
    }

    /// Steps that reach the same lattice points as [`Self::steps`]: the first is a shortest step
    /// of the lattice, and the second is at least 60 degrees from it.
    pub fn reduced_steps(&self) -> ([f32; 2], [f32; 2]) {
        let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
        let (u, v) = self.steps();
        let mut u = [f64::from(u[0]), f64::from(u[1])];
        let mut v = [f64::from(v[0]), f64::from(v[1])];
        if dot(u, u) > dot(v, v) {
            std::mem::swap(&mut u, &mut v);
        }
        // Lagrange-Gauss reduction: subtract the nearest multiple of the shorter step from the
        // longer one until the longer one stops getting shorter.
        for _ in 0..256 {
            let multiple = (dot(u, v) / dot(u, u)).round();
            v = [v[0] - multiple * u[0], v[1] - multiple * u[1]];
            if dot(v, v) >= dot(u, u) {
                break;
            }
            std::mem::swap(&mut u, &mut v);
        }
        ([u[0] as f32, u[1] as f32], [v[0] as f32, v[1] as f32])
    }
}

/// A symbol in a lattice: its shape, the area of its bounding square in square pixels (as for
/// symbol marks), and its rotation in degrees.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct PatternSymbol {
    pub shape: PatternSymbolShape,
    #[educe(Hash(method(hash_floats)))]
    pub size: f32,

    #[serde(default)]
    #[educe(Hash(method(hash_floats)))]
    pub rotation: f32,
}

/// The shape of a pattern symbol: one of the built-in symbol shapes, each with a closed-form
/// distance to its edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternSymbolShape {
    #[default]
    Circle,
    Square,
    Diamond,
    Cross,
    TriangleUp,
    TriangleDown,
    TriangleLeft,
    TriangleRight,
    Triangle,
    Wedge,
    Pentagon,
    Arrow,
    Star,
    Wye,
    #[serde(rename = "star-4")]
    Star4,
}

impl PatternSymbolShape {
    /// Every shape.
    pub const ALL: [Self; 15] = [
        Self::Circle,
        Self::Square,
        Self::Diamond,
        Self::Cross,
        Self::TriangleUp,
        Self::TriangleDown,
        Self::TriangleLeft,
        Self::TriangleRight,
        Self::Triangle,
        Self::Wedge,
        Self::Pentagon,
        Self::Arrow,
        Self::Star,
        Self::Wye,
        Self::Star4,
    ];

    /// The shape's name, as symbol marks name it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Square => "square",
            Self::Diamond => "diamond",
            Self::Cross => "cross",
            Self::TriangleUp => "triangle-up",
            Self::TriangleDown => "triangle-down",
            Self::TriangleLeft => "triangle-left",
            Self::TriangleRight => "triangle-right",
            Self::Triangle => "triangle",
            Self::Wedge => "wedge",
            Self::Pentagon => "pentagon",
            Self::Arrow => "arrow",
            Self::Star => "star",
            Self::Wye => "wye",
            Self::Star4 => "star-4",
        }
    }

    /// The symbol mark shape of the same name, whose path fits a one-pixel square.
    pub fn symbol_shape(self) -> SymbolShape {
        SymbolShape::from_vega_str(self.name()).expect("a built-in symbol shape parses")
    }
}

impl PatternSymbol {
    pub fn validate(&self) -> Result<(), PatternValidationError> {
        validate_positive(self.size, "symbol.size")?;
        validate_finite(self.rotation, "symbol.rotation")
    }
}

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "kebab-case"
)]
pub enum SymbolPaint {
    Filled,
    /// The outline, with a stroke width in pixels.
    Open {
        #[educe(Hash(method(hash_floats)))]
        stroke_width: f32,
    },
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

/// Why a pattern cannot be drawn.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatternValidationError {
    #[error("pattern {0} must be finite")]
    NonFinite(&'static str),
    #[error("pattern {0} must be positive")]
    NonPositive(&'static str),
    #[error("pattern {0} must be from 0 to 1")]
    InvalidAlpha(&'static str),
    #[error("a pattern symbol lattice needs two directions that are not parallel")]
    ParallelLattice,
    #[error("a pattern has at most {max} layers, not {0}", max = PatternFill::MAX_LAYERS)]
    TooManyLayers(usize),
    #[error(
        "a pattern symbol spans at most {max} of its lattice's shortest steps",
        max = SymbolLayer::MAX_SPAN
    )]
    SymbolOverlap,
}

/// The pattern fills of a mark: one for every item, or one for each item. `None` draws no
/// pattern.
#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(transparent)]
pub struct PatternFills(#[educe(Hash(method(hash_values)))] pub ScalarOrArray<Option<PatternFill>>);

impl Default for PatternFills {
    fn default() -> Self {
        Self(ScalarOrArray::new_scalar(None))
    }
}

impl PatternFills {
    /// Whether no item has a pattern fill, because the fills are one `None` for every item.
    pub fn is_none(&self) -> bool {
        matches!(self.0.value(), ScalarOrArrayValue::Scalar(None))
    }

    /// The pattern fill of each of `len` items, through `indices` when the mark has them.
    pub fn iter<'a>(
        &'a self,
        len: usize,
        indices: Option<&'a std::sync::Arc<Vec<usize>>>,
    ) -> Box<dyn Iterator<Item = &'a Option<PatternFill>> + 'a> {
        self.0.as_iter(len, indices)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<PatternFill> for PatternFills {
    fn from(fill: PatternFill) -> Self {
        Self(ScalarOrArray::new_scalar(Some(fill)))
    }
}

impl From<Option<PatternFill>> for PatternFills {
    fn from(fill: Option<PatternFill>) -> Self {
        Self(ScalarOrArray::new_scalar(fill))
    }
}

impl From<Vec<Option<PatternFill>>> for PatternFills {
    fn from(fills: Vec<Option<PatternFill>>) -> Self {
        Self(ScalarOrArray::new_array(fills))
    }
}

impl From<ScalarOrArray<Option<PatternFill>>> for PatternFills {
    fn from(fills: ScalarOrArray<Option<PatternFill>>) -> Self {
        Self(fills)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{DefaultHasher, Hash, Hasher};

    #[test]
    fn pattern_layer_operation_round_trips_explicit_values() {
        for (operation, expected) in [
            (PatternLayerOperation::Add, "add"),
            (PatternLayerOperation::Subtract, "subtract"),
            (PatternLayerOperation::Xor, "xor"),
        ] {
            let layer =
                PatternLayer::from(StripeLayer::new(45.0, 16.0, 1.25)).with_operation(operation);
            let value = serde_json::to_value(&layer).unwrap();

            if operation == PatternLayerOperation::Add {
                assert!(value.get("operation").is_none());
            } else {
                assert_eq!(value["operation"], expected);
            }

            assert_eq!(value["type"], "stripe");
            assert_eq!(value["angle"], 45.0);
            assert_eq!(value["spacing"], 16.0);
            assert_eq!(value["stroke-width"], 1.25);
            let restored: PatternLayer = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(restored, layer);
            let mut value = value;
            value["operation"] = serde_json::Value::String(expected.to_string());
            let restored: PatternLayer = serde_json::from_value(value).unwrap();
            assert_eq!(restored.operation, operation);
        }
    }

    #[test]
    fn layer_hash_changes_when_operation_changes() {
        let add = PatternLayer::from(StripeLayer::new(45.0, 16.0, 1.25));
        let subtract = add.clone().with_operation(PatternLayerOperation::Subtract);

        let mut add_hasher = DefaultHasher::new();
        add.hash(&mut add_hasher);
        let mut subtract_hasher = DefaultHasher::new();
        subtract.hash(&mut subtract_hasher);

        assert_ne!(add_hasher.finish(), subtract_hasher.finish());
    }

    #[test]
    fn default_pattern_fill_uses_frame_anchor_and_auto_contrast() {
        let fill = PatternFill::default();

        assert_eq!(fill.anchor, PatternAnchor::Group);
        assert!(matches!(
            fill.ink,
            PatternInk::AutoContrast { opacity } if opacity == default_pattern_opacity()
        ));
        assert!(fill.layers.is_empty());
    }

    #[test]
    fn pattern_validation_rejects_invalid_stripe_spacing() {
        let fill = PatternFill {
            layers: vec![StripeLayer::new(45.0, 0.0, 1.0).into()],
            ..Default::default()
        };

        assert_eq!(
            fill.validate(),
            Err(PatternValidationError::NonPositive("stripe.spacing"))
        );
    }

    #[test]
    fn patterns_have_at_most_max_layers() {
        let layers = |count| PatternFill {
            layers: vec![StripeLayer::new(45.0, 8.0, 1.0).into(); count],
            ..Default::default()
        };
        assert_eq!(layers(PatternFill::MAX_LAYERS).validate(), Ok(()));
        assert_eq!(
            layers(PatternFill::MAX_LAYERS + 1).validate(),
            Err(PatternValidationError::TooManyLayers(
                PatternFill::MAX_LAYERS + 1
            ))
        );
    }

    #[test]
    fn symbol_layers_name_their_shape() {
        let layer = PatternLayer::from(SymbolLayer {
            lattice: SymbolLattice2d {
                u_spacing: 8.0,
                u_angle: 0.0,
                v_spacing: 8.0,
                v_angle: 90.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: PatternSymbolShape::TriangleUp,
                size: 16.0,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        })
        .with_operation(PatternLayerOperation::Xor);
        let value = serde_json::to_value(&layer).unwrap();
        assert_eq!(value["type"], "symbol");
        assert_eq!(value["operation"], "xor");
        assert_eq!(value["symbol"]["shape"], "triangle-up");
        assert_eq!(
            serde_json::from_value::<PatternLayer>(value).unwrap(),
            layer
        );
    }

    #[test]
    fn open_symbol_paint_names_its_fields_in_kebab_case() {
        let paint = SymbolPaint::Open { stroke_width: 1.5 };
        let value = serde_json::to_value(&paint).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type": "open", "stroke-width": 1.5})
        );
        assert_eq!(serde_json::from_value::<SymbolPaint>(value).unwrap(), paint);
    }

    #[test]
    fn parallel_symbol_lattice_directions_are_invalid() {
        let lattice = SymbolLattice2d {
            u_spacing: 8.0,
            u_angle: 0.0,
            v_spacing: 8.0,
            v_angle: 180.0,
            u_phase: 0.0,
            v_phase: 0.0,
        };
        assert_eq!(
            lattice.validate(),
            Err(PatternValidationError::ParallelLattice)
        );
    }

    #[test]
    fn reduced_lattice_steps_reach_the_same_points() {
        // Steps (10, 0) and (52, 3) reach the same points as (2, 3) and (6, -6).
        let lattice = SymbolLattice2d {
            u_spacing: 10.0,
            u_angle: 0.0,
            v_spacing: 52.0_f32.hypot(3.0),
            v_angle: 3.0_f32.atan2(52.0).to_degrees(),
            u_phase: 0.0,
            v_phase: 0.0,
        };
        let (u, v) = lattice.reduced_steps();
        for (actual, expected) in u.into_iter().chain(v).zip([2.0, 3.0, 6.0, -6.0]) {
            assert!((actual - expected).abs() < 1e-3, "{u:?} {v:?}");
        }
    }

    #[test]
    fn symbols_span_at_most_four_shortest_lattice_steps() {
        let layer = |size: f32, paint: SymbolPaint| SymbolLayer {
            lattice: SymbolLattice2d {
                u_spacing: 5.0,
                u_angle: 0.0,
                v_spacing: 50.0,
                v_angle: 90.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: PatternSymbolShape::Circle,
                size,
                rotation: 0.0,
            },
            paint,
        };
        let open = SymbolPaint::Open { stroke_width: 2.0 };
        assert_eq!(layer(400.0, SymbolPaint::Filled).validate(), Ok(()));
        assert_eq!(layer(18.0 * 18.0, open.clone()).validate(), Ok(()));
        assert_eq!(
            layer(18.5 * 18.5, open).validate(),
            Err(PatternValidationError::SymbolOverlap)
        );
    }
}
