use crate::cartesian::{Cartesian, CartesianAxis};
use crate::plot::Plot;

// Generic implementation for any CartesianAxis type
impl<A> Plot<Cartesian<A>>
where
    A: CartesianAxis + Default + 'static,
{
    // All axis and scale configuration has been moved to channel-level
    // Use mark.x_with() and mark.y_with() for configuring axes and scales
}
