use crate::coords::CoordinateSystem;
use crate::legend::Legend;
use crate::plot::Plot;

/// Methods for adding legends to Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Configure a legend by channel name
    pub fn legend<F>(mut self, channel: &str, f: F) -> Self
    where
        F: FnOnce(Legend) -> Legend,
    {
        let current = self.legends.shift_remove(channel).unwrap_or_default();
        let configured = f(current);
        self.legends.insert(channel.to_string(), configured);
        self
    }
}
