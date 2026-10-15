use lyon_extra::euclid::UnknownUnit;
use lyon_extra::parser::{ParseError, ParserOptions, Source};
use lyon_path::geom::euclid::Point2D;
use lyon_path::Path;
use lyon_path::PathEvent;
use ordered_float::OrderedFloat;
use std::hash::{Hash, Hasher};

use crate::value::{ScalarOrArray, ScalarOrArrayValue};

pub fn hash_point<H: Hasher>(point: &Point2D<f32, UnknownUnit>, hasher: &mut H) {
    OrderedFloat::from(point.x).hash(hasher);
    OrderedFloat::from(point.y).hash(hasher);
}

pub fn hash_lyon_path<H: Hasher>(path: &Path, hasher: &mut H) {
    for evt in path.iter() {
        // hash enum variant
        let variant = std::mem::discriminant(&evt);
        variant.hash(hasher);

        // hash enum value
        match evt {
            PathEvent::Begin { at } => hash_point(&at, hasher),
            PathEvent::Line { from, to, .. } => {
                hash_point(&from, hasher);
                hash_point(&to, hasher);
            }
            PathEvent::End { last, first, close } => {
                hash_point(&last, hasher);
                hash_point(&first, hasher);
                close.hash(hasher);
            }
            PathEvent::Quadratic { from, ctrl, to, .. } => {
                hash_point(&from, hasher);
                hash_point(&ctrl, hasher);
                hash_point(&to, hasher);
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                hash_point(&from, hasher);
                hash_point(&ctrl1, hasher);
                hash_point(&ctrl2, hasher);
                hash_point(&to, hasher);
            }
        }
    }
}

impl Hash for ScalarOrArray<lyon_path::Path> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_cached(state, hash_lyon_path);
    }
}

/// Whether two paths have the same events, as `hash_lyon_path` hashes them.
pub fn lyon_paths_equal(a: &Path, b: &Path) -> bool {
    a.iter().eq(b.iter())
}

/// Whether two path channels have the same paths, the same way.
pub fn lyon_path_values_equal(a: &ScalarOrArray<Path>, b: &ScalarOrArray<Path>) -> bool {
    match (&a.value, &b.value) {
        (ScalarOrArrayValue::Scalar(a), ScalarOrArrayValue::Scalar(b)) => lyon_paths_equal(a, b),
        (ScalarOrArrayValue::Array(a), ScalarOrArrayValue::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| lyon_paths_equal(a, b))
        }
        _ => false,
    }
}

pub fn parse_svg_path(path: &str) -> Result<Path, ParseError> {
    let mut source = Source::new(path.chars());
    let mut parser = lyon_extra::parser::PathParser::new();
    let opts = ParserOptions::DEFAULT;
    let mut builder = lyon_path::Path::builder();
    parser.parse(&opts, &mut source, &mut builder)?;
    Ok(builder.build())
}
