//! Color space conversion functions
//!
//! Ported from Mozilla Stylo's style/color/convert.rs
//! https://searchfox.org/mozilla-central/source/servo/components/style/color/convert.rs

use super::types::ColorSpace;

/// Normalize hue into [0, 360) range
#[inline]
pub fn normalize_hue(hue: f32) -> f32 {
    hue - 360.0 * (hue / 360.0).floor()
}

/// Convert from orthogonal (rectangular) to polar (cylindrical) coordinates
///
/// Used for Lab -> LCH and Oklab -> Oklch conversions
///
/// # Arguments
/// * `components` - [L, a, b] in orthogonal space
/// * `epsilon` - Small value for checking if chroma is effectively zero
///
/// # Returns
/// [L, C, H] where:
/// - L: lightness (unchanged)
/// - C: chroma (calculated from a and b)
/// - H: hue in degrees (NaN if chroma is near zero)
pub fn orthogonal_to_polar(components: &[f32; 3], epsilon: f32) -> [f32; 3] {
    let [lightness, a, b] = *components;

    let chroma = (a * a + b * b).sqrt();

    let hue = if a.abs() < epsilon && b.abs() < epsilon {
        // For extremely small values of a and b, hue is undefined
        f32::NAN
    } else if chroma.abs() < epsilon {
        // Very small chroma makes hue meaningless
        f32::NAN
    } else {
        normalize_hue(b.atan2(a).to_degrees())
    };

    [lightness, chroma, hue]
}

/// Convert from polar (cylindrical) to orthogonal (rectangular) coordinates
///
/// Used for LCH -> Lab and Oklch -> Oklab conversions
///
/// # Arguments
/// * `components` - [L, C, H] in polar space where H is in degrees
///
/// # Returns
/// [L, a, b] in orthogonal space
#[inline]
pub fn polar_to_orthogonal(components: &[f32; 3]) -> [f32; 3] {
    let [lightness, chroma, hue] = *components;

    // A missing hue (NaN) results in an achromatic color
    if hue.is_nan() {
        return [lightness, 0.0, 0.0];
    }

    let hue_rad = hue.to_radians();
    let a = chroma * hue_rad.cos();
    let b = chroma * hue_rad.sin();

    [lightness, a, b]
}

/// 3x3 matrix multiplication for color space conversions
///
/// This replaces the dependency on the `euclid` crate's Transform3D
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

/// sRGB to XYZ-D65 transformation matrix
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_390_8, 0.357_584_33, 0.180_480_8],
    [0.212_639, 0.715_168_65, 0.072_192_32],
    [0.019_330_818, 0.119_194_78, 0.950_532_14],
];

/// XYZ-D65 to sRGB transformation matrix
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
    // First convert to linear light
    let linear = [
        gamma_to_linear(srgb[0]),
        gamma_to_linear(srgb[1]),
        gamma_to_linear(srgb[2]),
    ];

    // Then apply matrix transform
    matrix_multiply(&SRGB_TO_XYZ, &linear)
}

/// Convert XYZ-D65 to sRGB
fn xyz_to_srgb(xyz: &[f32; 3]) -> [f32; 3] {
    // Apply matrix transform
    let linear = matrix_multiply(&XYZ_TO_SRGB, xyz);

    // Then convert to gamma-corrected
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
    // Oklab -> LMS
    let lms = matrix_multiply(&OKLAB_TO_LMS, oklab);

    // Cube each component
    let lms = [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)];

    // LMS -> XYZ
    matrix_multiply(&LMS_TO_XYZ, &lms)
}

/// Convert XYZ-D65 to Oklab
fn xyz_to_oklab(xyz: &[f32; 3]) -> [f32; 3] {
    // XYZ -> LMS
    let lms = matrix_multiply(&XYZ_TO_LMS, xyz);

    // Cube root each component
    let lms = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];

    // LMS -> Oklab
    matrix_multiply(&LMS_TO_OKLAB, &lms)
}

// ============================================================================
// High-level color space conversion
// ============================================================================

/// Convert color components from one color space to another
///
/// This is the main entry point for color space conversions
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

        // Catch-all for identity (should be unreachable due to early return)
        _ => *components,
    }
}

// ============================================================================
// Lab <-> XYZ Conversion (CIE Lab)
// ============================================================================

const LAB_KAPPA: f32 = 24389.0 / 27.0; // 903.3
const LAB_EPSILON: f32 = 216.0 / 24389.0; // 0.008856

/// D50 white point
const D50_WHITE: [f32; 3] = [0.9642, 1.0, 0.8251];

/// Convert Lab to XYZ-D50
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

    // Convert from D50 to D65
    let xyz_d50 = [xr * D50_WHITE[0], yr * D50_WHITE[1], zr * D50_WHITE[2]];
    xyz_d50_to_d65(&xyz_d50)
}

/// Convert XYZ-D50 to Lab
fn xyz_to_lab(xyz_d65: &[f32; 3]) -> [f32; 3] {
    // Convert from D65 to D50
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
    [-0.028_369_706, 1.009_995_5, 0.020_507_697],
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
    // First convert to HSL to get hue
    let (hue, _, _) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);

    // Whiteness is the minimum component
    let whiteness = rgb[0].min(rgb[1]).min(rgb[2]);

    // Blackness is 1 - maximum component
    let blackness = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);

    [hue, whiteness * 100.0, blackness * 100.0]
}

/// Convert HWB to sRGB
pub fn hwb_to_rgb(hwb: &[f32; 3]) -> [f32; 3] {
    let [hue, whiteness, blackness] = *hwb;
    let w = whiteness / 100.0;
    let b = blackness / 100.0;

    // If whiteness + blackness >= 1, result is gray
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray, gray, gray];
    }

    // Convert via HSL
    let (r, g, b_comp) = hsl_to_rgb(hue, 100.0, 50.0); // Full saturation, mid lightness
    let mut rgb = [r, g, b_comp];

    // Apply whiteness and blackness
    for component in &mut rgb {
        *component = *component * (1.0 - w - b) + w;
    }

    rgb
}

// ============================================================================
// HSL <-> sRGB Conversion
// ============================================================================

/// Calculate the hue from RGB components and return it along with the min and max RGB values.
///
/// This is a helper function used by both rgb_to_hsl and rgb_to_hwb.
///
/// # Returns
///
/// (hue, min, max) where:
/// - hue is in degrees [0, 360) or NaN if undefined (for achromatic colors)
/// - min is the minimum of (red, green, blue)
/// - max is the maximum of (red, green, blue)
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
        // Achromatic color - hue is undefined, represented as NaN
        f32::NAN
    };

    (hue, min, max)
}

/// Convert from HSL notation to RGB notation.
///
/// # Arguments
///
/// * `hue` - Hue in degrees [0, 360]. Values outside this range will be normalized.
/// * `saturation` - Saturation as a percentage [0, 100]
/// * `lightness` - Lightness as a percentage [0, 100]
///
/// # Returns
///
/// (red, green, blue) where each component is in the range [0.0, 1.0]
///
/// # Reference
///
/// https://drafts.csswg.org/css-color-4/#hsl-to-rgb
#[inline]
pub fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (f32, f32, f32) {
    /// Helper function for HSL to RGB conversion
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

    // Normalize hue (handle NaN by converting to 0.0)
    let hue = if hue.is_nan() { 0.0 } else { hue };

    // Convert saturation and lightness from percentage to [0, 1]
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

/// Convert from RGB notation to HSL notation.
///
/// # Arguments
///
/// * `red` - Red component in the range [0.0, 1.0]
/// * `green` - Green component in the range [0.0, 1.0]
/// * `blue` - Blue component in the range [0.0, 1.0]
///
/// # Returns
///
/// (hue, saturation, lightness) where:
/// - hue is in degrees [0, 360) or NaN for achromatic colors
/// - saturation is a percentage [0, 100]
/// - lightness is a percentage [0, 100]
///
/// # Reference
///
/// https://drafts.csswg.org/css-color-4/#rgb-to-hsl
#[inline]
pub fn rgb_to_hsl(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
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

/// HSL color space conversions with array-based interface.
pub mod hsl {
    /// Convert HSL to RGB (array version)
    pub fn hsl_to_rgb(hsl: &[f32; 3]) -> [f32; 3] {
        let (r, g, b) = super::hsl_to_rgb(hsl[0], hsl[1], hsl[2]);
        [r, g, b]
    }

    /// Convert RGB to HSL (array version)
    pub fn rgb_to_hsl(rgb: &[f32; 3]) -> [f32; 3] {
        let (h, s, l) = super::rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
        [h, s, l]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_hue() {
        assert_eq!(normalize_hue(0.0), 0.0);
        assert_eq!(normalize_hue(180.0), 180.0);
        assert_eq!(normalize_hue(360.0), 0.0);
        assert_eq!(normalize_hue(450.0), 90.0);
        assert_eq!(normalize_hue(-90.0), 270.0);
    }

    #[test]
    fn test_polar_orthogonal_roundtrip() {
        let ortho = [50.0, 25.0, -30.0];
        let polar = orthogonal_to_polar(&ortho, 0.0001);
        let back = polar_to_orthogonal(&polar);

        assert!((ortho[0] - back[0]).abs() < 0.01);
        assert!((ortho[1] - back[1]).abs() < 0.01);
        assert!((ortho[2] - back[2]).abs() < 0.01);
    }

    #[test]
    fn test_srgb_xyz_roundtrip() {
        let srgb = [0.5, 0.3, 0.8];
        let xyz = srgb_to_xyz(&srgb);
        let back = xyz_to_srgb(&xyz);

        assert!((srgb[0] - back[0]).abs() < 0.001);
        assert!((srgb[1] - back[1]).abs() < 0.001);
        assert!((srgb[2] - back[2]).abs() < 0.001);
    }

    #[test]
    fn test_oklab_xyz_roundtrip() {
        let oklab = [0.5, 0.1, -0.1];
        let xyz = oklab_to_xyz(&oklab);
        let back = xyz_to_oklab(&xyz);

        assert!((oklab[0] - back[0]).abs() < 0.001);
        assert!((oklab[1] - back[1]).abs() < 0.001);
        assert!((oklab[2] - back[2]).abs() < 0.001);
    }

    #[test]
    fn test_srgb_to_oklab() {
        // Red in sRGB should convert to Oklab
        let red_srgb = [1.0, 0.0, 0.0];
        let oklab = convert_color_space(&red_srgb, ColorSpace::Srgb, ColorSpace::Oklab);
        // Oklab red is approximately [0.628, 0.225, 0.126]
        assert!((oklab[0] - 0.628).abs() < 0.01);
        assert!((oklab[1] - 0.225).abs() < 0.01);
        assert!((oklab[2] - 0.126).abs() < 0.01);
    }
}
