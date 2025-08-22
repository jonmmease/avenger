use crate::coords::CoordinateSystem;
use crate::marks::MarkState;

/// Base struct for Symbol mark
#[allow(dead_code)]
pub struct Symbol<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) __phantom: std::marker::PhantomData<C>,
}

/// Base struct for Line mark
#[allow(dead_code)]
pub struct Line<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) __phantom: std::marker::PhantomData<C>,
}

/// Base struct for Rect mark
#[allow(dead_code)]
pub struct Rect<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) __phantom: std::marker::PhantomData<C>,
}
