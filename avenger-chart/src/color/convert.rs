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
/// Used for Lab → LCH and Oklab → Oklch conversions
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
/// Used for LCH → Lab and Oklch → Oklab conversions
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
// sRGB ↔ XYZ Conversion
// ============================================================================

/// sRGB to XYZ-D65 transformation matrix
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.4123907992659595, 0.35758433938387796, 0.1804807884018343],
    [0.21263900587151036, 0.7151686787677559, 0.07219231536073371],
    [0.01933081871559185, 0.11919477979462599, 0.9505321522496606],
];

/// XYZ-D65 to sRGB transformation matrix
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.2409699419045213, -1.5373831775700935, -0.4986107602930033],
    [-0.9692436362808798, 1.8759675015077206, 0.04155505740717561],
    [
        0.05563007969699361,
        -0.20397695888897657,
        1.0569715142428786,
    ],
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
// Oklab ↔ XYZ Conversion
// ============================================================================

/// XYZ-D65 to LMS transformation matrix
const XYZ_TO_LMS: [[f32; 3]; 3] = [
    [0.8190224432164319, 0.3619062562801221, -0.12887378261216414],
    [0.0329836671980271, 0.9292868468965546, 0.03614466816999844],
    [
        0.048177199566046255,
        0.26423952494422764,
        0.6335478258136937,
    ],
];

/// LMS to XYZ-D65 transformation matrix
const LMS_TO_XYZ: [[f32; 3]; 3] = [
    [1.2268798733741557, -0.5578149965554813, 0.28139105017721583],
    [
        -0.04057576262431372,
        1.1122868293970594,
        -0.07171106666151701,
    ],
    [
        -0.07637294974672142,
        -0.4214933239627914,
        1.5869240244272418,
    ],
];

/// LMS to Oklab transformation matrix
const LMS_TO_OKLAB: [[f32; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];

/// Oklab to LMS transformation matrix
const OKLAB_TO_LMS: [[f32; 3]; 3] = [
    [
        0.99999999845051981432,
        0.39633779217376785678,
        0.21580375806075880339,
    ],
    [
        1.0000000088817607767,
        -0.1055613423236563494,
        -0.063854174771705903402,
    ],
    [
        1.0000000546724109177,
        -0.089484182094965759684,
        -1.2914855378640917399,
    ],
];

/// Convert Oklab to XYZ-D65
fn oklab_to_xyz(oklab: &[f32; 3]) -> [f32; 3] {
    // Oklab → LMS
    let lms = matrix_multiply(&OKLAB_TO_LMS, oklab);

    // Cube each component
    let lms = [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)];

    // LMS → XYZ
    matrix_multiply(&LMS_TO_XYZ, &lms)
}

/// Convert XYZ-D65 to Oklab
fn xyz_to_oklab(xyz: &[f32; 3]) -> [f32; 3] {
    // XYZ → LMS
    let lms = matrix_multiply(&XYZ_TO_LMS, xyz);

    // Cube root each component
    let lms = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];

    // LMS → Oklab
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
        // Direct conversions: sRGB ↔ HSL
        (Srgb, Hsl) => crate::color::convert::hsl::rgb_to_hsl(components),
        (Hsl, Srgb) => crate::color::convert::hsl::hsl_to_rgb(components),

        // Polar ↔ Orthogonal conversions
        (Lab, Lch) => orthogonal_to_polar(components, 0.0001),
        (Lch, Lab) => polar_to_orthogonal(components),
        (Oklab, Oklch) => orthogonal_to_polar(components, 0.0001),
        (Oklch, Oklab) => polar_to_orthogonal(components),

        // Via XYZ conversions
        // sRGB → other
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

        // other → sRGB
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

        // HSL → other (via sRGB)
        (Hsl, Lab) | (Hsl, Lch) | (Hsl, Oklab) | (Hsl, Oklch) | (Hsl, Hwb) => {
            let srgb = crate::color::convert::hsl::hsl_to_rgb(components);
            convert_color_space(&srgb, Srgb, to)
        }

        // other → HSL (via sRGB)
        (Lab, Hsl) | (Lch, Hsl) | (Oklab, Hsl) | (Oklch, Hsl) | (Hwb, Hsl) => {
            let srgb = convert_color_space(components, from, Srgb);
            crate::color::convert::hsl::rgb_to_hsl(&srgb)
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

        // Lab/Lch ↔ Oklab/Oklch (via XYZ)
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
// Lab ↔ XYZ Conversion (CIE Lab)
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
    [
        1.0479298208405488,
        0.022946793341019088,
        -0.05019222954313557,
    ],
    [
        0.029627815688159344,
        0.990434484573249,
        -0.01707382502938514,
    ],
    [
        -0.009243058152591178,
        0.015055144896577895,
        0.7518742899580008,
    ],
];

/// XYZ-D50 to XYZ-D65 chromatic adaptation
const XYZ_D50_TO_D65: [[f32; 3]; 3] = [
    [
        0.9554734527042182,
        -0.023098536874261423,
        0.0632593086610217,
    ],
    [
        -0.028369706963208136,
        1.0099954580058226,
        0.020507696433988772,
    ],
    [
        0.012314001688319899,
        -0.020507696433988772,
        1.3303659366080753,
    ],
];

fn xyz_d65_to_d50(xyz: &[f32; 3]) -> [f32; 3] {
    matrix_multiply(&XYZ_D65_TO_D50, xyz)
}

fn xyz_d50_to_d65(xyz: &[f32; 3]) -> [f32; 3] {
    matrix_multiply(&XYZ_D50_TO_D65, xyz)
}

// ============================================================================
// HWB ↔ sRGB Conversion
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
// HSL ↔ sRGB Conversion
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

/// Wrapper module for array-based HSL conversions
pub mod hsl {
    //! HSL color space conversions with array-based interface

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
