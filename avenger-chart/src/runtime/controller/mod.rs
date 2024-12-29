use std::{fmt::Debug, sync::Arc};

use param_stream::ParamStream;
use crate::param::Param;
use crate::types::mark::Mark;

pub mod pan_zoom;
pub mod param_stream;

pub trait Controller: Debug + Send + Sync + 'static {
    /// controller name that's unique in the group
    fn name(&self) -> &str;

    /// param streams that this controller provides
    fn param_streams(&self) -> Vec<Arc<dyn ParamStream>>;

    /// params that are updated by param_streams, with default values
    fn params(&self) -> Vec<Param>;

    // /// marks that this controller provides
    // fn marks(&self) -> &[Mark] {
    //     &[]
    // }
}
