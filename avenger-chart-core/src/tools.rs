use std::sync::Arc;

use crate::{
    AvengerChartError, CoordinateSystemCore, CoordinationScope, DomainCoordination, Mark, Param,
    RepeatContext, Selection, Store, UnitAspectConstraint, event::ChartEventBinding,
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
    pub scale_targets: &'a [ToolScaleTarget],
    pub unit_aspect_constraints: &'a [UnitAspectConstraint],
    pub repeat_context: Option<&'a RepeatContext>,
}

impl<'a> ToolExpansionContext<'a> {
    pub fn new(tool_id: &'a str, scale_targets: &'a [ToolScaleTarget]) -> Self {
        Self {
            tool_id,
            scale_targets,
            unit_aspect_constraints: &[],
            repeat_context: None,
        }
    }

    pub fn empty(tool_id: &'a str) -> Self {
        Self::new(tool_id, &[])
    }

    pub fn with_repeat_context(mut self, repeat_context: &'a RepeatContext) -> Self {
        self.repeat_context = Some(repeat_context);
        self
    }

    pub fn with_unit_aspect_constraints(mut self, constraints: &'a [UnitAspectConstraint]) -> Self {
        self.unit_aspect_constraints = constraints;
        self
    }

    pub fn repeat_cell_id(&self) -> Option<String> {
        let repeat = self.repeat_context?;
        match (&repeat.item, &repeat.row, &repeat.column) {
            (Some(item), _, _) => Some(format!("repeat_cell:item:{}", item.id)),
            (_, Some(row), Some(column)) => Some(format!("repeat_cell:{}:{}", row.id, column.id)),
            (_, Some(row), None) => Some(format!("repeat_cell:row:{}", row.id)),
            (_, None, Some(column)) => Some(format!("repeat_cell:column:{}", column.id)),
            _ => None,
        }
    }

    pub fn scale_targets_for_channel<'b>(
        &'b self,
        channel: &'b str,
    ) -> impl Iterator<Item = &'b ToolScaleTarget> + 'b {
        self.scale_targets
            .iter()
            .filter(move |target| target.coord_channel == channel)
    }

    pub fn single_domain_coordination_for_channel(
        &self,
        channel: &str,
    ) -> Option<DomainCoordination> {
        let mut coordination = None;
        for target in self
            .scale_targets
            .iter()
            .filter(|target| target.coord_channel == channel)
        {
            match &coordination {
                Some(existing) if existing != &target.domain_coordination => return None,
                Some(_) => {}
                None => coordination = Some(target.domain_coordination.clone()),
            }
        }
        coordination
    }

    pub fn unit_aspect_constraint_for_channels(
        &self,
        x_channel: &str,
        y_channel: &str,
    ) -> Option<&'a UnitAspectConstraint> {
        self.unit_aspect_constraints.iter().find(|constraint| {
            constraint.x_channel == x_channel && constraint.y_channel == y_channel
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolScaleTarget {
    pub coord_channel: String,
    pub scale_name: String,
    pub domain_coordination: DomainCoordination,
}

pub struct ToolExpansion<C: CoordinateSystemCore> {
    pub params: Vec<ToolParamExpansion>,
    pub stores: Vec<Store>,
    pub selections: Vec<Selection>,
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
            selections: self.selections.clone(),
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
            selections: Vec::new(),
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

    pub fn selection(mut self, selection: Selection) -> Self {
        self.selections.push(selection);
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
    Explicit(CoordinationScope),
    MirrorScale { channel: String },
}

impl ToolParamSharing {
    pub fn explicit(sharing: CoordinationScope) -> Self {
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
