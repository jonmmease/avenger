//! avenger-sql for a plan that bakes, which runs away from the session that planned it, such as in
//! a page: the host that encodes the library's functions, the transforms, and the extension nodes
//! of avenger-sql's plans for avenger-chart-datafusion's codec, and the session in which baked
//! plans run.

use std::sync::Arc;

use avenger_chart_datafusion::{codec::HostCodec, functions::Combine};
use avenger_datafusion_shared::SharedNode;
use avenger_datafusion_transforms::codec as transforms;
use datafusion::{
    common::{exec_datafusion_err, Result},
    execution::SessionState,
    logical_expr::{AggregateUDF, LogicalPlan, ScalarUDF, UserDefinedLogicalNode, WindowUDF},
    optimizer::{simplify_expressions::SimplifyExpressions, Analyzer, Optimizer},
    prelude::{SessionConfig, SessionContext},
};

/// The kinds of the bytes of an extension node, the first byte of each.
const SHARED: u8 = 1;
const TRANSFORM: u8 = 2;

/// The host of avenger-sql's plans, which [`register`](crate::register) adds to a session.
pub fn host() -> Arc<dyn HostCodec> {
    Arc::new(Host)
}

#[derive(Debug)]
struct Host;

impl HostCodec for Host {
    fn name(&self) -> &str {
        "avenger-sql"
    }

    fn encode_udf(&self, udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
        crate::library::codec::encode_udf(udf).or_else(|| transforms::encode_udf(udf))
    }

    fn decode_udf(&self, name: &str, bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>> {
        (crate::library::codec::decode_udf(name, bytes))
            .or_else(|| transforms::decode_udf(name, bytes))
    }

    fn encode_udaf(&self, udaf: &AggregateUDF) -> Option<Result<Vec<u8>>> {
        transforms::encode_udaf(udaf)
    }

    fn decode_udaf(&self, name: &str, bytes: &[u8]) -> Option<Result<Arc<AggregateUDF>>> {
        transforms::decode_udaf(name, bytes)
    }

    fn encode_udwf(&self, udwf: &WindowUDF) -> Option<Result<Vec<u8>>> {
        transforms::encode_udwf(udwf)
    }

    fn decode_udwf(&self, name: &str, bytes: &[u8]) -> Option<Result<Arc<WindowUDF>>> {
        transforms::decode_udwf(name, bytes)
    }

    fn encode_node(&self, node: &dyn UserDefinedLogicalNode) -> Option<Result<Vec<u8>>> {
        if let Some(shared) = node.as_any().downcast_ref::<SharedNode>() {
            let mut bytes = vec![SHARED];
            bytes.extend(shared.to_bytes());
            return Some(Ok(bytes));
        }
        // A table function of the transforms, such as `impute`.
        let transform = transforms::encode_node(node)?;
        Some(transform.map(|transform| {
            let mut bytes = vec![TRANSFORM];
            bytes.extend(transform);
            bytes
        }))
    }

    fn decode_node(
        &self,
        bytes: &[u8],
        inputs: &[LogicalPlan],
    ) -> Result<Arc<dyn UserDefinedLogicalNode>> {
        match (bytes.split_first(), inputs) {
            (Some((&SHARED, bytes)), [input]) => {
                Ok(Arc::new(SharedNode::from_bytes(bytes, input.clone())?))
            }
            (Some((&TRANSFORM, bytes)), _) => match transforms::decode_node(bytes, inputs) {
                Some(node) => node,
                None => Err(exec_datafusion_err!(
                    "the bytes of a node of avenger-sql: a transform that this build does not know"
                )),
            },
            _ => Err(exec_datafusion_err!(
                "the bytes of a node of avenger-sql: a kind that this build does not know"
            )),
        }
    }
}

/// `plan`, an unoptimized plan of `state`, as a host sends it: analyzed, and with its expressions
/// simplified. The simplification binds each call of a mark and of a composition to its aggregate,
/// which holds the call's options as its state, and folds the constants of the calls' arguments, so
/// the plan holds no declaration or constructor of avenger-sql's chart functions, which the session
/// of a baked plan does not know.
///
/// The rest of the optimizer does not run. It would move a filter by a selection, whose
/// `selected` call names no column until a state binds it, and drop the columns that the
/// selection tests.
pub fn bake_plan(plan: LogicalPlan, state: &SessionState) -> Result<LogicalPlan> {
    // The analyzer's rules without its function rewrites, one of which fails a plan that still
    // holds a `selected` call. The session of a baked plan applies the rewrites after it binds.
    let analyzer = Analyzer::with_rules(state.analyzer().rules.clone());
    let analyzed = analyzer.execute_and_check(plan, state.config_options(), |_, _| {})?;
    Optimizer::with_rules(vec![Arc::new(SimplifyExpressions::new())]).optimize(
        analyzed,
        state,
        |_, _| {},
    )
}

/// The session in which the baked plans that a host sent run, and in which the host checks that
/// they plan: one partition, DataFusion's functions, the scalar compositions, and the planner, the
/// rules, and the host of [`register`](crate::register). A function with state reaches the session
/// through the plan's codec, and not through the session's functions.
pub fn baked_session() -> Result<SessionContext> {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
    crate::register(&ctx)?;
    for function in Combine::all() {
        ctx.register_udf(function.as_ref().clone());
    }
    Ok(ctx)
}
