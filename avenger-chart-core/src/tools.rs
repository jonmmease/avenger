use std::sync::Arc;

use crate::{
    AvengerChartError, CoordinateSystemCore, Mark, Param, Sharing, Store, event::ChartEventBinding,
};
use serde::{Deserialize, Serialize};

/// Public trait for chart tools.
///
/// Tools are authoring-time packages. During plot compilation they expand into
/// ordinary chart params, scale edits, event bindings, and metadata.
pub trait ChartTool<C: CoordinateSystemCore>: Send + Sync + 'static {
    fn id(&self) -> &str;

    fn expand(&self, ctx: ToolExpansionContext<'_>) -> Result<ToolExpansion<C>, AvengerChartError>;
}

#[derive(Clone, Copy, Debug)]
pub struct ToolExpansionContext<'a> {
    pub tool_id: &'a str,
}

pub struct ToolExpansion<C: CoordinateSystemCore> {
    pub params: Vec<ToolParamExpansion>,
    pub stores: Vec<Store>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub marks: Vec<Arc<dyn Mark<C>>>,
    pub metadata: Vec<ToolMetadata>,
}

impl<C: CoordinateSystemCore> Clone for ToolExpansion<C> {
    fn clone(&self) -> Self {
        Self {
            params: self.params.clone(),
            stores: self.stores.clone(),
            event_bindings: self.event_bindings.clone(),
            scale_edits: self.scale_edits.clone(),
            marks: self.marks.clone(),
            metadata: self.metadata.clone(),
        }
    }
}

impl<C: CoordinateSystemCore> Default for ToolExpansion<C> {
    fn default() -> Self {
        Self {
            params: Vec::new(),
            stores: Vec::new(),
            event_bindings: Vec::new(),
            scale_edits: Vec::new(),
            marks: Vec::new(),
            metadata: Vec::new(),
        }
    }
}

impl<C: CoordinateSystemCore> ToolExpansion<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn param(mut self, param: Param, sharing: ToolParamSharing) -> Self {
        self.params.push(ToolParamExpansion { param, sharing });
        self
    }

    pub fn store(mut self, store: Store) -> Self {
        self.stores.push(store);
        self
    }

    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    pub fn scale_edit(mut self, edit: ToolScaleEdit) -> Self {
        self.scale_edits.push(edit);
        self
    }

    pub fn mark(mut self, mark: impl Mark<C>) -> Self {
        self.marks.push(Arc::new(mark));
        self
    }

    pub fn mark_arc(mut self, mark: Arc<dyn Mark<C>>) -> Self {
        self.marks.push(mark);
        self
    }

    pub fn metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata.push(metadata);
        self
    }
}

#[derive(Clone, Debug)]
pub struct ToolParamExpansion {
    pub param: Param,
    pub sharing: ToolParamSharing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolParamSharing {
    Explicit(Sharing),
    MirrorScale { channel: String },
}

impl ToolParamSharing {
    pub fn explicit(sharing: Sharing) -> Self {
        Self::Explicit(sharing)
    }

    pub fn mirror_scale(channel: impl Into<String>) -> Self {
        Self::MirrorScale {
            channel: channel.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ToolScaleEdit {
    RawDomain {
        channel: String,
        param_name: String,
        override_existing: bool,
        disable_nice_zero: bool,
    },
}

impl ToolScaleEdit {
    pub fn raw_domain(channel: impl Into<String>, param_name: impl Into<String>) -> Self {
        Self::RawDomain {
            channel: channel.into(),
            param_name: param_name.into(),
            override_existing: false,
            disable_nice_zero: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub id: String,
    pub label: String,
    pub enabled_param: Option<String>,
}

impl ToolMetadata {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled_param: None,
        }
    }

    pub fn enabled_param(mut self, name: impl Into<String>) -> Self {
        self.enabled_param = Some(name.into());
        self
    }
}
