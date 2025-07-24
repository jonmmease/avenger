use crate::coords::{CoordinateSystem, Cartesian, Polar};
use crate::marks::{Mark, MarkConfig, channel::ChannelValue};
use crate::{impl_mark_common, encoding_methods};
use datafusion::dataframe::DataFrame;

pub struct Rect<C: CoordinateSystem> {
    config: MarkConfig<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Common mark methods
impl_mark_common!(Rect, "rect");

// Common encodings for all coordinate systems
impl<C: CoordinateSystem> Rect<C> {
    encoding_methods!(fill, stroke, stroke_width, opacity, corner_radius);
}

// Cartesian-specific encodings
impl Rect<Cartesian> {
    encoding_methods!(x, x2, y, y2);
}

// Polar-specific encodings
impl Rect<Polar> {
    encoding_methods!(r, r2, theta, theta2);
}