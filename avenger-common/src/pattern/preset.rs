//! Named patterns, which the pattern text writes by name with an optional scale.

use super::{
    PatternLayer, PatternLayerKind, PatternLayerOperation, PatternSymbol, PatternSymbolShape,
    StripeDash, StripeLayer, SymbolLattice2d, SymbolLayer, SymbolPaint,
};

/// A named pattern. Its layers repeat every 8 to 16 pixels at scale 1, and ink about a quarter of
/// the area, except the checkerboard, which inks half of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternPreset {
    /// `stripes 45 8 2`: stripes down to the right.
    DiagonalDown,
    /// `stripes -45 8 2`: stripes up to the right.
    DiagonalUp,
    /// `stripes 0 8 2`
    Horizontal,
    /// `stripes 90 8 2`
    Vertical,
    /// `stripes 45 8 1.5 + stripes -45 8 1.5`
    Crosshatch,
    /// `stripes 0 8 1.5 + stripes 90 8 1.5`
    Gridlines,
    /// `stripes 0 6 2 dash 6 3`
    Dashes,
    /// `circle 16 on hex 8`
    Dots,
    /// `circle 36 open 1 on hex 10`
    Rings,
    /// `triangle-up 30 on hex 10`
    Triangles,
    /// `stripes 0 8 4 ^ stripes 90 8 4`
    Checkerboard,
    /// `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`:
    /// mortar lines around bricks 16 pixels wide and 8 tall.
    Bricks,
}

impl PatternPreset {
    pub const ALL: [Self; 12] = [
        Self::DiagonalDown,
        Self::DiagonalUp,
        Self::Horizontal,
        Self::Vertical,
        Self::Crosshatch,
        Self::Gridlines,
        Self::Dashes,
        Self::Dots,
        Self::Rings,
        Self::Triangles,
        Self::Checkerboard,
        Self::Bricks,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::DiagonalDown => "diagonal-down",
            Self::DiagonalUp => "diagonal-up",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Crosshatch => "crosshatch",
            Self::Gridlines => "gridlines",
            Self::Dashes => "dashes",
            Self::Dots => "dots",
            Self::Rings => "rings",
            Self::Triangles => "triangles",
            Self::Checkerboard => "checkerboard",
            Self::Bricks => "bricks",
        }
    }

    /// The preset that `name` names.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// The layers of the preset at `scale`: each length times `scale`, and each symbol size, which
    /// is an area, times its square. A scale keeps the share of the area that the layers ink.
    pub fn layers(self, scale: f32) -> Vec<PatternLayer> {
        let k = scale;
        let stripes = |angle: f32, spacing: f32, width: f32| -> StripeLayer {
            StripeLayer::new(angle, spacing * k, width * k)
        };
        let dashed =
            |angle: f32, spacing: f32, width: f32, dash: [f32; 3], offset: f32| StripeLayer {
                dash: Some(StripeDash {
                    length: dash[0] * k,
                    gap: dash[1] * k,
                    phase: dash[2] * k,
                }),
                phase: offset * k,
                ..stripes(angle, spacing, width)
            };
        let hex =
            |shape: PatternSymbolShape, size: f32, spacing: f32, paint: SymbolPaint| SymbolLayer {
                lattice: SymbolLattice2d {
                    u_spacing: spacing * k,
                    u_angle: 0.0,
                    v_spacing: spacing * k,
                    v_angle: 60.0,
                    u_phase: 0.0,
                    v_phase: 0.0,
                },
                symbol: PatternSymbol {
                    shape,
                    size: size * (k * k),
                    rotation: 0.0,
                },
                paint,
            };
        let xor = |layer: StripeLayer| {
            PatternLayer::from(layer).with_operation(PatternLayerOperation::Xor)
        };
        match self {
            Self::DiagonalDown => vec![stripes(45.0, 8.0, 2.0).into()],
            Self::DiagonalUp => vec![stripes(-45.0, 8.0, 2.0).into()],
            Self::Horizontal => vec![stripes(0.0, 8.0, 2.0).into()],
            Self::Vertical => vec![stripes(90.0, 8.0, 2.0).into()],
            Self::Crosshatch => vec![
                stripes(45.0, 8.0, 1.5).into(),
                stripes(-45.0, 8.0, 1.5).into(),
            ],
            Self::Gridlines => vec![
                stripes(0.0, 8.0, 1.5).into(),
                stripes(90.0, 8.0, 1.5).into(),
            ],
            Self::Dashes => vec![dashed(0.0, 6.0, 2.0, [6.0, 3.0, 0.0], 0.0).into()],
            Self::Dots => {
                vec![hex(PatternSymbolShape::Circle, 16.0, 8.0, SymbolPaint::Filled).into()]
            }
            Self::Rings => vec![hex(
                PatternSymbolShape::Circle,
                36.0,
                10.0,
                SymbolPaint::Open { stroke_width: k },
            )
            .into()],
            Self::Triangles => {
                vec![hex(
                    PatternSymbolShape::TriangleUp,
                    30.0,
                    10.0,
                    SymbolPaint::Filled,
                )
                .into()]
            }
            Self::Checkerboard => vec![stripes(0.0, 8.0, 4.0).into(), xor(stripes(90.0, 8.0, 4.0))],
            Self::Bricks => vec![
                stripes(0.0, 8.0, 1.5).into(),
                dashed(90.0, 16.0, 1.5, [8.0, 8.0, 0.0], 0.0).into(),
                dashed(90.0, 16.0, 1.5, [8.0, 8.0, 8.0], 8.0).into(),
            ],
        }
    }

    /// The preset and the scale whose layers are exactly `layers`, if any.
    pub fn matching(layers: &[PatternLayer]) -> Option<(Self, f32)> {
        Self::ALL.into_iter().find_map(|preset| {
            let at_one = preset.layers(1.0);
            if at_one.len() != layers.len() {
                return None;
            }
            // The first length of the preset gives the scale, up to the rounding of the division.
            let scale = first_length(&layers[0]) / first_length(&at_one[0]);
            [scale, scale.next_down(), scale.next_up()]
                .into_iter()
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .find(|scale| preset.layers(*scale) == layers)
                .map(|scale| (preset, scale))
        })
    }
}

/// The spacing of a layer's stripes or of the first step of its lattice.
fn first_length(layer: &PatternLayer) -> f32 {
    match &layer.kind {
        PatternLayerKind::Stripe(stripe) => stripe.spacing,
        PatternLayerKind::Symbol(symbol) => symbol.lattice.u_spacing,
    }
}
