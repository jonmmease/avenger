use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::coerce::Coercer;
use datafusion::{
    common::ParamValues,
    prelude::{DataFrame, SessionContext},
};

use super::scale::EvaluatedScale;

pub struct CompilationContext {
    pub ctx: SessionContext,
    pub coercer: Arc<Coercer>,
    pub param_values: ParamValues,
}
