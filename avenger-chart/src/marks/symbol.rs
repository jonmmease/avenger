use datafusion::dataframe::DataFrame;
use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::marks::{Mark, MarkConfig, channel::ChannelValue};
use crate::{impl_mark_common, encoding_methods};

pub struct Symbol<C: CoordinateSystem> {
    config: MarkConfig<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Symbol, "symbol");

// Symbol-specific encoding methods
impl<C: CoordinateSystem> Symbol<C> {
    encoding_methods!(size, fill, stroke, stroke_width, opacity, shape);
}

// Cartesian-specific position methods
impl Symbol<Cartesian> {
    encoding_methods!(x, y);
}

// Polar-specific position methods
impl Symbol<Polar> {
    encoding_methods!(r, theta);
}