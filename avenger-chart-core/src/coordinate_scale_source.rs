use std::{collections::HashMap, sync::Arc};

use datafusion::dataframe::DataFrame;
use serde::{Deserialize, Serialize};

use crate::{
    AvengerChartError, Axis, CompiledDataContext, DataContext, FacetDataScope, MarkDataMode,
    RepeatContext,
};

/// Non-rendered coordinate-owned scale/domain source.
///
/// Coordinate systems use these sources when their scales are declared on the
/// coordinate itself rather than on a rendered mark. The source participates in
/// scale, domain, and guide planning, but it does not render scene marks or
/// expose public mark targets.
#[derive(Clone)]
pub struct CoordinateScaleSource {
    pub data: DataContext,
    pub data_mode: MarkDataMode,
    pub facet_data_scope: FacetDataScope,
    pub exclude_from_scale_domains: bool,
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

impl Default for CoordinateScaleSource {
    fn default() -> Self {
        Self {
            data: DataContext::default(),
            data_mode: MarkDataMode::Inherit,
            facet_data_scope: FacetDataScope::FILTERED,
            exclude_from_scale_domains: false,
            axis_configs: HashMap::new(),
        }
    }
}

impl CoordinateScaleSource {
    pub fn new(data: DataContext) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }

    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        let mut resolved = self.clone();
        resolved.data = self.data.resolve_repeat(ctx)?;
        resolved.axis_configs = self
            .axis_configs
            .iter()
            .map(|(channel, axis)| {
                let mapped = axis
                    .as_ref()
                    .map_exprs(&mut |expr| crate::resolve_repeat_placeholders(expr, ctx))?;
                Ok((channel.clone(), Arc::from(mapped)))
            })
            .collect::<Result<_, AvengerChartError>>()?;
        Ok(resolved)
    }

    pub fn compile(&self, inherited_dataframe: Option<DataFrame>) -> CompiledCoordinateScaleSource {
        let dataframe = if self.data_mode == MarkDataMode::Unit {
            None
        } else {
            self.data.dataframe().cloned().or(inherited_dataframe)
        };
        let data = if let Some(store_data) = self.data.store_data_ref() {
            CompiledDataContext::new_store_data(
                store_data.clone(),
                self.data.transforms().to_vec(),
                self.data.channels().clone(),
            )
        } else {
            CompiledDataContext::new(
                dataframe,
                self.data.transforms().to_vec(),
                self.data.channels().clone(),
            )
        };
        CompiledCoordinateScaleSource {
            data,
            data_mode: self.data_mode,
            facet_data_scope: self.facet_data_scope,
            exclude_from_scale_domains: self.exclude_from_scale_domains,
            axis_configs: self.axis_configs.clone(),
        }
    }
}

/// Serializable compiled form of a coordinate-owned scale source.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCoordinateScaleSource {
    pub data: CompiledDataContext,
    #[serde(default)]
    pub data_mode: MarkDataMode,
    pub facet_data_scope: FacetDataScope,
    #[serde(default)]
    pub exclude_from_scale_domains: bool,
    #[serde(default)]
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}
