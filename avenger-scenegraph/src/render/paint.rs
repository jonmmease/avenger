//! How paint resolves over the box that an item places it in.

use avenger_color::{ColorOrGradient, Gradient, LinearGradient, RadialGradient};
use lyon_path::Path;
use thiserror::Error;

/// Reference rectangle for normalized gradient coordinates, independent of painted coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientBounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl GradientBounds {
    /// Use the unexpanded path bounds for gradient placement.
    pub fn from_path(path: &Path) -> Self {
        let bounds = lyon_algorithms::aabb::bounding_box(path);
        Self {
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
        }
    }

    /// Symbols use their nominal square regardless of shape and rotation.
    pub fn symbol(center: [f32; 2], size: f32) -> Self {
        let half = size.sqrt() / 2.0;
        Self {
            min: [center[0] - half, center[1] - half],
            max: [center[0] + half, center[1] + half],
        }
    }

    /// Radial gradients use a centered enclosing square.
    pub fn radial(self) -> Self {
        let width = self.max[0] - self.min[0];
        let height = self.max[1] - self.min[1];
        let side = width.max(height);
        let min = [
            self.min[0] - (side - width) / 2.0,
            self.min[1] - (side - height) / 2.0,
        ];
        Self {
            min,
            max: [min[0] + side, min[1] + side],
        }
    }
}

/// How a paint draws over a box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedPaint<'a> {
    /// A straight-alpha color.
    Solid([f32; 4]),
    Linear(&'a LinearGradient, GradientBounds),
    /// A radial gradient, over the square that centers on and encloses the box.
    Radial(&'a RadialGradient, GradientBounds),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("gradient index {0} is out of range")]
pub struct GradientIndexError(pub u32);

/// How `paint` draws over `bounds`, or `None` when it draws nothing: a transparent color, a
/// gradient without stops, a radial gradient between identical circles, or a gradient that varies
/// over a box with no area. A gradient with one stop, or a linear gradient whose ends coincide,
/// draws its last stop's color.
pub fn resolve_paint<'a>(
    paint: &ColorOrGradient,
    gradients: &'a [Gradient],
    bounds: GradientBounds,
) -> Result<Option<ResolvedPaint<'a>>, GradientIndexError> {
    let index = match paint {
        ColorOrGradient::Color(color) => {
            return Ok((color[3] > 0.0).then_some(ResolvedPaint::Solid(*color)));
        }
        ColorOrGradient::GradientIndex(index) => *index,
    };
    let gradient = gradients
        .get(index as usize)
        .ok_or(GradientIndexError(index))?;
    let Some(last) = gradient.stops().last() else {
        return Ok(None);
    };
    let constant = gradient.stops().len() == 1
        || matches!(gradient, Gradient::LinearGradient(g) if g.x0 == g.x1 && g.y0 == g.y1);
    if constant {
        return Ok((last.color[3] > 0.0).then_some(ResolvedPaint::Solid(last.color)));
    }
    Ok(match gradient {
        Gradient::LinearGradient(g) => (bounds.min[0] < bounds.max[0]
            && bounds.min[1] < bounds.max[1])
            .then_some(ResolvedPaint::Linear(g, bounds)),
        Gradient::RadialGradient(g) => {
            let square = bounds.radial();
            let identical = g.x0 == g.x1 && g.y0 == g.y1 && g.r0 == g.r1;
            (!identical && square.min[0] < square.max[0])
                .then_some(ResolvedPaint::Radial(g, square))
        }
    })
}

#[cfg(test)]
mod tests {
    use avenger_color::GradientStop;

    use super::*;

    #[test]
    fn degenerate_gradients_paint_their_last_stop_or_nothing() {
        let square = GradientBounds {
            min: [0.0, 0.0],
            max: [10.0, 10.0],
        };
        let flat = GradientBounds {
            min: [0.0, 5.0],
            max: [10.0, 5.0],
        };
        let stops = vec![
            GradientStop {
                offset: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            GradientStop {
                offset: 1.0,
                color: [0.0, 0.0, 1.0, 0.5],
            },
        ];
        let linear = |x1, stops| {
            Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1,
                y1: 0.0,
                stops,
            })
        };
        let last = Some(ResolvedPaint::Solid([0.0, 0.0, 1.0, 0.5]));
        for (gradient, bounds, expected) in [
            (linear(1.0, vec![]), square, None),
            (linear(1.0, stops[1..].to_vec()), square, last),
            (linear(0.0, stops.clone()), square, last),
            (linear(0.0, stops.clone()), flat, last),
            (linear(1.0, stops.clone()), flat, None),
            (
                Gradient::RadialGradient(RadialGradient {
                    x0: 0.5,
                    y0: 0.5,
                    x1: 0.5,
                    y1: 0.5,
                    r0: 0.5,
                    r1: 0.5,
                    stops,
                }),
                square,
                None,
            ),
        ] {
            let paint = ColorOrGradient::GradientIndex(0);
            assert_eq!(
                resolve_paint(&paint, std::slice::from_ref(&gradient), bounds),
                Ok(expected),
                "{gradient:?} over {bounds:?}"
            );
        }
        let missing = ColorOrGradient::GradientIndex(1);
        assert_eq!(
            resolve_paint(&missing, &[], square),
            Err(GradientIndexError(1))
        );
        let transparent = ColorOrGradient::Color([1.0, 0.0, 0.0, 0.0]);
        assert_eq!(resolve_paint(&transparent, &[], square), Ok(None));
    }
}
