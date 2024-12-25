use std::{collections::HashMap, sync::Arc};

use avenger_scales2::scales::coerce::Coercer;
use datafusion::{
    common::ParamValues,
    prelude::{DataFrame, SessionContext},
};

use super::scale::EvaluatedScale;

pub struct CompilationContext {
    pub ctx: SessionContext,
    pub dataframes: HashMap<String, DataFrame>,
    pub coercer: Arc<Coercer>,
    pub scales: HashMap<String, EvaluatedScale>,
    pub params: ParamValues,
}
