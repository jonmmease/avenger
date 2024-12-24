use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    common::ParamValues,
    prelude::{DataFrame, SessionContext},
};

use super::scale::EvaluatedScale;

pub struct CompilationContext {
    pub ctx: SessionContext,
    pub dataframes: HashMap<String, DataFrame>,
    pub coerce_scale: Arc<ConfiguredScale>,
    pub scales: HashMap<String, EvaluatedScale>,
    pub params: ParamValues,
}
