use crate::plot::Plot;
use crate::polar::PolarAxis;
use crate::polar::coord::PolarGeneral;

// Generic implementation for any PolarAxis type
impl<A> Plot<PolarGeneral<A>>
where
    A: PolarAxis + Default + 'static,
{
    // All axis and scale configuration has been moved to channel-level
    // Use mark.r_with() and mark.theta_with() for configuring axes and scales
}

// The type alias Polar = PolarGeneral<DefaultPolarAxis> makes Plot<Polar> work automatically
