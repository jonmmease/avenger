//! The transforms in a plan that travels, such as to a page: each function travels by its name
//! and the bytes of its state, and a [`TableFunctionNode`] by the bytes of the transform of each
//! of its partitions, its partition columns, and its schema, with its input as its child. A
//! front end that sends its plans, such as avenger-sql, calls these from its codec.
//!
//! The bytes are the messages of package `avenger.transforms.v1`, which [`proto`](crate::proto)
//! holds: a node travels as a `Node`, and the state of the function that makes a struct as a
//! `StructState`. A function without state travels as its name alone, with no bytes.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    common::{Column, DFSchema, DFSchemaRef, Result, ScalarValue, exec_datafusion_err},
    logical_expr::{AggregateUDF, Expr, LogicalPlan, ScalarUDF, UserDefinedLogicalNode, WindowUDF},
};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    bin,
    impute::{Fill, GapKind, Gaps, Spec, Stat, Step},
    match_recognize::{Anchor, Program, Skip, node::PartitionMatch, pattern::Inst},
    proto::{
        self, Impute, Instruction, MatchRecognize, Node, Operation, Statistic, StructState,
        TableFunction, node::Kind, table_function::Transform,
    },
    stack,
    table_function::{PartitionTransform, TableFunctionNode, TableTransform},
    timeunit::functions as timeunit,
    utils::{self, MakeStruct},
};

fn invalid(error: impl std::fmt::Display) -> datafusion::common::DataFusionError {
    exec_datafusion_err!("the bytes of a transform: {error}")
}

/// The bytes of `value`: an encoded `datafusion_common.ScalarValue`.
fn scalar(value: &ScalarValue) -> Result<Vec<u8>> {
    let value = protobuf::ScalarValue::try_from(value).map_err(invalid)?;
    Ok(value.encode_to_vec())
}

/// The value of `bytes`, an encoded `datafusion_common.ScalarValue`.
fn value(bytes: &[u8]) -> Result<ScalarValue> {
    let value = protobuf::ScalarValue::decode(bytes).map_err(invalid)?;
    ScalarValue::try_from(&value).map_err(invalid)
}

fn values(values: &[ScalarValue]) -> Result<Vec<Vec<u8>>> {
    values.iter().map(scalar).collect()
}

fn scalars(bytes: &[Vec<u8>]) -> Result<Vec<ScalarValue>> {
    bytes.iter().map(|bytes| value(bytes)).collect()
}

fn indices(indices: &[usize]) -> Vec<u64> {
    indices.iter().map(|index| *index as u64).collect()
}

fn usizes(indices: &[u64]) -> Vec<usize> {
    indices.iter().map(|index| *index as usize).collect()
}

/// The transforms' scalar functions without state.
fn stateless_udfs() -> [Arc<ScalarUDF>; 4] {
    [
        Arc::new(bin::BIN_PARAMS.clone()),
        Arc::new(bin::BIN_APPLY.clone()),
        Arc::new(timeunit::TIMEUNIT.clone()),
        Arc::new(timeunit::APPLY.clone()),
    ]
}

/// The bytes of the state of `udf`, when it is one of the transforms' scalar functions: a
/// [`StructState`] for a struct of a field, as `MakeStruct` makes it, and no bytes for the others.
pub fn encode_udf(udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
    let inner = udf.inner().as_ref() as &dyn std::any::Any;
    if let Some(make) = inner.downcast_ref::<MakeStruct>() {
        let schema = Schema::new(make.fields.clone());
        return Some(
            protobuf::Schema::try_from(&schema)
                .map_err(invalid)
                .map(|fields| {
                    StructState {
                        fields: Some(fields.encode_to_vec()),
                        nullable: make.nullable,
                        metadata: make.metadata.clone().into_iter().collect(),
                    }
                    .encode_to_vec()
                }),
        );
    }
    (stateless_udfs().iter())
        .any(|function| function.name() == udf.name())
        .then(|| Ok(Vec::new()))
}

/// The transforms' scalar function named `name` with the state `bytes`, when it is one of theirs.
pub fn decode_udf(name: &str, bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>> {
    if name == "__make_struct" || name == "__make_nullable_struct" {
        return Some((|| {
            let decoded = StructState::decode(bytes).map_err(invalid)?;
            // A state that failed to encode holds no fields, and is not a struct without fields.
            let Some(fields) = &decoded.fields else {
                return Err(invalid("a struct without fields"));
            };
            let fields = protobuf::Schema::decode(fields.as_slice()).map_err(invalid)?;
            let schema = Schema::try_from(&fields).map_err(invalid)?;
            let field = Field::new(
                "",
                DataType::Struct(schema.fields().clone()),
                decoded.nullable,
            )
            .with_metadata(decoded.metadata);
            Ok(Arc::new(utils::struct_udf(&field)))
        })());
    }
    (stateless_udfs().into_iter())
        .find(|function| function.name() == name)
        .map(Ok)
}

/// The bytes of the state of `udaf`, when it is one of the transforms' aggregate functions.
pub fn encode_udaf(udaf: &AggregateUDF) -> Option<Result<Vec<u8>>> {
    (udaf.name() == timeunit::CHOOSE.name()).then(|| Ok(Vec::new()))
}

/// The transforms' aggregate function named `name`, when it is one of theirs.
pub fn decode_udaf(name: &str, _bytes: &[u8]) -> Option<Result<Arc<AggregateUDF>>> {
    (name == timeunit::CHOOSE.name()).then(|| Ok(Arc::new(timeunit::CHOOSE.clone())))
}

/// The bytes of the state of `udwf`, when it is one of the transforms' window functions.
pub fn encode_udwf(udwf: &WindowUDF) -> Option<Result<Vec<u8>>> {
    let inner = udwf.inner().as_ref() as &dyn std::any::Any;
    inner
        .downcast_ref::<stack::StackWindow>()
        .map(|_| Ok(Vec::new()))
}

/// The transforms' window function named `name`, when it is one of theirs.
pub fn decode_udwf(name: &str, _bytes: &[u8]) -> Option<Result<Arc<WindowUDF>>> {
    let stack = stack::window();
    (name == stack.name()).then_some(Ok(stack))
}

/// The transform of a table function's node that a reader decoded: the transform of each
/// partition, as the node's planner made it where the plan was written, with the schema of its
/// result. Its arguments are bound into the transform, so it has none.
///
/// Two of them are equal when their messages and the columns of their inputs are, so the nodes
/// that two readers build from the same bytes are equal and hash equally.
#[derive(Debug, Clone)]
struct Resolved {
    /// The transform's message, as the field of a [`TableFunction`] that holds it, encoded.
    bytes: Vec<u8>,
    /// The name of the transform's logical node.
    name: &'static str,
    transform: Arc<dyn PartitionTransform>,
    schema: DFSchemaRef,
    /// The names of the columns of the input, which the transform reads by position.
    input: Vec<String>,
}

impl PartialEq for Resolved {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.input == other.input
    }
}

impl std::hash::Hash for Resolved {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (&self.bytes, &self.input).hash(state);
    }
}

/// The names of the columns of `schema`, in order.
fn names(schema: &DFSchema) -> Vec<String> {
    let fields = schema.fields().iter();
    fields.map(|field| field.name().clone()).collect()
}

impl TableTransform for Resolved {
    fn name(&self) -> &'static str {
        self.name
    }

    fn arguments(&self) -> Vec<Expr> {
        Vec::new()
    }

    fn with_arguments(&self, _arguments: Vec<Expr>) -> Result<Arc<dyn TableTransform>> {
        Ok(Arc::new(self.clone()))
    }

    fn schema(&self, _input: &DFSchema, _partition_by: &[Column]) -> Result<DFSchemaRef> {
        Ok(self.schema.clone())
    }

    fn partition_transform(&self, input: &DFSchema) -> Result<Arc<dyn PartitionTransform>> {
        // The transform reads its input's columns by position, so the input has the columns
        // that it had where the plan was written.
        if names(input) != self.input {
            return Err(invalid(format!(
                "a table function over the columns {:?}, which read {:?} where its plan was written",
                names(input),
                self.input
            )));
        }
        Ok(self.transform.clone())
    }

    fn fmt_for_explain(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self.transform)
    }
}

/// The bytes of `node` without its input, a [`Node`], when it is a [`TableFunctionNode`]: its
/// transform as the transform of each partition, so the plan's parameters are bound.
pub fn encode_node(node: &dyn UserDefinedLogicalNode) -> Option<Result<Vec<u8>>> {
    let node = node.as_any().downcast_ref::<TableFunctionNode>()?;
    Some((|| {
        let input = node.input.schema();
        let transform = node.transform.partition_transform(input)?;
        let partition_by = node.partition_by.iter();
        let schema = protobuf::DfSchema::try_from(&node.schema).map_err(invalid)?;
        let function = TableFunction {
            transform: Some(transform_state(transform.as_ref())?),
            partition_by: partition_by
                .map(|column| protobuf::Column::from(column).encode_to_vec())
                .collect(),
            schema: schema.encode_to_vec(),
            input: names(input),
        };
        let kind = Some(Kind::TableFunction(function));
        Ok(Node { kind }.encode_to_vec())
    })())
}

/// The node of `bytes`, a [`Node`], over its decoded inputs, when this build knows the node's
/// kind.
pub fn decode_node(
    bytes: &[u8],
    inputs: &[LogicalPlan],
) -> Option<Result<Arc<dyn UserDefinedLogicalNode>>> {
    let node = match Node::decode(bytes) {
        Ok(node) => node,
        Err(error) => return Some(Err(invalid(error))),
    };
    let Kind::TableFunction(function) = node.kind?;
    Some(table_function(function, inputs))
}

/// The [`TableFunctionNode`] of `function` over its decoded input, the one of `inputs`.
fn table_function(
    function: TableFunction,
    inputs: &[LogicalPlan],
) -> Result<Arc<dyn UserDefinedLogicalNode>> {
    let [input] = inputs else {
        return Err(invalid(format!(
            "{} inputs of a table function",
            inputs.len()
        )));
    };
    // The schema of a table function's result has columns, so its bytes are never empty.
    if function.schema.is_empty() {
        return Err(invalid("no schema"));
    }
    let schema = protobuf::DfSchema::decode(function.schema.as_slice()).map_err(invalid)?;
    let schema = DFSchemaRef::try_from(schema).map_err(invalid)?;
    let state = function
        .transform
        .ok_or_else(|| invalid("a table function whose transform this build does not know"))?;
    let mut bytes = Vec::new();
    state.encode(&mut bytes);
    let (name, transform): (_, Arc<dyn PartitionTransform>) = match &state {
        Transform::Impute(impute) => ("Impute", Arc::new(impute_of(impute)?)),
        Transform::MatchRecognize(matcher) => ("MatchRecognize", Arc::new(match_of(matcher)?)),
    };
    let resolved = Resolved {
        bytes,
        name,
        transform,
        schema,
        input: function.input,
    };
    let partition_by = (function.partition_by.iter())
        .map(|column| {
            let column = protobuf::Column::decode(column.as_slice()).map_err(invalid)?;
            Ok(Column::from(&column))
        })
        .collect::<Result<_>>()?;
    let node = TableFunctionNode::try_new(input.clone(), partition_by, Arc::new(resolved))?;
    Ok(Arc::new(node))
}

/// The message of `transform`, the transform of each partition of a table function.
fn transform_state(transform: &dyn PartitionTransform) -> Result<Transform> {
    let any = transform.as_any();
    if let Some(spec) = any.downcast_ref::<Spec>() {
        return Ok(Transform::Impute(impute_state(spec)?));
    }
    if let Some(matcher) = any.downcast_ref::<PartitionMatch>() {
        return Ok(Transform::MatchRecognize(match_state(matcher)));
    }
    Err(invalid(format!("no codec of the transform {transform:?}")))
}

/// The statistic `stat` as a message names it.
fn statistic(stat: Stat) -> Statistic {
    match stat {
        Stat::Avg => Statistic::Avg,
        Stat::Median => Statistic::Median,
        Stat::Min => Statistic::Min,
        Stat::Max => Statistic::Max,
    }
}

/// The statistic that the number `statistic` of a message names.
fn stat_of(statistic: i32) -> Result<Stat> {
    match Statistic::try_from(statistic) {
        Ok(Statistic::Avg) => Ok(Stat::Avg),
        Ok(Statistic::Median) => Ok(Stat::Median),
        Ok(Statistic::Min) => Ok(Stat::Min),
        Ok(Statistic::Max) => Ok(Stat::Max),
        Ok(Statistic::Unspecified) | Err(_) => Err(invalid(format!("the statistic {statistic}"))),
    }
}

/// The kind of gaps `kind` as a message names it.
fn gap_kind(kind: GapKind) -> proto::GapKind {
    match kind {
        GapKind::TimeUnit => proto::GapKind::TimeUnit,
        GapKind::Interval => proto::GapKind::Interval,
        GapKind::Date => proto::GapKind::Date,
        GapKind::Number => proto::GapKind::Number,
    }
}

/// The kind of gaps that the number `kind` of a message names.
fn gap_kind_of(kind: i32) -> Result<GapKind> {
    match proto::GapKind::try_from(kind) {
        Ok(proto::GapKind::TimeUnit) => Ok(GapKind::TimeUnit),
        Ok(proto::GapKind::Interval) => Ok(GapKind::Interval),
        Ok(proto::GapKind::Date) => Ok(GapKind::Date),
        Ok(proto::GapKind::Number) => Ok(GapKind::Number),
        Ok(proto::GapKind::Unspecified) | Err(_) => {
            Err(invalid(format!("the kind of gaps {kind}")))
        }
    }
}

/// What `impute` imputes, as [`Spec`] holds it.
fn impute_state(spec: &Spec) -> Result<Impute> {
    let mut state = Impute {
        key: spec.key as u64,
        groups: indices(&spec.groups),
        val: spec.val as u64,
        has_keys: spec.keys.is_some(),
        keys: values(spec.keys.as_deref().unwrap_or_default())?,
        extra_keys: values(&spec.extra_keys)?,
        ..Impute::default()
    };
    match &spec.fill {
        Fill::Value(fill) => state.fill_value = Some(scalar(fill)?),
        Fill::Stat { stat, frame } => {
            state.fill_stat = statistic(*stat).into();
            state.framed = frame.is_some();
            if let Some((start, end)) = frame {
                (state.frame_start, state.frame_end) = (*start, *end);
            }
        }
    }
    if let Some(gaps) = &spec.gaps {
        let (width, unit) = match &gaps.step {
            Some(Step::Width(width)) => (Some(scalar(width)?), None),
            Some(Step::Unit(unit)) => (None, Some(unit.clone())),
            None => (None, None),
        };
        state.gaps = Some(proto::Gaps {
            kind: gap_kind(gaps.kind).into(),
            start: gaps.start.as_ref().map(scalar).transpose()?,
            end: gaps.end.as_ref().map(scalar).transpose()?,
            width,
            unit,
        });
    }
    Ok(state)
}

fn impute_of(state: &Impute) -> Result<Spec> {
    let fill = match &state.fill_value {
        Some(fill) => Fill::Value(value(fill)?),
        None => Fill::Stat {
            stat: stat_of(state.fill_stat)?,
            frame: state.framed.then_some((state.frame_start, state.frame_end)),
        },
    };
    let gaps = match &state.gaps {
        Some(gaps) => Some(Gaps {
            kind: gap_kind_of(gaps.kind)?,
            start: gaps.start.as_deref().map(value).transpose()?,
            end: gaps.end.as_deref().map(value).transpose()?,
            step: match (&gaps.width, &gaps.unit) {
                (Some(width), _) => Some(Step::Width(value(width)?)),
                (None, Some(unit)) => Some(Step::Unit(unit.clone())),
                (None, None) => None,
            },
        }),
        None => None,
    };
    Ok(Spec {
        key: state.key as usize,
        groups: usizes(&state.groups),
        val: state.val as usize,
        fill,
        keys: state.has_keys.then(|| scalars(&state.keys)).transpose()?,
        extra_keys: scalars(&state.extra_keys)?,
        gaps,
    })
}

/// The matches of a partition, as [`PartitionMatch`] holds them, with the program's
/// instructions.
fn match_state(matcher: &PartitionMatch) -> MatchRecognize {
    let program = (matcher.program.insts.iter())
        .map(|inst| {
            let (operation, a, b) = match *inst {
                Inst::Label(variable) => (Operation::Label, variable, 0),
                Inst::Split(first, second) => (Operation::Split, first, second),
                Inst::Jump(to) => (Operation::Jump, to, 0),
                Inst::Anchor(Anchor::Start) => (Operation::AnchorStart, 0, 0),
                Inst::Anchor(Anchor::End) => (Operation::AnchorEnd, 0, 0),
                Inst::Match => (Operation::Match, 0, 0),
            };
            Instruction {
                operation: operation.into(),
                a: a as u64,
                b: b as u64,
            }
        })
        .collect();
    MatchRecognize {
        position: matcher.position as u64,
        variables: indices(&matcher.variables),
        names: matcher.names.clone(),
        program,
        skip_to_next_row: matcher.skip == Skip::ToNextRow,
        longest: matcher.longest,
    }
}

fn match_of(state: &MatchRecognize) -> Result<PartitionMatch> {
    let insts = (state.program.iter())
        .map(|inst| {
            let (a, b) = (inst.a as usize, inst.b as usize);
            Ok(match Operation::try_from(inst.operation) {
                Ok(Operation::Label) => Inst::Label(a),
                Ok(Operation::Split) => Inst::Split(a, b),
                Ok(Operation::Jump) => Inst::Jump(a),
                Ok(Operation::AnchorStart) => Inst::Anchor(Anchor::Start),
                Ok(Operation::AnchorEnd) => Inst::Anchor(Anchor::End),
                Ok(Operation::Match) => Inst::Match,
                Ok(Operation::Unspecified) | Err(_) => {
                    return Err(invalid(format!("the instruction {}", inst.operation)));
                }
            })
        })
        .collect::<Result<_>>()?;
    Ok(PartitionMatch {
        position: state.position as usize,
        variables: usizes(&state.variables),
        names: state.names.clone(),
        program: Program { insts },
        skip: match state.skip_to_next_row {
            true => Skip::ToNextRow,
            false => Skip::PastLastRow,
        },
        longest: state.longest,
    })
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hasher};

    use datafusion::logical_expr::EmptyRelation;

    use super::*;

    /// Each kind of state decodes from its bytes into the state that encoded it.
    #[test]
    fn each_transform_decodes_from_its_bytes() {
        let specs = [
            Spec {
                key: 0,
                groups: vec![1, 2],
                val: 3,
                fill: Fill::Value(ScalarValue::Int64(Some(0))),
                keys: None,
                extra_keys: Vec::new(),
                gaps: None,
            },
            Spec {
                key: 2,
                groups: Vec::new(),
                val: 0,
                fill: Fill::Stat {
                    stat: Stat::Median,
                    frame: Some((Some(-2), None)),
                },
                keys: Some(vec![ScalarValue::from("a"), ScalarValue::from("b")]),
                extra_keys: vec![ScalarValue::from("c")],
                gaps: Some(Gaps {
                    kind: GapKind::Number,
                    start: Some(ScalarValue::Float64(Some(0.5))),
                    end: None,
                    step: Some(Step::Width(ScalarValue::Float64(Some(2.0)))),
                }),
            },
            Spec {
                key: 1,
                groups: vec![0],
                val: 2,
                fill: Fill::Stat {
                    stat: Stat::Avg,
                    frame: None,
                },
                keys: Some(Vec::new()),
                extra_keys: Vec::new(),
                gaps: Some(Gaps {
                    kind: GapKind::TimeUnit,
                    start: None,
                    end: None,
                    step: Some(Step::Unit("month".into())),
                }),
            },
        ];
        for spec in &specs {
            let decoded = impute_of(&impute_state(spec).unwrap()).unwrap();
            assert_eq!(format!("{decoded:?}"), format!("{spec:?}"));
        }
        let matcher = PartitionMatch {
            position: 4,
            variables: vec![2, 3],
            names: vec!["up".into(), "down".into()],
            program: Program {
                insts: vec![
                    Inst::Anchor(Anchor::Start),
                    Inst::Split(2, 4),
                    Inst::Label(0),
                    Inst::Jump(1),
                    Inst::Label(1),
                    Inst::Anchor(Anchor::End),
                    Inst::Match,
                ],
            },
            skip: Skip::ToNextRow,
            longest: true,
        };
        let decoded = match_of(&match_state(&matcher)).unwrap();
        assert_eq!(format!("{decoded:?}"), format!("{matcher:?}"));
        let field = Field::new(
            "",
            DataType::Struct(
                vec![
                    Field::new("start", DataType::Float64, false),
                    Field::new("end", DataType::Float64, true),
                ]
                .into(),
            ),
            true,
        )
        .with_metadata([("kind".to_string(), "bin".to_string())].into());
        let udf = utils::struct_udf(&field);
        let bytes = encode_udf(&udf).unwrap().unwrap();
        assert_eq!(*decode_udf(udf.name(), &bytes).unwrap().unwrap(), udf);

        // The nodes that two readers build from the bytes of one node are equal and hash equally,
        // and encode into those bytes again. The nodes of two transforms over one input differ.
        let fields = vec![Field::new("month", DataType::Int64, true)];
        let schema = Arc::new(DFSchema::try_from(Schema::new(fields)).unwrap());
        let input = LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: false,
            schema: schema.clone(),
        });
        let bytes = |transform: Transform| {
            let column = protobuf::Column::from(&Column::from_name("month"));
            let function = TableFunction {
                transform: Some(transform),
                partition_by: vec![column.encode_to_vec()],
                schema: (protobuf::DfSchema::try_from(&schema).unwrap()).encode_to_vec(),
                input: names(&schema),
            };
            let kind = Some(Kind::TableFunction(function));
            Node { kind }.encode_to_vec()
        };
        let node =
            |bytes: &[u8]| (decode_node(bytes, std::slice::from_ref(&input)).unwrap()).unwrap();
        let hash = |node: &dyn UserDefinedLogicalNode| {
            let mut hasher = DefaultHasher::new();
            node.dyn_hash(&mut hasher);
            hasher.finish()
        };
        let impute = bytes(Transform::Impute(impute_state(&specs[1]).unwrap()));
        let matches = bytes(Transform::MatchRecognize(match_state(&matcher)));
        let (first, second) = (node(&impute), node(&impute));
        assert!(first.dyn_eq(second.as_ref()));
        assert_eq!(hash(first.as_ref()), hash(second.as_ref()));
        assert_eq!(encode_node(first.as_ref()).unwrap().unwrap(), impute);
        assert!(!first.dyn_eq(node(&matches).as_ref()));
        assert!(decode_node(&[], std::slice::from_ref(&input)).is_none());
    }
}
