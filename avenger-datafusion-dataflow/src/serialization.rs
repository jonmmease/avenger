mod expressions;
use crate::{
    graph::{
        analysis,
        reference::{placeholder_id, GraphRead, ScalarRef, TableRef},
        scope::ScopeDef,
        GraphDef, InputDef, InputKind, NodeDef, NodeKind, OutputDef,
    },
    partition::ScopeIdentity,
    Dataflow, Error, PlanNode, Result, Runtime, ScopeHandle, SemanticConfig, TableSnapshot,
};
use datafusion::{
    arrow::{
        datatypes::{Field, Schema},
        ipc::{reader::StreamReader, writer::StreamWriter},
    },
    catalog::TableProvider,
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        DFSchema, DataFusionError, TableReference,
    },
    execution::{context::SessionContext, TaskContext},
    logical_expr::{
        expr_rewriter::NamePreserver, registry::FunctionRegistry, AggregateUDF, Expr, Extension,
        HigherOrderUDF, LogicalPlan, LogicalPlanBuilder, ScalarUDF, Volatility,
        WindowFunctionDefinition, WindowUDF,
    },
};
use datafusion_proto::{
    logical_plan::{
        from_proto::parse_expr, to_proto::serialize_expr, AsLogicalPlan,
        DefaultLogicalExtensionCodec, LogicalExtensionCodec,
    },
    protobuf::LogicalPlanNode,
};
use prost::Message;
use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
    sync::{Arc, Mutex},
};

mod wire {
    include!(concat!(env!("OUT_DIR"), "/avenger.dataflow.rs"));
}
const ENGINE: &str = "54.1.0";
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Artifact(e.to_string())
}
fn df_error(e: impl std::fmt::Display) -> DataFusionError {
    DataFusionError::Plan(e.to_string())
}
fn required<T>(value: Option<T>, name: &str) -> Result<T> {
    value.ok_or_else(|| invalid(format!("missing {name}")))
}
fn volatility(v: Volatility) -> i32 {
    match v {
        Volatility::Immutable => 0,
        Volatility::Stable => 1,
        Volatility::Volatile => 2,
    }
}

pub(crate) fn encode_table(table: &TableSnapshot) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut writer = StreamWriter::try_new(&mut bytes, table.schema())?;
    for batch in table.batches() {
        writer.write(batch)?;
    }
    writer.finish()?;
    Ok(bytes)
}
pub(crate) fn decode_table(bytes: &[u8]) -> Result<TableSnapshot> {
    let reader = StreamReader::try_new(Cursor::new(bytes), None)?;
    let schema = reader.schema();
    TableSnapshot::from_batches(schema, reader.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[derive(Debug)]
struct Codec {
    graph: u64,
    assets: Vec<usize>,
    application: Arc<dyn LogicalExtensionCodec>,
    semantics: SemanticConfig,
    builtin: Arc<TaskContext>,
    functions: Mutex<BTreeMap<String, wire::FunctionRequirement>>,
    errors: Mutex<Vec<String>>,
}
impl Codec {
    fn new(
        graph: u64,
        assets: Vec<usize>,
        application: Arc<dyn LogicalExtensionCodec>,
        semantics: SemanticConfig,
    ) -> Self {
        Self {
            graph,
            assets,
            application,
            semantics,
            builtin: SessionContext::new().task_ctx(),
            functions: Mutex::default(),
            errors: Mutex::default(),
        }
    }
    fn record(
        &self,
        kind: &str,
        name: &str,
        v: Volatility,
        builtin: bool,
        result: datafusion::common::Result<()>,
    ) -> datafusion::common::Result<()> {
        let key = format!("{kind}:{name}");
        let result = result.and_then(|()| {
            let version = if builtin {
                format!("datafusion:{ENGINE}")
            } else {
                self.semantics
                    .function_versions
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| {
                        df_error(format!("custom function {key} needs a semantic version"))
                    })?
            };
            self.functions.lock().expect("function manifest").insert(
                key,
                wire::FunctionRequirement {
                    kind: kind.into(),
                    name: name.into(),
                    version,
                    volatility: volatility(v),
                },
            );
            Ok(())
        });
        if let Err(error) = &result {
            self.errors
                .lock()
                .expect("codec errors")
                .push(error.to_string());
        }
        result
    }
    fn check(&self) -> Result<()> {
        if let Some(error) = self.errors.lock().expect("codec errors").first() {
            return Err(invalid(error));
        }
        Ok(())
    }
}
impl LogicalExtensionCodec for Codec {
    fn try_encode(
        &self,
        extension: &Extension,
        buf: &mut Vec<u8>,
    ) -> datafusion::common::Result<()> {
        let read = extension
            .node
            .as_any()
            .downcast_ref::<GraphRead>()
            .ok_or_else(|| df_error("unsupported logical extension"))?;
        if read.graph != self.graph {
            return Err(df_error("foreign dataflow reference"));
        }
        let (kind, index) = match read.source {
            TableRef::Input(i) => (wire::read::Kind::Input, i),
            TableRef::Node(i) => (wire::read::Kind::Node, i),
            TableRef::Rows(i) => (wire::read::Kind::Rows, i),
            TableRef::Asset(i) => (wire::read::Kind::Asset, self.assets[i]),
        };
        wire::Read {
            kind: kind as i32,
            index: index as u32,
            schema: Some(read.schema.as_ref().try_into().map_err(df_error)?),
            label: read.label.to_string(),
        }
        .encode(buf)
        .map_err(df_error)
    }
    fn try_decode(
        &self,
        bytes: &[u8],
        inputs: &[LogicalPlan],
        _: &TaskContext,
    ) -> datafusion::common::Result<Extension> {
        if !inputs.is_empty() {
            return Err(df_error("dataflow reference must be a leaf"));
        }
        let read = wire::Read::decode(bytes).map_err(df_error)?;
        let i = read.index as usize;
        let source = match wire::read::Kind::try_from(read.kind).map_err(df_error)? {
            wire::read::Kind::Input => TableRef::Input(i),
            wire::read::Kind::Node => TableRef::Node(i),
            wire::read::Kind::Rows => TableRef::Rows(i),
            wire::read::Kind::Asset => TableRef::Asset(i),
        };
        let schema = DFSchema::try_from(
            &read
                .schema
                .ok_or_else(|| df_error("missing reference schema"))?,
        )
        .map_err(df_error)?;
        Ok(Extension {
            node: Arc::new(GraphRead {
                graph: self.graph,
                source,
                schema: Arc::new(schema),
                label: read.label.into(),
            }),
        })
    }
    fn try_encode_table_provider(
        &self,
        _: &TableReference,
        _: Arc<dyn TableProvider>,
        _: &mut Vec<u8>,
    ) -> datafusion::common::Result<()> {
        Err(df_error("external sources cannot be exported"))
    }
    fn try_decode_table_provider(
        &self,
        _: &[u8],
        _: &TableReference,
        _: Arc<Schema>,
        _: &TaskContext,
    ) -> datafusion::common::Result<Arc<dyn TableProvider>> {
        Err(df_error("external sources are not supported in artifacts"))
    }
    fn try_encode_udf(&self, f: &ScalarUDF, buf: &mut Vec<u8>) -> datafusion::common::Result<()> {
        if f == &ScalarUDF::from(crate::graph::normalize::NullableField) {
            buf.extend_from_slice(b"avenger-nullable-v1");
            self.functions.lock().expect("manifest").insert(
                "scalar:__avenger_nullable_field".into(),
                wire::FunctionRequirement {
                    kind: "scalar".into(),
                    name: f.name().into(),
                    version: "avenger:1".into(),
                    volatility: 0,
                },
            );
            return Ok(());
        }
        self.record(
            "scalar",
            f.name(),
            f.signature().volatility,
            self.builtin.udf(f.name()).is_ok_and(|b| b.as_ref() == f),
            self.application.try_encode_udf(f, buf),
        )
    }
    fn try_encode_udaf(
        &self,
        f: &AggregateUDF,
        buf: &mut Vec<u8>,
    ) -> datafusion::common::Result<()> {
        self.record(
            "aggregate",
            f.name(),
            f.signature().volatility,
            self.builtin.udaf(f.name()).is_ok_and(|b| b.as_ref() == f),
            self.application.try_encode_udaf(f, buf),
        )
    }
    fn try_encode_udwf(&self, f: &WindowUDF, buf: &mut Vec<u8>) -> datafusion::common::Result<()> {
        self.record(
            "window",
            f.name(),
            f.signature().volatility,
            self.builtin.udwf(f.name()).is_ok_and(|b| b.as_ref() == f),
            self.application.try_encode_udwf(f, buf),
        )
    }
    fn try_encode_higher_order_function(
        &self,
        f: &HigherOrderUDF,
        buf: &mut Vec<u8>,
    ) -> datafusion::common::Result<()> {
        self.record(
            "higher_order",
            f.name(),
            f.signature().volatility,
            false,
            self.application.try_encode_higher_order_function(f, buf),
        )
    }
    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> datafusion::common::Result<Arc<ScalarUDF>> {
        if name == "__avenger_nullable_field" && buf == b"avenger-nullable-v1" {
            return Ok(Arc::new(ScalarUDF::from(
                crate::graph::normalize::NullableField,
            )));
        }
        self.application.try_decode_udf(name, buf)
    }
    fn try_decode_udaf(
        &self,
        name: &str,
        buf: &[u8],
    ) -> datafusion::common::Result<Arc<AggregateUDF>> {
        self.application.try_decode_udaf(name, buf)
    }
    fn try_decode_udwf(
        &self,
        name: &str,
        buf: &[u8],
    ) -> datafusion::common::Result<Arc<WindowUDF>> {
        self.application.try_decode_udwf(name, buf)
    }
    fn try_decode_higher_order_function(
        &self,
        name: &str,
        buf: &[u8],
    ) -> datafusion::common::Result<Arc<HigherOrderUDF>> {
        self.application.try_decode_higher_order_function(name, buf)
    }
}

fn remap(plan: LogicalPlan, graph: &GraphDef, exporting: bool) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up_with_subqueries(|plan| {
            let names = NamePreserver::new(&plan);
            plan.map_expressions(|expr| {
                let name = names.save(&expr);
                expr.transform_up(|expr| {
                    let Expr::Placeholder(mut p) = expr else {
                        return Ok(Transformed::no(expr));
                    };
                    if exporting {
                        let (reference, _) = graph
                            .placeholders
                            .get(&p.id)
                            .ok_or_else(|| df_error("unknown placeholder"))?;
                        p.id = match reference {
                            ScalarRef::Input(i) => format!("$dataflow_input_{i}"),
                            ScalarRef::Node(i) => format!("$dataflow_node_{i}"),
                        };
                    } else {
                        let source = if let Some(i) = p.id.strip_prefix("$dataflow_input_") {
                            ScalarRef::Input(i.parse::<usize>().map_err(df_error)?)
                        } else if let Some(i) = p.id.strip_prefix("$dataflow_node_") {
                            ScalarRef::Node(i.parse::<usize>().map_err(df_error)?)
                        } else {
                            return Err(df_error("invalid artifact placeholder"));
                        };
                        p.id = placeholder_id(graph.id, source);
                        let (_, expected) = graph
                            .placeholders
                            .get(&p.id)
                            .ok_or_else(|| df_error("unknown scalar reference"))?;
                        if p.field.as_ref().is_none_or(|f| {
                            f.data_type() != expected.data_type()
                                || f.is_nullable() != expected.is_nullable()
                                || f.metadata() != expected.metadata()
                        }) {
                            return Err(df_error("scalar reference type mismatch"));
                        }
                        p.field = Some(expected.clone());
                    }
                    Ok(Transformed::yes(Expr::Placeholder(p)))
                })
                .map(|r| r.update_data(|e| name.restore(e)))
            })?
            .map_data(LogicalPlan::recompute_schema)
        })?
        .data)
}

fn validate_functions(
    plan: &LogicalPlan,
    requirements: &HashMap<String, wire::FunctionRequirement>,
    builtins: &TaskContext,
) -> Result<()> {
    plan.apply_with_subqueries(|plan| {
        for expr in plan.expressions() {
            expr.apply(|expr| {
                let function = match expr {
                    Expr::ScalarFunction(f) => Some((
                        "scalar",
                        f.func.name(),
                        f.func.signature().volatility,
                        builtins
                            .udf(f.func.name())
                            .is_ok_and(|b| b.as_ref() == f.func.as_ref()),
                    )),
                    Expr::AggregateFunction(f) => Some((
                        "aggregate",
                        f.func.name(),
                        f.func.signature().volatility,
                        builtins
                            .udaf(f.func.name())
                            .is_ok_and(|b| b.as_ref() == f.func.as_ref()),
                    )),
                    Expr::WindowFunction(f) => match &f.fun {
                        WindowFunctionDefinition::AggregateUDF(f) => Some((
                            "aggregate",
                            f.name(),
                            f.signature().volatility,
                            builtins
                                .udaf(f.name())
                                .is_ok_and(|b| b.as_ref() == f.as_ref()),
                        )),
                        WindowFunctionDefinition::WindowUDF(f) => Some((
                            "window",
                            f.name(),
                            f.signature().volatility,
                            builtins
                                .udwf(f.name())
                                .is_ok_and(|b| b.as_ref() == f.as_ref()),
                        )),
                    },
                    Expr::HigherOrderFunction(f) => Some((
                        "higher_order",
                        f.func.name(),
                        f.func.signature().volatility,
                        false,
                    )),
                    _ => None,
                };
                if let Some((kind, name, v, builtin)) = function {
                    let key = format!("{kind}:{name}");
                    let requirement = requirements.get(&key).ok_or_else(|| {
                        df_error(format!("missing function manifest entry {key}"))
                    })?;
                    if requirement.version == format!("datafusion:{ENGINE}") && !builtin {
                        return Err(df_error(format!(
                            "built-in implementation mismatch for {key}"
                        )));
                    }
                    if requirement.volatility != volatility(v) {
                        return Err(df_error(format!("volatility mismatch for {key}")));
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(())
}

impl Dataflow {
    /// Encode this native definition and its fixed assets without executing it.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with_codec(Arc::new(DefaultLogicalExtensionCodec {}))
    }

    /// Encode configured UDFs with application codecs and declared semantic versions.
    pub fn to_bytes_with_codec(
        &self,
        application: Arc<dyn LogicalExtensionCodec>,
    ) -> Result<Vec<u8>> {
        let graph = &self.inner;
        let mut identities = HashMap::new();
        let mut assets = Vec::new();
        let mut indices = Vec::new();
        for asset in &graph.assets {
            let index = if let Some(index) = identities.get(&asset.id()) {
                *index
            } else {
                let index = assets.len();
                assets.push(encode_table(asset)?);
                identities.insert(asset.id(), index);
                index
            };
            indices.push(index);
        }
        let codec = Codec::new(graph.id, indices, application, graph.semantics.clone());
        let mut nodes = Vec::new();
        for (id, node) in graph.nodes.iter().enumerate() {
            node.plan.apply_with_subqueries(|plan| {
                if matches!(plan, LogicalPlan::TableScan(_)) {
                    return Err(df_error(format!(
                        "node {} contains an external source",
                        node.name
                    )));
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
            let plan = remap(node.plan.clone(), graph, true)?;
            let mut expression_extensions = Vec::new();
            let plan = expressions::encode(plan, &codec, &mut expression_extensions)?;
            let program = match node.kind {
                NodeKind::Table => wire::node::Program::Plan(Box::new(
                    LogicalPlanNode::try_from_logical_plan(&plan, &codec)?,
                )),
                NodeKind::Scalar => {
                    let LogicalPlan::Projection(p) = plan else {
                        return Err(invalid("scalar node must be a singleton projection"));
                    };
                    wire::node::Program::Expression(
                        serialize_expr(&p.expr[0], &codec).map_err(invalid)?,
                    )
                }
            };
            codec.check()?;
            nodes.push(wire::Node {
                expression_extensions,
                id: id as u32,
                scope: node.scope as u32,
                name: node.name.to_string(),
                local_rows: node.local_rows,
                schema: Some(node.plan.schema().as_ref().try_into().map_err(invalid)?),
                program: Some(program),
            });
        }
        let inputs = graph
            .inputs
            .iter()
            .enumerate()
            .map(|(id, input)| {
                Ok(wire::Input {
                    id: id as u32,
                    scope: input.scope as u32,
                    name: input.name.to_string(),
                    kind: Some(match &input.kind {
                        InputKind::Table(schema) => {
                            wire::input::Kind::Table(schema.as_ref().try_into().map_err(invalid)?)
                        }
                        InputKind::Scalar(field) => {
                            wire::input::Kind::Scalar(field.as_ref().try_into().map_err(invalid)?)
                        }
                    }),
                })
            })
            .collect::<Result<_>>()?;
        let scopes = graph
            .scopes
            .iter()
            .enumerate()
            .map(|(id, scope)| {
                Ok(wire::Scope {
                    id: id as u32,
                    parent: scope.parent.map(|i| i as u32),
                    name: scope
                        .handle
                        .as_ref()
                        .map_or_else(String::new, |h| h.name().into()),
                    discovery: scope.discovery.map(|i| i as u32),
                    rows: scope.rows.as_ref().map(|r| {
                        let TableRef::Node(i) = r.read.source else {
                            unreachable!()
                        };
                        i as u32
                    }),
                    keys: scope
                        .handle
                        .as_ref()
                        .map(|h| h.key_schema().as_ref().try_into().map_err(invalid))
                        .transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        let artifact = wire::DataflowArtifact {
            version: 1,
            datafusion_version: ENGINE.into(),
            inputs,
            nodes,
            scopes,
            outputs: graph
                .outputs
                .iter()
                .map(|o| wire::Output {
                    scope: o.scope as u32,
                    node: o.node as u32,
                    name: o.name.to_string(),
                })
                .collect(),
            assets,
            time_zone: graph.semantics.time_zone.clone(),
            functions: codec
                .functions
                .into_inner()
                .expect("manifest")
                .into_values()
                .collect(),
        };
        Ok(artifact.encode_to_vec())
    }
}

impl Runtime {
    /// Decode an artifact into native plans and expressions using this runtime's registry and codecs.
    pub fn decode_dataflow(&self, bytes: &[u8]) -> Result<Dataflow> {
        let artifact = wire::DataflowArtifact::decode(bytes).map_err(invalid)?;
        if artifact.version != 1 || artifact.datafusion_version != ENGINE {
            return Err(invalid("unsupported artifact or DataFusion version"));
        }
        let mut requirements = HashMap::new();
        let mut semantics = SemanticConfig {
            time_zone: artifact.time_zone,
            function_versions: BTreeMap::new(),
        };
        for requirement in artifact.functions {
            let key = format!("{}:{}", requirement.kind, requirement.name);
            if !["scalar", "aggregate", "window", "higher_order"]
                .contains(&requirement.kind.as_str())
                || !(0..=2).contains(&requirement.volatility)
            {
                return Err(invalid("invalid function requirement"));
            }
            if requirement.version != format!("datafusion:{ENGINE}")
                && !(key == "scalar:__avenger_nullable_field" && requirement.version == "avenger:1")
            {
                semantics
                    .function_versions
                    .insert(key.clone(), requirement.version.clone());
            }
            if requirements.insert(key, requirement).is_some() {
                return Err(invalid("duplicate function requirement"));
            }
        }
        self.check_semantics(&semantics)?;
        let assets = artifact
            .assets
            .iter()
            .map(|b| decode_table(b))
            .collect::<Result<Vec<_>>>()?;
        let mut expected_rows = Vec::new();
        let mut graph = GraphDef {
            id: crate::fresh_id(),
            assets,
            semantics,
            inputs: Vec::new(),
            nodes: Vec::new(),
            scopes: Vec::new(),
            outputs: Vec::new(),
            placeholders: HashMap::new(),
        };
        for (id, scope) in artifact.scopes.into_iter().enumerate() {
            expected_rows.push(scope.rows.map(|i| i as usize));
            if scope.id as usize != id {
                return Err(invalid("scope IDs must be dense and ordered"));
            }
            if id == 0 {
                if scope.parent.is_some()
                    || scope.discovery.is_some()
                    || scope.rows.is_some()
                    || scope.keys.is_some()
                    || !scope.name.is_empty()
                {
                    return Err(invalid("invalid root scope"));
                }
                graph.scopes.push(ScopeDef::root());
                continue;
            }
            let parent = required(scope.parent, "scope parent")? as usize;
            if parent >= id
                || scope.name.is_empty()
                || !graph.scopes[parent].child_names.insert(scope.name.clone())
            {
                return Err(invalid("invalid scope parent or name"));
            }
            let key_schema =
                Arc::new(Schema::try_from(&required(scope.keys, "scope keys")?).map_err(invalid)?);
            if key_schema.fields().is_empty() {
                return Err(invalid("empty scope key"));
            }
            for field in key_schema.fields() {
                crate::partition::validate_key_type(field.data_type())?;
            }
            let mut path = graph.scopes[parent]
                .handle
                .as_ref()
                .map(|h| h.path.to_vec())
                .unwrap_or_default();
            path.push(ScopeIdentity {
                index: id,
                name: scope.name.into(),
            });
            graph.scopes.push(ScopeDef {
                parent: Some(parent),
                handle: Some(ScopeHandle {
                    graph: graph.id,
                    index: id,
                    parent,
                    path: path.into(),
                    key_schema,
                }),
                discovery: Some(required(scope.discovery, "discovery")? as usize),
                ..ScopeDef::root()
            });
        }
        if graph.scopes.is_empty() {
            return Err(invalid("missing root scope"));
        }
        for (id, input) in artifact.inputs.into_iter().enumerate() {
            if input.id as usize != id {
                return Err(invalid("input IDs must be dense and ordered"));
            }
            let scope = input.scope as usize;
            let owner = graph
                .scopes
                .get_mut(scope)
                .ok_or_else(|| invalid("invalid input scope"))?;
            if input.name.is_empty() || !owner.input_names.insert(input.name.clone()) {
                return Err(invalid("duplicate or empty input name"));
            }
            let kind = match required(input.kind, "input type")? {
                wire::input::Kind::Table(schema) => {
                    InputKind::Table(Arc::new(DFSchema::try_from(&schema).map_err(invalid)?))
                }
                wire::input::Kind::Scalar(field) => {
                    let field = Arc::new(Field::try_from(&field).map_err(invalid)?);
                    graph.placeholders.insert(
                        placeholder_id(graph.id, ScalarRef::Input(id)),
                        (ScalarRef::Input(id), field.clone()),
                    );
                    InputKind::Scalar(field)
                }
            };
            graph.inputs.push(InputDef {
                scope,
                name: input.name.into(),
                kind,
            });
        }
        let codec = Codec::new(
            graph.id,
            Vec::new(),
            self.inner.codec.clone(),
            graph.semantics.clone(),
        );
        let task = self.inner.state.task_ctx();
        for (id, node) in artifact.nodes.into_iter().enumerate() {
            if node.id as usize != id {
                return Err(invalid("node IDs must be dense and topologically ordered"));
            }
            let scope = node.scope as usize;
            let owner = graph
                .scopes
                .get_mut(scope)
                .ok_or_else(|| invalid("invalid node scope"))?;
            if node.name.is_empty() || !owner.node_names.insert(node.name.clone()) {
                return Err(invalid("duplicate or empty node name"));
            }
            let (kind, plan) = match required(node.program, "node program")? {
                wire::node::Program::Plan(plan) => {
                    (NodeKind::Table, plan.try_into_logical_plan(&task, &codec)?)
                }
                wire::node::Program::Expression(expr) => (
                    NodeKind::Scalar,
                    LogicalPlanBuilder::empty(true)
                        .project(vec![parse_expr(&expr, &task, &codec).map_err(invalid)?])?
                        .build()?,
                ),
            };
            let plan = expressions::decode(plan, &node.expression_extensions, &task, &codec)?;
            let plan = remap(plan, &graph, false)?;
            plan.apply_with_subqueries(|p| {
                if matches!(p, LogicalPlan::TableScan(_)) {
                    return Err(df_error("external sources are not supported in artifacts"));
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
            let schema =
                DFSchema::try_from(&required(node.schema, "node schema")?).map_err(invalid)?;
            if !crate::graph::same_schema(plan.schema(), &schema) {
                return Err(invalid(format!(
                    "schema mismatch for node {}: expected {:?}, decoded {:?}",
                    node.name,
                    schema,
                    plan.schema()
                )));
            }
            validate_functions(&plan, &requirements, &codec.builtin)?;
            if matches!(kind, NodeKind::Scalar) {
                if let LogicalPlan::Projection(p) = &plan {
                    analysis::validate_scalar(&p.expr[0])?;
                }
            }
            if let Some(discovery) = graph.scopes[scope].discovery {
                if discovery >= id {
                    return Err(invalid("scope discovery must precede child computations"));
                }
            }
            plan.apply_with_subqueries(|part| {
                if let LogicalPlan::Extension(e) = part {
                    if let Some(read) = e.node.as_any().downcast_ref::<GraphRead>() {
                        if let TableRef::Rows(owner) = read.source {
                            if !node.local_rows
                                || owner != scope
                                || !matches!(&plan, LogicalPlan::Extension(_))
                            {
                                return Err(df_error(
                                    "local-row leaves are only valid as the scope's rows node",
                                ));
                            }
                        }
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
            let analysis = analysis::analyze(&plan, &graph, scope)?;
            if matches!(kind, NodeKind::Scalar) {
                if plan.schema().fields().len() != 1 {
                    return Err(invalid("invalid scalar schema"));
                }
                graph.placeholders.insert(
                    placeholder_id(graph.id, ScalarRef::Node(id)),
                    (ScalarRef::Node(id), plan.schema().field(0).clone()),
                );
            }
            if node.local_rows {
                let LogicalPlan::Extension(e) = &plan else {
                    return Err(invalid("invalid local rows"));
                };
                let read = e
                    .node
                    .as_any()
                    .downcast_ref::<GraphRead>()
                    .ok_or_else(|| invalid("invalid local rows"))?;
                if !matches!(kind, NodeKind::Table)
                    || expected_rows[scope] != Some(id)
                    || scope == 0
                    || read.source != TableRef::Rows(scope)
                    || graph.scopes[scope].rows.is_some()
                {
                    return Err(invalid("invalid local rows source"));
                }
                graph.scopes[scope].rows = Some(PlanNode {
                    read: GraphRead {
                        source: TableRef::Node(id),
                        ..read.clone()
                    },
                });
            }
            graph.nodes.push(NodeDef {
                scope,
                local_rows: node.local_rows,
                name: node.name.into(),
                kind,
                plan,
                analysis,
            });
        }
        for scope in graph.scopes.iter().skip(1) {
            let rows = scope
                .rows
                .as_ref()
                .ok_or_else(|| invalid("missing local rows"))?;
            let discovery = graph
                .nodes
                .get(scope.discovery.expect("child discovery"))
                .ok_or_else(|| invalid("invalid discovery"))?;
            if discovery.scope != scope.parent.expect("child parent") {
                return Err(invalid("discovery owner mismatch"));
            }
            let width = rows.schema().fields().len();
            let keys = scope.handle.as_ref().expect("child handle").key_schema();
            if discovery.plan.schema().fields().len() != width + keys.fields().len() {
                return Err(invalid("discovery schema width mismatch"));
            }
            for (a, b) in discovery.plan.schema().fields()[..width]
                .iter()
                .zip(rows.schema().fields())
            {
                if a != b {
                    return Err(invalid("local rows schema mismatch"));
                }
            }
            for (a, b) in discovery.plan.schema().fields()[width..]
                .iter()
                .zip(keys.fields())
            {
                if a.data_type() != b.data_type() || a.is_nullable() != b.is_nullable() {
                    return Err(invalid("partition key schema mismatch"));
                }
            }
        }
        for output in artifact.outputs {
            let scope = output.scope as usize;
            let node = output.node as usize;
            if graph.nodes.get(node).is_none_or(|n| n.scope != scope) {
                return Err(invalid("invalid output producer"));
            }
            if output.name.is_empty()
                || !graph.scopes[scope].output_names.insert(output.name.clone())
            {
                return Err(invalid("duplicate or empty output"));
            }
            graph.outputs.push(OutputDef {
                scope,
                node,
                name: output.name.into(),
            });
        }
        Ok(Dataflow {
            inner: Arc::new(graph),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_artifacts_are_recoverable_errors() -> Result<()> {
        let mut builder = crate::DataflowBuilder::new();
        let value = builder.add_expr("value", datafusion::logical_expr::lit(42_i64))?;
        builder.scalar_output("value", &value)?;
        let bytes = builder.finish()?.to_bytes()?;
        let runtime = Runtime::new(crate::RuntimeConfig::default())?;
        for length in [1, bytes.len() / 2, bytes.len() - 1] {
            assert!(runtime.decode_dataflow(&bytes[..length]).is_err());
        }
        let original = wire::DataflowArtifact::decode(bytes.as_slice()).unwrap();
        let mut bad = original.clone();
        bad.version = 99;
        assert!(runtime.decode_dataflow(&bad.encode_to_vec()).is_err());
        let mut bad = original.clone();
        bad.outputs[0].node = 999;
        assert!(runtime.decode_dataflow(&bad.encode_to_vec()).is_err());
        let mut bad = original.clone();
        bad.nodes[0].scope = 999;
        assert!(runtime.decode_dataflow(&bad.encode_to_vec()).is_err());
        let mut bad = original.clone();
        bad.assets.push(vec![1, 2, 3]);
        assert!(runtime.decode_dataflow(&bad.encode_to_vec()).is_err());
        let mut bad = original;
        bad.scopes.clear();
        assert!(runtime.decode_dataflow(&bad.encode_to_vec()).is_err());
        Ok(())
    }
}
