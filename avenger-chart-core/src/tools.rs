use std::sync::Arc;

use crate::{
    AvengerChartError, CompiledIdentityAllocator, CoordinateMetricDescriptor, CoordinateSystemCore,
    CoordinationScope, DomainCoordination, Mark, MarkId, Param, ParamRef, RepeatContext, Selection,
    SelectionRef, Store, StoreRef, ToolInstanceId,
    event::{ChartEventBinding, ChartParamChangeBinding},
};
use serde::{Deserialize, Serialize};

/// Public trait for chart tools.
///
/// Tools are authoring-time packages. During plot compilation they expand into
/// ordinary chart params, scale edits, event bindings, and metadata.
pub trait ChartTool<C: CoordinateSystemCore>: Send + Sync + 'static {
    fn id(&self) -> &str;

    fn expand(
        &self,
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<C>, AvengerChartError>;
}

#[derive(Clone, Debug)]
pub struct ToolExpansionContext<'a> {
    pub tool_id: &'a str,
    pub instance_id: ToolInstanceId,
    pub instance_ancestry: Vec<ToolInstanceId>,
    pub scale_targets: &'a [ToolScaleTarget],
    pub coordinate_metrics: &'a [CoordinateMetricDescriptor],
    pub repeat_context: Option<&'a RepeatContext>,
}

impl<'a> ToolExpansionContext<'a> {
    pub fn new(tool_id: &'a str, scale_targets: &'a [ToolScaleTarget]) -> Self {
        let instance_id = CompiledIdentityAllocator::new(format!("tool-context:{tool_id}"))
            .allocate_tool_instance();
        Self {
            tool_id,
            instance_id,
            instance_ancestry: Vec::new(),
            scale_targets,
            coordinate_metrics: &[],
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

    pub fn with_coordinate_metrics(mut self, metrics: &'a [CoordinateMetricDescriptor]) -> Self {
        self.coordinate_metrics = metrics;
        self
    }

    #[doc(hidden)]
    pub fn with_resolved_instance(
        mut self,
        instance_id: ToolInstanceId,
        instance_ancestry: Vec<ToolInstanceId>,
    ) -> Self {
        self.instance_id = instance_id;
        self.instance_ancestry = instance_ancestry;
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

    pub fn coordinate_metric_for_channels(
        &self,
        x_channel: &str,
        y_channel: &str,
    ) -> Option<&'a CoordinateMetricDescriptor> {
        self.coordinate_metrics
            .iter()
            .find(|metric| metric.x_channel == x_channel && metric.y_channel == y_channel)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolScaleTarget {
    pub coord_channel: String,
    pub scale_name: String,
    pub domain_coordination: DomainCoordination,
}

#[derive(Clone, Debug)]
pub enum ResolvedStateDeclaration {
    Param {
        runtime_id: ParamRef,
        param: Param,
        sharing: ToolParamSharing,
    },
    Store {
        runtime_id: StoreRef,
        store: Store,
    },
    Selection {
        runtime_id: SelectionRef,
        selection: Selection,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolExportTarget {
    Param(ParamRef),
    Store(StoreRef),
    Selection(SelectionRef),
    Mark(MarkId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExport {
    pub alias: String,
    pub target: ToolExportTarget,
}

/// Serializable identity and export surface retained after tool expansion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledToolBehavior {
    pub source_id: String,
    pub instance_id: ToolInstanceId,
    pub instance_ancestry: Vec<ToolInstanceId>,
    pub component_kind: String,
    pub component_id: Option<String>,
    pub exports: Vec<ToolExport>,
}

pub struct ResolvedToolMark<C: CoordinateSystemCore> {
    pub runtime_id: MarkId,
    pub mark: Arc<dyn Mark<C>>,
    pub part_alias: Option<String>,
}

impl<C: CoordinateSystemCore> Clone for ResolvedToolMark<C> {
    fn clone(&self) -> Self {
        Self {
            runtime_id: self.runtime_id.clone(),
            mark: self.mark.clone(),
            part_alias: self.part_alias.clone(),
        }
    }
}

/// Canonical resolved behavior produced by native and future DSL-defined tools.
pub struct ToolBehaviorExpansion<C: CoordinateSystemCore> {
    pub instance_id: ToolInstanceId,
    pub instance_ancestry: Vec<ToolInstanceId>,
    pub component_kind: String,
    pub component_id: Option<String>,
    pub state: Vec<ResolvedStateDeclaration>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub param_change_bindings: Vec<ChartParamChangeBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub marks: Vec<ResolvedToolMark<C>>,
    pub nested_tools: Vec<Arc<dyn ChartTool<C>>>,
    pub exports: Vec<ToolExport>,
    pub metadata: Vec<ToolMetadata>,
}

impl<C: CoordinateSystemCore> Clone for ToolBehaviorExpansion<C> {
    fn clone(&self) -> Self {
        Self {
            instance_id: self.instance_id.clone(),
            instance_ancestry: self.instance_ancestry.clone(),
            component_kind: self.component_kind.clone(),
            component_id: self.component_id.clone(),
            state: self.state.clone(),
            event_bindings: self.event_bindings.clone(),
            param_change_bindings: self.param_change_bindings.clone(),
            scale_edits: self.scale_edits.clone(),
            marks: self.marks.clone(),
            nested_tools: self.nested_tools.clone(),
            exports: self.exports.clone(),
            metadata: self.metadata.clone(),
        }
    }
}

impl<C: CoordinateSystemCore> ToolBehaviorExpansion<C> {
    pub fn new(instance_id: ToolInstanceId) -> Self {
        Self {
            instance_id,
            instance_ancestry: Vec::new(),
            component_kind: "tool".to_string(),
            component_id: None,
            state: Vec::new(),
            event_bindings: Vec::new(),
            param_change_bindings: Vec::new(),
            scale_edits: Vec::new(),
            marks: Vec::new(),
            nested_tools: Vec::new(),
            exports: Vec::new(),
            metadata: Vec::new(),
        }
    }

    pub fn with_instance_ancestry(mut self, ancestry: Vec<ToolInstanceId>) -> Self {
        self.instance_ancestry = ancestry;
        self
    }

    /// Iterate the parameter declarations owned by this resolved behavior.
    pub fn params(&self) -> impl Iterator<Item = (&Param, &ToolParamSharing)> {
        self.state.iter().filter_map(|state| match state {
            ResolvedStateDeclaration::Param { param, sharing, .. } => Some((param, sharing)),
            _ => None,
        })
    }

    pub fn component(mut self, kind: impl Into<String>, id: impl Into<String>) -> Self {
        self.component_kind = kind.into();
        self.component_id = Some(id.into());
        self
    }

    pub fn param(self, param: Param, sharing: ToolParamSharing) -> Self {
        let alias = param.name.clone();
        self.param_as(alias, param, sharing)
    }

    pub fn param_as(
        mut self,
        alias: impl Into<String>,
        param: Param,
        sharing: ToolParamSharing,
    ) -> Self {
        let runtime_id = CompiledIdentityAllocator::derive_tool_param(
            &self.instance_id,
            self.state
                .iter()
                .filter(|state| matches!(state, ResolvedStateDeclaration::Param { .. }))
                .count() as u64,
        );
        self.exports.push(ToolExport {
            alias: alias.into(),
            target: ToolExportTarget::Param(runtime_id.clone()),
        });
        self.state.push(ResolvedStateDeclaration::Param {
            runtime_id,
            param,
            sharing,
        });
        self
    }

    /// Add another public alias for an already-declared parameter state slot.
    pub fn export_param_as(mut self, alias: impl Into<String>, param: &Param) -> Self {
        let runtime_id = self
            .state
            .iter()
            .find_map(|state| match state {
                ResolvedStateDeclaration::Param {
                    runtime_id,
                    param: declared,
                    ..
                } if declared.name == param.name => Some(runtime_id.clone()),
                _ => None,
            })
            .expect("export_param_as requires a parameter already declared by this behavior");
        let alias = alias.into();
        if !self.exports.iter().any(|export| {
            export.alias == alias && export.target == ToolExportTarget::Param(runtime_id.clone())
        }) {
            self.exports.push(ToolExport {
                alias,
                target: ToolExportTarget::Param(runtime_id),
            });
        }
        self
    }

    pub fn store(self, store: Store) -> Self {
        let alias = store.name.clone();
        self.store_as(alias, store)
    }

    pub fn store_as(mut self, alias: impl Into<String>, store: Store) -> Self {
        let runtime_id = CompiledIdentityAllocator::derive_tool_store(
            &self.instance_id,
            self.state
                .iter()
                .filter(|state| matches!(state, ResolvedStateDeclaration::Store { .. }))
                .count() as u64,
        );
        self.exports.push(ToolExport {
            alias: alias.into(),
            target: ToolExportTarget::Store(runtime_id.clone()),
        });
        self.state
            .push(ResolvedStateDeclaration::Store { runtime_id, store });
        self
    }

    pub fn selection(self, selection: Selection) -> Self {
        let alias = selection.id.clone();
        self.selection_as(alias, selection)
    }

    pub fn selection_as(mut self, alias: impl Into<String>, selection: Selection) -> Self {
        let runtime_id = CompiledIdentityAllocator::derive_tool_selection(
            &self.instance_id,
            self.state
                .iter()
                .filter(|state| matches!(state, ResolvedStateDeclaration::Selection { .. }))
                .count() as u64,
        );
        self.exports.push(ToolExport {
            alias: alias.into(),
            target: ToolExportTarget::Selection(runtime_id.clone()),
        });
        self.state.push(ResolvedStateDeclaration::Selection {
            runtime_id,
            selection,
        });
        self
    }

    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    pub fn param_change_binding(mut self, binding: ChartParamChangeBinding) -> Self {
        self.param_change_bindings.push(binding);
        self
    }

    pub fn scale_edit(mut self, edit: ToolScaleEdit) -> Self {
        self.scale_edits.push(edit);
        self
    }

    pub fn mark(mut self, mark: impl Mark<C>) -> Self {
        self = self.mark_arc(Arc::new(mark));
        self
    }

    pub fn mark_arc(mut self, mark: Arc<dyn Mark<C>>) -> Self {
        let part_alias = mark.state().id.clone();
        self.push_mark(mark, part_alias);
        self
    }

    pub fn mark_part(mut self, part_alias: impl Into<String>, mark: impl Mark<C>) -> Self {
        self.push_mark(Arc::new(mark), Some(part_alias.into()));
        self
    }

    fn push_mark(&mut self, mark: Arc<dyn Mark<C>>, part_alias: Option<String>) {
        let runtime_id =
            CompiledIdentityAllocator::derive_tool_mark(&self.instance_id, self.marks.len() as u64);
        if let Some(alias) = &part_alias {
            self.exports.push(ToolExport {
                alias: alias.clone(),
                target: ToolExportTarget::Mark(runtime_id.clone()),
            });
        }
        self.marks.push(ResolvedToolMark {
            runtime_id,
            mark,
            part_alias,
        });
    }

    pub fn nested_tool(mut self, tool: Arc<dyn ChartTool<C>>) -> Self {
        self.nested_tools.push(tool);
        self
    }

    pub fn metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata.push(metadata);
        self
    }
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
