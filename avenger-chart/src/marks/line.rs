use datafusion::dataframe::DataFrame;
use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::marks::{Mark, MarkConfig, channel::ChannelValue};
use crate::{impl_mark_common, encoding_methods};

pub struct Line<C: CoordinateSystem> {
    config: MarkConfig<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Line, "line");

// Line-specific encoding methods
impl<C: CoordinateSystem> Line<C> {
    encoding_methods!(stroke, stroke_width, stroke_dash, opacity, interpolate);
}

// Cartesian-specific position methods
impl Line<Cartesian> {
    encoding_methods!(x, y);
}

// Polar-specific position methods
impl Line<Polar> {
    encoding_methods!(r, theta);
}