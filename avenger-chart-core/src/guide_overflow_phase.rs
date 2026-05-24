/// Which overflow contract a guide should use while measuring frame demand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuideOverflowPhase {
    /// Initial/local measurement before facet coordination has produced a contract.
    Measurement,
    /// Final realization after facet coordination has produced a contract.
    Final,
}
