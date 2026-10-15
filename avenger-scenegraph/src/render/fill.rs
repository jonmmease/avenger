//! The areas that paths fill, shared by renderers and geometry queries.

use avenger_common::types::FillRule;
use i_overlay::{core::overlay_rule::OverlayRule, float::overlay::FloatOverlay};
use lyon_path::{iterator::PathIterator, math::point, Path, PathEvent};

/// Disjoint polygons, each containing an exterior followed by its interior rings.
pub type FilledContours = Vec<Vec<Vec<[f32; 2]>>>;

/// Resolve a compound path's filled region under the nonzero rule, including intersections and
/// holes.
pub fn filled_contours(path: &Path, tolerance: f32) -> FilledContours {
    let mut contours = Vec::new();
    let mut contour = Vec::new();
    for event in path.iter().flattened(tolerance) {
        match event {
            PathEvent::Begin { at } => contour.push(at.to_array()),
            PathEvent::Line { to, .. } => contour.push(to.to_array()),
            PathEvent::End { .. } => {
                // Overlay contours close implicitly.
                if contour.first() == contour.last() {
                    contour.pop();
                }
                if contour.len() >= 3 {
                    contours.push(std::mem::take(&mut contour));
                }
                contour.clear();
            }
            _ => unreachable!("flattened paths contain only line segments"),
        }
    }
    resolve_contours(&contours, FillRule::NonZero)
}

/// The area that closed contours enclose under `rule`.
pub fn resolve_contours(contours: &[Vec<[f32; 2]>], rule: FillRule) -> FilledContours {
    if contours.is_empty() {
        return Vec::new();
    }
    let rule = match rule {
        FillRule::NonZero => i_overlay::core::fill_rule::FillRule::NonZero,
        FillRule::EvenOdd => i_overlay::core::fill_rule::FillRule::EvenOdd,
    };
    FloatOverlay::with_subj(contours).overlay(OverlayRule::Subject, rule)
}

/// Closed line contours that trace `shapes`.
pub(crate) fn contours_to_path(shapes: &FilledContours) -> Path {
    let mut builder = Path::builder();
    for contour in shapes.iter().flatten() {
        if let Some(first) = contour.first() {
            builder.begin(point(first[0], first[1]));
            for p in &contour[1..] {
                builder.line_to(point(p[0], p[1]));
            }
            builder.close();
        }
    }
    builder.build()
}
