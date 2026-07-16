use avenger_chart_core::{
    ChartActionParamValue, ChartEventAction, ChartEventAssignmentScope, ChartEventBinding,
    ChartEventCursorAction, ChartEventParamAction, ChartEventSelectionAction,
    ChartEventStoreAction, CompiledIdentityAllocator, CoordinateSystemCore, DataTransformStage,
    DefaultLogicalExprNodeExt, Mark, Param, ResolvedStateTarget, SelectionRef, SelectionUpdate,
    StateMigrationKey, StateSymbol, StateSymbolTable, StoreRef, StoreUpdate, ToolInstanceId,
    ToolMetadata, ToolScaleEdit, ViewId, WidgetInstanceId, WidgetItems, WidgetMeasureSpec,
    WidgetPresentationBindings,
};
use datafusion::{arrow::datatypes::DataType, prelude::lit};
use datafusion_proto::protobuf::LogicalExprNode;
use std::sync::Arc;

/// Compile-only summaries of the landed ownership boundaries. These remain in
/// the contract fixture so later frontend work can review the cross-crate shape
/// without introducing a second execution model.
#[allow(dead_code)]
mod target_shape {
    use super::*;

    pub struct ViewScopeContract {
        pub id: ViewId,
    }

    pub struct PipelineContract {
        pub stages: Vec<DataTransformStage>,
        pub output_names: Vec<String>,
    }

    pub enum StateDeclarationContract {
        Store {
            id: StoreRef,
            migration_key: Option<StateMigrationKey>,
        },
        Selection {
            id: SelectionRef,
            migration_key: Option<StateMigrationKey>,
        },
    }

    pub struct ToolBehaviorContract<C: CoordinateSystemCore> {
        pub instance_id: ToolInstanceId,
        pub state: Vec<StateDeclarationContract>,
        pub event_bindings: Vec<ChartEventBinding>,
        pub scale_edits: Vec<ToolScaleEdit>,
        pub marks: Vec<Arc<dyn Mark<C>>>,
        pub metadata: Vec<ToolMetadata>,
    }

    pub struct WidgetContract<C: CoordinateSystemCore> {
        pub instance_id: WidgetInstanceId,
        pub behavior: ToolBehaviorContract<C>,
        pub items: Option<WidgetItems>,
        pub measure: WidgetMeasureSpec,
        pub presentation: WidgetPresentationBindings,
    }
}

fn expr(value: impl Into<datafusion::scalar::ScalarValue>) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(lit(value.into())).expect("contract expression serializes")
}

#[test]
fn target_identity_and_action_contracts_are_ordered_and_serializable() {
    let mut ids = CompiledIdentityAllocator::new("contract-chart");
    let param = ids.allocate_param();
    let store = ids.allocate_store();
    let selection = ids.allocate_selection();

    let actions = vec![
        ChartEventAction::SetStore(ChartEventStoreAction {
            target: ResolvedStateTarget::new(store, "rows"),
            update: StoreUpdate::clear(),
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: false,
        }),
        ChartEventAction::SetParam(ChartEventParamAction {
            target: ResolvedStateTarget::new(param, "count"),
            value: ChartActionParamValue::Expr { expr: expr(2_i64) },
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: false,
            reject_null: false,
        }),
        ChartEventAction::SetSelection(Box::new(ChartEventSelectionAction {
            target: ResolvedStateTarget::new(selection, "picked"),
            update: SelectionUpdate::Clear,
            scope: ChartEventAssignmentScope::Current,
        })),
        ChartEventAction::SetCursor(ChartEventCursorAction {
            value: expr("crosshair"),
        }),
    ];

    let restored: Vec<ChartEventAction> =
        bincode::deserialize(&bincode::serialize(&actions).unwrap()).unwrap();
    assert_eq!(restored, actions);
    assert!(matches!(restored[0], ChartEventAction::SetStore(_)));
    assert!(matches!(restored[1], ChartEventAction::SetParam(_)));
    assert!(matches!(restored[2], ChartEventAction::SetSelection(_)));
    assert!(matches!(restored[3], ChartEventAction::SetCursor(_)));
}

#[test]
fn same_named_component_state_does_not_require_generated_name_prefixes() {
    let mut ids = CompiledIdentityAllocator::new("two-component-instances");
    let mut first = StateSymbolTable::default();
    let mut second = StateSymbolTable::default();
    let first_id = ids.allocate_param();
    let second_id = ids.allocate_param();

    first
        .insert("value", StateSymbol::Param(first_id.clone()))
        .unwrap();
    second
        .insert("value", StateSymbol::Param(second_id.clone()))
        .unwrap();

    assert_ne!(first_id, second_id);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
}

#[test]
fn explicit_param_type_and_migration_metadata_are_independent_contracts() {
    let param = Param::typed("value", DataType::Int32, 1_i32).unwrap();
    assert_eq!(param.data_type, DataType::Int32);

    let mut ids = CompiledIdentityAllocator::new("contract-chart");
    let runtime_id = ids.allocate_param();
    let migration_key = ids.migration_key("component[0]/param:value");
    assert_ne!(runtime_id.as_opaque_str(), migration_key.as_opaque_str());

    // These identities pin the distinct type-level boundaries used by the
    // corresponding landed runtime owners.
    let mark = ids.allocate_mark();
    let view = ids.allocate_view();
    assert_ne!(mark.as_opaque_str(), view.as_opaque_str());
    let widget = ids.allocate_widget_instance();
    let behavior = ids.widget_behavior_instance(&widget);
    assert_ne!(widget.as_opaque_str(), behavior.as_opaque_str());
}
