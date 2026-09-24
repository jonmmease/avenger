//! Conversions between the supported [`ColorSpace`] variants.
//!
//! Lab and LCH use D50. Conversions through XYZ use D65, with Bradford adaptation
//! between the two white points. Component values are not clipped.

// Ported from Mozilla Stylo's style/color/convert.rs
// https://searchfox.org/mozilla-central/source/servo/components/style/color/convert.rs

use super::types::ColorSpace;

/// Normalize a hue in degrees to [0, 360). Non-finite inputs produce `NaN`.
#[inline]
pub(super) fn normalize_hue(hue: f32) -> f32 {
    hue - 360.0 * (hue / 360.0).floor()
}

/// Convert Lab or Oklab `[L, a, b]` to cylindrical `[L, C, H]`.
///
/// Lightness is unchanged and hue is in degrees. Hue is `NaN` when both `a` and
/// `b` have absolute values below `epsilon`, or chroma is below `epsilon`.
fn orthogonal_to_polar(components: &[f32; 3], epsilon: f32) -> [f32; 3] {
    let [lightness, a, b] = *components;

    let chroma = (a * a + b * b).sqrt();

    let hue = if (a.abs() < epsilon && b.abs() < epsilon) || chroma.abs() < epsilon {
        f32::NAN
    } else {
        normalize_hue(b.atan2(a).to_degrees())
    };

    [lightness, chroma, hue]
}

/// Convert LCH or Oklch `[L, C, H]` to rectangular `[L, a, b]`.
///
/// Hue is in degrees. A `NaN` hue produces `[L, 0, 0]`.
#[inline]
fn polar_to_orthogonal(components: &[f32; 3]) -> [f32; 3] {
    let [lightness, chroma, hue] = *components;

    if hue.is_nan() {
        return [lightness, 0.0, 0.0];
    }

    let hue_rad = hue.to_radians();
    let a = chroma * hue_rad.cos();
    let b = chroma * hue_rad.sin();

    [lightness, a, b]
}

/// Multiply a 3×3 color-conversion matrix by a three-component vector.
#[inline]
fn matrix_multiply(matrix: &[[f32; 3]; 3], vector: &[f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
}

// ============================================================================
// sRGB <-> XYZ Conversion
// ============================================================================

/// Linear sRGB to XYZ-D65 transformation matrix.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_390_8, 0.357_584_33, 0.180_480_8],
    [0.212_639, 0.715_168_65, 0.072_192_32],
    [0.019_330_818, 0.119_194_78, 0.950_532_14],
];

/// XYZ-D65 to linear sRGB transformation matrix.
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.240_97, -1.537_383_2, -0.498_610_76],
    [-0.969_243_65, 1.875_967_5, 0.041_555_06],
    [0.055_630_08, -0.203_976_96, 1.056_971_5],
];

/// Convert linear sRGB component to gamma-corrected sRGB
#[inline]
fn linear_to_gamma(value: f32) -> f32 {
    let abs = value.abs();
    if abs > 0.0031308 {
        value.signum() * (1.055 * abs.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * value
    }
}

/// Convert gamma-corrected sRGB component to linear sRGB
#[inline]
fn gamma_to_linear(value: f32) -> f32 {
    let abs = value.abs();
    if abs < 0.04045 {
        value / 12.92
    } else {
        value.signum() * ((abs + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert sRGB to XYZ-D65
fn srgb_to_xyz(srgb: &[f32; 3]) -> [f32; 3] {
    let linear = [
        gamma_to_linear(srgb[0]),
        gamma_to_linear(srgb[1]),
        gamma_to_linear(srgb[2]),
    ];

    matrix_multiply(&SRGB_TO_XYZ, &linear)
}

/// Convert XYZ-D65 to sRGB
fn xyz_to_srgb(xyz: &[f32; 3]) -> [f32; 3] {
    let linear = matrix_multiply(&XYZ_TO_SRGB, xyz);

    [
        linear_to_gamma(linear[0]),
        linear_to_gamma(linear[1]),
        linear_to_gamma(linear[2]),
    ]
}

// ============================================================================
// Oklab <-> XYZ Conversion
// ============================================================================

/// XYZ-D65 to LMS transformation matrix
const XYZ_TO_LMS: [[f32; 3]; 3] = [
    [0.819_022_4, 0.361_906_26, -0.128_873_78],
    [0.032_983_67, 0.929_286_84, 0.036_144_666],
    [0.048_177_2, 0.264_239_52, 0.633_547_84],
];

/// LMS to XYZ-D65 transformation matrix
const LMS_TO_XYZ: [[f32; 3]; 3] = [
    [1.226_879_8, -0.557_815, 0.281_391_05],
    [-0.040_575_76, 1.112_286_8, -0.071_711_06],
    [-0.076_372_95, -0.421_493_32, 1.586_924_1],
];

/// LMS to Oklab transformation matrix
const LMS_TO_OKLAB: [[f32; 3]; 3] = [
    [0.210_454_26, 0.793_617_8, -0.004_072_047],
    [1.977_998_5, -2.428_592_2, 0.450_593_7],
    [0.025_904_037, 0.782_771_77, -0.808_675_77],
];

/// Oklab to LMS transformation matrix
const OKLAB_TO_LMS: [[f32; 3]; 3] = [
    [0.99999999845051981432, 0.396_337_78, 0.215_803_76],
    [1.0000000088817607767, -0.105_561_346, -0.063_854_17],
    [1.0000000546724109177, -0.089_484_185, -1.291_485_5],
];

/// Convert Oklab to XYZ-D65
fn oklab_to_xyz(oklab: &[f32; 3]) -> [f32; 3] {
    let lms = matrix_multiply(&OKLAB_TO_LMS, oklab);

    let lms = [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)];

    matrix_multiply(&LMS_TO_XYZ, &lms)
}

/// Convert XYZ-D65 to Oklab
fn xyz_to_oklab(xyz: &[f32; 3]) -> [f32; 3] {
    let lms = matrix_multiply(&XYZ_TO_LMS, xyz);

    let lms = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];

    matrix_multiply(&LMS_TO_OKLAB, &lms)
}

// ============================================================================
// High-level color space conversion
// ============================================================================

/// Convert three components from `from` to `to` without clipping.
///
/// Component order and units are defined by [`ColorSpace`]. Identical source and
/// target spaces return the input unchanged. For colors with alpha, use
/// [`AbsoluteColor::to_color_space`](crate::AbsoluteColor::to_color_space).
pub fn convert_color_space(components: &[f32; 3], from: ColorSpace, to: ColorSpace) -> [f32; 3] {
    if from == to {
        return *components;
    }

    use ColorSpace::*;

    match (from, to) {
        // Direct conversions: sRGB <-> HSL
        (Srgb, Hsl) => hsl::rgb_to_hsl(components),
        (Hsl, Srgb) => hsl::hsl_to_rgb(components),

        // Polar <-> Orthogonal conversions
        (Lab, Lch) => orthogonal_to_polar(components, 0.0001),
        (Lch, Lab) => polar_to_orthogonal(components),
        (Oklab, Oklch) => orthogonal_to_polar(components, 0.0001),
        (Oklch, Oklab) => polar_to_orthogonal(components),

        // Via XYZ conversions
        // sRGB -> other
        (Srgb, Lab) => xyz_to_lab(&srgb_to_xyz(components)),
        (Srgb, Lch) => {
            let lab = xyz_to_lab(&srgb_to_xyz(components));
            orthogonal_to_polar(&lab, 0.0001)
        }
        (Srgb, Oklab) => xyz_to_oklab(&srgb_to_xyz(components)),
        (Srgb, Oklch) => {
            let oklab = xyz_to_oklab(&srgb_to_xyz(components));
            orthogonal_to_polar(&oklab, 0.0001)
        }

        // other -> sRGB
        (Lab, Srgb) => xyz_to_srgb(&lab_to_xyz(components)),
        (Lch, Srgb) => {
            let lab = polar_to_orthogonal(components);
            xyz_to_srgb(&lab_to_xyz(&lab))
        }
        (Oklab, Srgb) => xyz_to_srgb(&oklab_to_xyz(components)),
        (Oklch, Srgb) => {
            let oklab = polar_to_orthogonal(components);
            xyz_to_srgb(&oklab_to_xyz(&oklab))
        }

        // HSL -> other (via sRGB)
        (Hsl, Lab) | (Hsl, Lch) | (Hsl, Oklab) | (Hsl, Oklch) | (Hsl, Hwb) => {
            let srgb = hsl::hsl_to_rgb(components);
            convert_color_space(&srgb, Srgb, to)
        }

        // other -> HSL (via sRGB)
        (Lab, Hsl) | (Lch, Hsl) | (Oklab, Hsl) | (Oklch, Hsl) | (Hwb, Hsl) => {
            let srgb = convert_color_space(components, from, Srgb);
            hsl::rgb_to_hsl(&srgb)
        }

        // HWB conversions (via sRGB)
        (Srgb, Hwb) => rgb_to_hwb(components),
        (Hwb, Srgb) => hwb_to_rgb(components),
        (Hwb, Lab) | (Hwb, Lch) | (Hwb, Oklab) | (Hwb, Oklch) => {
            let srgb = hwb_to_rgb(components);
            convert_color_space(&srgb, Srgb, to)
        }
        (Lab, Hwb) | (Lch, Hwb) | (Oklab, Hwb) | (Oklch, Hwb) => {
            let srgb = convert_color_space(components, from, Srgb);
            rgb_to_hwb(&srgb)
        }

        // Lab/Lch <-> Oklab/Oklch (via XYZ)
        (Lab, Oklab) | (Lab, Oklch) => {
            let xyz = lab_to_xyz(components);
            let oklab = xyz_to_oklab(&xyz);
            if to == Oklch {
                orthogonal_to_polar(&oklab, 0.0001)
            } else {
                oklab
            }
        }
        (Lch, Oklab) | (Lch, Oklch) => {
            let lab = polar_to_orthogonal(components);
            let xyz = lab_to_xyz(&lab);
            let oklab = xyz_to_oklab(&xyz);
            if to == Oklch {
                orthogonal_to_polar(&oklab, 0.0001)
            } else {
                oklab
            }
        }
        (Oklab, Lab) | (Oklab, Lch) => {
            let xyz = oklab_to_xyz(components);
            let lab = xyz_to_lab(&xyz);
            if to == Lch {
                orthogonal_to_polar(&lab, 0.0001)
            } else {
                lab
            }
        }
        (Oklch, Lab) | (Oklch, Lch) => {
            let oklab = polar_to_orthogonal(components);
            let xyz = oklab_to_xyz(&oklab);
            let lab = xyz_to_lab(&xyz);
            if to == Lch {
                orthogonal_to_polar(&lab, 0.0001)
            } else {
                lab
            }
        }

        // Equal source and target spaces return before the match.
        _ => *components,
    }
}

// ============================================================================
// Lab <-> XYZ Conversion (CIE Lab)
// ============================================================================

const LAB_KAPPA: f32 = 24389.0 / 27.0; // 903.3
const LAB_EPSILON: f32 = 216.0 / 24389.0; // 0.008856

/// CSS D50 white point, consistent with the Bradford adaptation matrices below.
/// <https://drafts.csswg.org/css-color-4/#color-conversion-code>
const D50_WHITE: [f32; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

/// Convert D50 Lab to XYZ-D65, including chromatic adaptation.
fn lab_to_xyz(lab: &[f32; 3]) -> [f32; 3] {
    let [l, a, b] = *lab;

    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;

    let xr = if fx.powi(3) > LAB_EPSILON {
        fx.powi(3)
    } else {
        (116.0 * fx - 16.0) / LAB_KAPPA
    };

    let yr = if l > LAB_KAPPA * LAB_EPSILON {
        fy.powi(3)
    } else {
        l / LAB_KAPPA
    };

    let zr = if fz.powi(3) > LAB_EPSILON {
        fz.powi(3)
    } else {
        (116.0 * fz - 16.0) / LAB_KAPPA
    };

    let xyz_d50 = [xr * D50_WHITE[0], yr * D50_WHITE[1], zr * D50_WHITE[2]];
    xyz_d50_to_d65(&xyz_d50)
}

/// Convert XYZ-D65 to D50 Lab, including chromatic adaptation.
fn xyz_to_lab(xyz_d65: &[f32; 3]) -> [f32; 3] {
    let xyz = xyz_d65_to_d50(xyz_d65);

    let xr = xyz[0] / D50_WHITE[0];
    let yr = xyz[1] / D50_WHITE[1];
    let zr = xyz[2] / D50_WHITE[2];

    let fx = if xr > LAB_EPSILON {
        xr.cbrt()
    } else {
        (LAB_KAPPA * xr + 16.0) / 116.0
    };

    let fy = if yr > LAB_EPSILON {
        yr.cbrt()
    } else {
        (LAB_KAPPA * yr + 16.0) / 116.0
    };

    let fz = if zr > LAB_EPSILON {
        zr.cbrt()
    } else {
        (LAB_KAPPA * zr + 16.0) / 116.0
    };

    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);

    [l, a, b]
}

/// XYZ-D65 to XYZ-D50 chromatic adaptation
const XYZ_D65_TO_D50: [[f32; 3]; 3] = [
    [1.047_929_8, 0.022_946_794, -0.050_192_23],
    [0.029_627_815, 0.990_434_47, -0.017_073_825],
    [-0.009_243_058, 0.015_055_145, 0.751_874_27],
];

/// XYZ-D50 to XYZ-D65 chromatic adaptation
const XYZ_D50_TO_D65: [[f32; 3]; 3] = [
    [0.955_473_4, -0.023_098_538, 0.063_259_31],
    [-0.028_369_706, 1.009_995_5, 0.021_041_442],
    [0.012_314_002, -0.020_507_697, 1.330_365_9],
];

fn xyz_d65_to_d50(xyz: &[f32; 3]) -> [f32; 3] {
    matrix_multiply(&XYZ_D65_TO_D50, xyz)
}

fn xyz_d50_to_d65(xyz: &[f32; 3]) -> [f32; 3] {
    matrix_multiply(&XYZ_D50_TO_D65, xyz)
}

// ============================================================================
// HWB <-> sRGB Conversion
// ============================================================================

/// Convert sRGB to HWB
fn rgb_to_hwb(rgb: &[f32; 3]) -> [f32; 3] {
    let (hue, _, _) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);

    let whiteness = rgb[0].min(rgb[1]).min(rgb[2]);

    let blackness = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);

    [hue, whiteness * 100.0, blackness * 100.0]
}

/// Convert `[hue, whiteness, blackness]` to sRGB `[r, g, b]`.
///
/// Hue is in degrees and whiteness and blackness are percentages. When their sum
/// is at least 100, the result is gray with value `whiteness / (whiteness + blackness)`.
fn hwb_to_rgb(hwb: &[f32; 3]) -> [f32; 3] {
    let [hue, whiteness, blackness] = *hwb;
    let w = whiteness / 100.0;
    let b = blackness / 100.0;

    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray, gray, gray];
    }

    let (r, g, b_comp) = hsl_to_rgb(hue, 100.0, 50.0); // Full saturation, mid lightness
    let mut rgb = [r, g, b_comp];

    for component in &mut rgb {
        *component = *component * (1.0 - w - b) + w;
    }

    rgb
}

// ============================================================================
// HSL <-> sRGB Conversion
// ============================================================================

/// Return hue in degrees and the minimum and maximum RGB components.
///
/// Achromatic colors have `NaN` hue.
#[inline]
fn rgb_to_hue_min_max(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);

    let delta = max - min;

    let hue = if delta != 0.0 {
        60.0 * if max == red {
            (green - blue) / delta + if green < blue { 6.0 } else { 0.0 }
        } else if max == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        }
    } else {
        f32::NAN
    };

    (hue, min, max)
}

/// Convert HSL to sRGB `(red, green, blue)`.
///
/// Hue is in degrees and is normalized to [0, 360). A `NaN` hue is treated as zero.
/// Saturation and lightness are percentages. Inputs in [0, 100] produce RGB
/// components in [0, 1].
///
/// <https://drafts.csswg.org/css-color-4/#hsl-to-rgb>
#[inline]
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (f32, f32, f32) {
    fn hue_to_rgb(t1: f32, t2: f32, hue: f32) -> f32 {
        let hue = normalize_hue(hue);

        if hue * 6.0 < 360.0 {
            t1 + (t2 - t1) * hue / 60.0
        } else if hue * 2.0 < 360.0 {
            t2
        } else if hue * 3.0 < 720.0 {
            t1 + (t2 - t1) * (240.0 - hue) / 60.0
        } else {
            t1
        }
    }

    let hue = if hue.is_nan() { 0.0 } else { hue };

    let saturation = saturation / 100.0;
    let lightness = lightness / 100.0;

    let t2 = if lightness <= 0.5 {
        lightness * (saturation + 1.0)
    } else {
        lightness + saturation - lightness * saturation
    };
    let t1 = lightness * 2.0 - t2;

    (
        hue_to_rgb(t1, t2, hue + 120.0),
        hue_to_rgb(t1, t2, hue),
        hue_to_rgb(t1, t2, hue - 120.0),
    )
}

/// Convert sRGB components in [0, 1] to `(hue, saturation, lightness)`.
///
/// Hue is in [0, 360) degrees, or `NaN` for achromatic colors. Saturation and
/// lightness are percentages.
///
/// <https://drafts.csswg.org/css-color-4/#rgb-to-hsl>
#[inline]
fn rgb_to_hsl(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let (hue, min, max) = rgb_to_hue_min_max(red, green, blue);

    let lightness = (min + max) / 2.0;
    let delta = max - min;

    let saturation = if delta != 0.0 {
        if lightness == 0.0 || lightness == 1.0 {
            0.0
        } else {
            (max - lightness) / lightness.min(1.0 - lightness)
        }
    } else {
        0.0
    };

    (hue, saturation * 100.0, lightness * 100.0)
}

/// Array interfaces for HSL and sRGB conversion.
mod hsl {
    /// Array form of [`super::hsl_to_rgb`], with the same units and hue handling.
    pub(super) fn hsl_to_rgb(hsl: &[f32; 3]) -> [f32; 3] {
        let (r, g, b) = super::hsl_to_rgb(hsl[0], hsl[1], hsl[2]);
        [r, g, b]
    }

    /// Array form of [`super::rgb_to_hsl`], with the same units and undefined hue handling.
    pub(super) fn rgb_to_hsl(rgb: &[f32; 3]) -> [f32; 3] {
        let (h, s, l) = super::rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
        [h, s, l]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AbsoluteColor;

    #[test]
    fn conversions_match_independent_reference_vectors() {
        // CSS Color 4 sample code, evaluated in f64 outside this crate:
        // https://www.w3.org/TR/css-color-4/#color-conversion-code
        // Each fixture represents one color in every space, covering all source/target pairs.
        for components in [
            [
                [0.5, 0.3, 0.8],
                [264.0, 55.55556, 55.0],
                [264.0, 30.0, 20.0],
                [43.76037, 42.2445, -59.30758],
                [43.76037, 72.81474, 305.4621],
                [0.5417581, 0.0894513, -0.1665469],
                [0.5417581, 0.1890487, 298.23996],
            ],
            [
                [1.0, 0.0, 0.0],
                [0.0, 100.0, 50.0],
                [0.0, 0.0, 0.0],
                [54.29054, 80.80493, 69.89096],
                [54.29054, 106.83718, 40.85766],
                [0.6279554, 0.2248631, 0.1258463],
                [0.6279554, 0.2576833, 29.23388],
            ],
        ] {
            let spaces = [
                ColorSpace::Srgb,
                ColorSpace::Hsl,
                ColorSpace::Hwb,
                ColorSpace::Lab,
                ColorSpace::Lch,
                ColorSpace::Oklab,
                ColorSpace::Oklch,
            ];
            for (source, input) in spaces.into_iter().zip(components) {
                let color = AbsoluteColor::new(source, input[0], input[1], input[2], 0.37);
                for (target, expected) in spaces.into_iter().zip(components) {
                    let actual = color.to_color_space(target);
                    assert_eq!(actual.color_space, target);
                    assert_eq!(actual.alpha, color.alpha);
                    for (i, expected) in expected.into_iter().enumerate() {
                        let difference = (actual.components[i] - expected).abs();
                        let difference = if target.hue_index() == Some(i) {
                            difference.min((difference - 360.0).abs())
                        } else {
                            difference
                        };
                        let tolerance = if matches!(target, ColorSpace::Srgb | ColorSpace::Oklab) {
                            1e-5
                        } else {
                            5e-4
                        };
                        assert!(
                            difference < tolerance,
                            "{source:?} -> {target:?}: {actual:?}, expected {expected} at {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn lab_d50_preserves_neutral_colors() {
        // CSS D50 Lab reference values: L*=50 encodes as this sRGB gray.
        for (lightness, gray) in [(0.0, 0.0), (50.0, 0.4663266), (100.0, 1.0)] {
            let lab = convert_color_space(&[gray; 3], ColorSpace::Srgb, ColorSpace::Lab);
            for (actual, expected) in lab.into_iter().zip([lightness, 0.0, 0.0]) {
                assert!((actual - expected).abs() < 1e-4, "{lab:?}");
            }
            let srgb =
                convert_color_space(&[lightness, 0.0, 0.0], ColorSpace::Lab, ColorSpace::Srgb);
            for actual in srgb {
                assert!((actual - gray).abs() < 1e-5, "{srgb:?}");
            }
        }
    }

    #[test]
    fn public_roundtrips_preserve_srgb_and_alpha() {
        for rgba in [
            [1.0, 1.0, 1.0, 0.37],
            [1.0, 0.0, 0.0, 0.0004],
            [0.5, 0.3, 0.8, 0.999995],
        ] {
            let original = AbsoluteColor::from_rgba(rgba);
            for space in [
                ColorSpace::Hsl,
                ColorSpace::Hwb,
                ColorSpace::Lab,
                ColorSpace::Lch,
                ColorSpace::Oklab,
                ColorSpace::Oklch,
            ] {
                let actual = original
                    .to_color_space(space)
                    .to_color_space(ColorSpace::Srgb);
                assert_eq!(actual.alpha, original.alpha);
                for (actual, expected) in actual.components.into_iter().zip(original.components) {
                    assert!(
                        (actual - expected).abs() < 1e-5,
                        "{space:?}: {actual} != {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn hwb_normalizes_whiteness_and_blackness_to_gray() {
        for (white, black, gray) in [(25.0, 75.0, 0.25), (80.0, 40.0, 2.0 / 3.0)] {
            for hue in [0.0, 240.0] {
                let color = AbsoluteColor::new(ColorSpace::Hwb, hue, white, black, 0.37);
                let actual = color.to_color_space(ColorSpace::Srgb);
                assert_eq!(actual.alpha, 0.37);
                for channel in actual.components {
                    assert!((channel - gray).abs() < 1e-6, "{color:?}: {actual:?}");
                }
            }
        }
    }
}
