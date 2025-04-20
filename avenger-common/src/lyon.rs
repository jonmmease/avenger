use lyon_extra::euclid::UnknownUnit;
use lyon_extra::parser::{ParseError, ParserOptions, Source};
use lyon_path::geom::euclid::Point2D;
use lyon_path::PathEvent;
use lyon_path::{Path, Winding};
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
        match &self.value {
            ScalarOrArrayValue::Scalar(path) => hash_lyon_path(path, state),
            ScalarOrArrayValue::Array(paths) => {
                paths.iter().for_each(|path| hash_lyon_path(path, state));
            }
        }
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

