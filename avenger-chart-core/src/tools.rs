use crate::{AvengerChartError, Param, Sharing, event::ChartEventBinding};
use serde::{Deserialize, Serialize};

/// Public trait for chart tools.
///
/// Tools are authoring-time packages. During plot compilation they expand into
/// ordinary chart params, scale edits, event bindings, and metadata.
pub trait ChartTool: Send + Sync + 'static {
    fn id(&self) -> &str;

    fn expand(&self, ctx: ToolExpansionContext<'_>) -> Result<ToolExpansion, AvengerChartError>;
}

#[derive(Clone, Copy, Debug)]
pub struct ToolExpansionContext<'a> {
    pub tool_id: &'a str,
}

#[derive(Clone, Debug, Default)]
pub struct ToolExpansion {
    pub params: Vec<ToolParamExpansion>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub metadata: Vec<ToolMetadata>,
}

impl ToolExpansion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn param(mut self, param: Param, sharing: ToolParamSharing) -> Self {
        self.params.push(ToolParamExpansion { param, sharing });
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
