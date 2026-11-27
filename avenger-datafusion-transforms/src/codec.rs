//! The transforms in serialized physical plans, such as the plans of a page's graph: each
//! function travels by its name and the bytes of its state, and a [`TableFunctionExec`] by the
//! bytes of its transform, its partition columns, and its schema, with its input as its child. A
//! front end that serializes its plans, such as avenger-sql, calls these from its codec.

use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    common::{Result, ScalarValue, exec_datafusion_err},
    logical_expr::{AggregateUDF, ScalarUDF, WindowUDF},
    physical_plan::ExecutionPlan,
};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    bin,
    impute::{Fill, GapKind, Gaps, Spec, Stat, Step},
    match_recognize::{Anchor, Program, Skip, node::PartitionMatch, pattern::Inst},
    stack,
    table_function::{PartitionTransform, TableFunctionExec},
    timeunit::functions as timeunit,
    utils::{self, MakeStruct},
};

fn invalid(error: impl std::fmt::Display) -> datafusion::common::DataFusionError {
    exec_datafusion_err!("the bytes of a transform: {error}")
}

fn scalar(value: &ScalarValue) -> Result<protobuf::ScalarValue> {
    protobuf::ScalarValue::try_from(value).map_err(invalid)
}

fn value(proto: &protobuf::ScalarValue) -> Result<ScalarValue> {
    ScalarValue::try_from(proto).map_err(invalid)
}

fn values(values: &[ScalarValue]) -> Result<Vec<protobuf::ScalarValue>> {
    values.iter().map(scalar).collect()
}

fn scalars(protos: &[protobuf::ScalarValue]) -> Result<Vec<ScalarValue>> {
    protos.iter().map(value).collect()
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

/// A struct of a field, as `MakeStruct` makes it.
#[derive(Clone, PartialEq, Message)]
struct StructBytes {
    #[prost(message, optional, tag = "1")]
    fields: Option<protobuf::Schema>,
    #[prost(bool, tag = "2")]
    nullable: bool,
    #[prost(map = "string, string", tag = "3")]
    metadata: HashMap<String, String>,
}

/// The bytes of the state of `udf`, when it is one of the transforms' scalar functions.
pub fn encode_udf(udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
    let inner = udf.inner().as_ref() as &dyn std::any::Any;
    if let Some(make) = inner.downcast_ref::<MakeStruct>() {
        let schema = Schema::new(make.fields.clone());
        return Some(
            protobuf::Schema::try_from(&schema)
                .map_err(invalid)
                .map(|fields| {
                    StructBytes {
                        fields: Some(fields),
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
            let decoded = StructBytes::decode(bytes).map_err(invalid)?;
            let fields = decoded
                .fields
                .ok_or_else(|| invalid("a struct without fields"))?;
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

/// The first byte of the bytes of a [`TableFunctionExec`].
const TABLE_FUNCTION: u8 = 1;

/// A [`TableFunctionExec`] without its input: its transform by its kind and its bytes, its
/// partition columns, and its schema.
#[derive(Clone, PartialEq, Message)]
struct TableFunctionBytes {
    #[prost(string, tag = "1")]
    kind: String,
    #[prost(bytes = "vec", tag = "2")]
    transform: Vec<u8>,
    #[prost(uint64, repeated, tag = "3")]
    partitions: Vec<u64>,
    #[prost(message, optional, tag = "4")]
    schema: Option<protobuf::Schema>,
}

const IMPUTE: &str = "impute";
const MATCH_RECOGNIZE: &str = "match_recognize";

/// The bytes of `plan` without its input, when it is a [`TableFunctionExec`].
pub fn encode_plan(plan: &dyn ExecutionPlan) -> Option<Result<Vec<u8>>> {
    let exec = (plan as &dyn std::any::Any).downcast_ref::<TableFunctionExec>()?;
    Some((|| {
        let (kind, transform) = transform_bytes(exec.transform.as_ref())?;
        let bytes = TableFunctionBytes {
            kind: kind.to_string(),
            transform,
            partitions: indices(&exec.partitions),
            schema: Some(protobuf::Schema::try_from(exec.schema.as_ref()).map_err(invalid)?),
        };
        let mut buf = vec![TABLE_FUNCTION];
        bytes.encode(&mut buf).map_err(invalid)?;
        Ok(buf)
    })())
}

/// The [`TableFunctionExec`] of `bytes` over its decoded input, the one of `children`, when
/// `bytes` are a table function's.
pub fn decode_plan(
    bytes: &[u8],
    children: &[Arc<dyn ExecutionPlan>],
) -> Option<Result<Arc<dyn ExecutionPlan>>> {
    let (&TABLE_FUNCTION, bytes) = bytes.split_first()? else {
        return None;
    };
    Some((|| {
        let decoded = TableFunctionBytes::decode(bytes).map_err(invalid)?;
        let [input] = children else {
            return Err(invalid(format!(
                "{} inputs of a table function",
                children.len()
            )));
        };
        let schema = decoded.schema.ok_or_else(|| invalid("no schema"))?;
        let schema = Arc::new(Schema::try_from(&schema).map_err(invalid)?);
        let transform: Arc<dyn PartitionTransform> = match decoded.kind.as_str() {
            IMPUTE => Arc::new(impute_of(&decoded.transform)?),
            MATCH_RECOGNIZE => Arc::new(match_of(&decoded.transform)?),
            kind => return Err(invalid(format!("the transform {kind}"))),
        };
        let exec = TableFunctionExec::new(
            input.clone(),
            transform,
            usizes(&decoded.partitions),
            schema,
        );
        Ok(Arc::new(exec) as Arc<dyn ExecutionPlan>)
    })())
}

fn transform_bytes(transform: &dyn PartitionTransform) -> Result<(&'static str, Vec<u8>)> {
    let any = transform.as_any();
    if let Some(spec) = any.downcast_ref::<Spec>() {
        return Ok((IMPUTE, impute_bytes(spec)?));
    }
    if let Some(matcher) = any.downcast_ref::<PartitionMatch>() {
        return Ok((MATCH_RECOGNIZE, match_bytes(matcher)));
    }
    Err(invalid(format!("no codec of the transform {transform:?}")))
}

/// What `impute` imputes, as [`Spec`] holds it.
#[derive(Clone, PartialEq, Message)]
struct ImputeBytes {
    #[prost(uint64, tag = "1")]
    key: u64,
    #[prost(uint64, repeated, tag = "2")]
    groups: Vec<u64>,
    #[prost(uint64, tag = "3")]
    val: u64,
    /// The value of `value =>`, or none for a statistic.
    #[prost(message, optional, tag = "4")]
    fill_value: Option<protobuf::ScalarValue>,
    /// The statistic of `method =>`: avg, median, min, or max.
    #[prost(string, tag = "5")]
    fill_stat: String,
    #[prost(bool, tag = "6")]
    framed: bool,
    #[prost(int64, optional, tag = "7")]
    frame_start: Option<i64>,
    #[prost(int64, optional, tag = "8")]
    frame_end: Option<i64>,
    #[prost(bool, tag = "9")]
    has_keys: bool,
    #[prost(message, repeated, tag = "10")]
    keys: Vec<protobuf::ScalarValue>,
    #[prost(message, repeated, tag = "11")]
    extra_keys: Vec<protobuf::ScalarValue>,
    #[prost(message, optional, tag = "12")]
    gaps: Option<GapsBytes>,
}

/// The gaps that `fill_gaps` fills, as [`Gaps`] holds them.
#[derive(Clone, PartialEq, Message)]
struct GapsBytes {
    /// time_unit, interval, date, or number.
    #[prost(string, tag = "1")]
    kind: String,
    #[prost(message, optional, tag = "2")]
    start: Option<protobuf::ScalarValue>,
    #[prost(message, optional, tag = "3")]
    end: Option<protobuf::ScalarValue>,
    #[prost(message, optional, tag = "4")]
    width: Option<protobuf::ScalarValue>,
    #[prost(string, optional, tag = "5")]
    unit: Option<String>,
}

const STATS: [(Stat, &str); 4] = [
    (Stat::Avg, "avg"),
    (Stat::Median, "median"),
    (Stat::Min, "min"),
    (Stat::Max, "max"),
];

const GAP_KINDS: [(GapKind, &str); 4] = [
    (GapKind::TimeUnit, "time_unit"),
    (GapKind::Interval, "interval"),
    (GapKind::Date, "date"),
    (GapKind::Number, "number"),
];

fn impute_bytes(spec: &Spec) -> Result<Vec<u8>> {
    let mut bytes = ImputeBytes {
        key: spec.key as u64,
        groups: indices(&spec.groups),
        val: spec.val as u64,
        has_keys: spec.keys.is_some(),
        keys: values(spec.keys.as_deref().unwrap_or_default())?,
        extra_keys: values(&spec.extra_keys)?,
        ..ImputeBytes::default()
    };
    match &spec.fill {
        Fill::Value(fill) => bytes.fill_value = Some(scalar(fill)?),
        Fill::Stat { stat, frame } => {
            bytes.fill_stat = (STATS.iter())
                .find(|(known, _)| known == stat)
                .map(|(_, name)| name.to_string())
                .expect("a statistic of STATS");
            bytes.framed = frame.is_some();
            if let Some((start, end)) = frame {
                (bytes.frame_start, bytes.frame_end) = (*start, *end);
            }
        }
    }
    if let Some(gaps) = &spec.gaps {
        let (width, unit) = match &gaps.step {
            Some(Step::Width(width)) => (Some(scalar(width)?), None),
            Some(Step::Unit(unit)) => (None, Some(unit.clone())),
            None => (None, None),
        };
        bytes.gaps = Some(GapsBytes {
            kind: (GAP_KINDS.iter())
                .find(|(known, _)| *known == gaps.kind)
                .map(|(_, name)| name.to_string())
                .expect("a kind of GAP_KINDS"),
            start: gaps.start.as_ref().map(scalar).transpose()?,
            end: gaps.end.as_ref().map(scalar).transpose()?,
            width,
            unit,
        });
    }
    Ok(bytes.encode_to_vec())
}

fn impute_of(bytes: &[u8]) -> Result<Spec> {
    let decoded = ImputeBytes::decode(bytes).map_err(invalid)?;
    let fill = match &decoded.fill_value {
        Some(fill) => Fill::Value(value(fill)?),
        None => Fill::Stat {
            stat: (STATS.iter())
                .find(|(_, name)| *name == decoded.fill_stat)
                .map(|(stat, _)| *stat)
                .ok_or_else(|| invalid(format!("the statistic {}", decoded.fill_stat)))?,
            frame: decoded
                .framed
                .then_some((decoded.frame_start, decoded.frame_end)),
        },
    };
    let gaps = match &decoded.gaps {
        Some(gaps) => Some(Gaps {
            kind: (GAP_KINDS.iter())
                .find(|(_, name)| *name == gaps.kind)
                .map(|(kind, _)| *kind)
                .ok_or_else(|| invalid(format!("the kind of gaps {}", gaps.kind)))?,
            start: gaps.start.as_ref().map(value).transpose()?,
            end: gaps.end.as_ref().map(value).transpose()?,
            step: match (&gaps.width, &gaps.unit) {
                (Some(width), _) => Some(Step::Width(value(width)?)),
                (None, Some(unit)) => Some(Step::Unit(unit.clone())),
                (None, None) => None,
            },
        }),
        None => None,
    };
    Ok(Spec {
        key: decoded.key as usize,
        groups: usizes(&decoded.groups),
        val: decoded.val as usize,
        fill,
        keys: decoded
            .has_keys
            .then(|| scalars(&decoded.keys))
            .transpose()?,
        extra_keys: scalars(&decoded.extra_keys)?,
        gaps,
    })
}

/// The matches of a partition, as [`PartitionMatch`] holds them, with the program's
/// instructions.
#[derive(Clone, PartialEq, Message)]
struct MatchBytes {
    #[prost(uint64, tag = "1")]
    position: u64,
    #[prost(uint64, repeated, tag = "2")]
    variables: Vec<u64>,
    #[prost(string, repeated, tag = "3")]
    names: Vec<String>,
    #[prost(message, repeated, tag = "4")]
    program: Vec<InstBytes>,
    #[prost(bool, tag = "5")]
    skip_to_next_row: bool,
    #[prost(bool, tag = "6")]
    longest: bool,
}

/// An instruction of a pattern's program: a label, a split, a jump, an anchor, or the match, by
/// `op`, with its operands.
#[derive(Clone, PartialEq, Message)]
struct InstBytes {
    #[prost(uint32, tag = "1")]
    op: u32,
    #[prost(uint64, tag = "2")]
    a: u64,
    #[prost(uint64, tag = "3")]
    b: u64,
}

fn match_bytes(matcher: &PartitionMatch) -> Vec<u8> {
    let program = (matcher.program.insts.iter())
        .map(|inst| {
            let (op, a, b) = match *inst {
                Inst::Label(variable) => (0, variable, 0),
                Inst::Split(first, second) => (1, first, second),
                Inst::Jump(to) => (2, to, 0),
                Inst::Anchor(Anchor::Start) => (3, 0, 0),
                Inst::Anchor(Anchor::End) => (3, 1, 0),
                Inst::Match => (4, 0, 0),
            };
            InstBytes {
                op,
                a: a as u64,
                b: b as u64,
            }
        })
        .collect();
    MatchBytes {
        position: matcher.position as u64,
        variables: indices(&matcher.variables),
        names: matcher.names.clone(),
        program,
        skip_to_next_row: matcher.skip == Skip::ToNextRow,
        longest: matcher.longest,
    }
    .encode_to_vec()
}

fn match_of(bytes: &[u8]) -> Result<PartitionMatch> {
    let decoded = MatchBytes::decode(bytes).map_err(invalid)?;
    let insts = (decoded.program.iter())
        .map(|inst| {
            let (a, b) = (inst.a as usize, inst.b as usize);
            Ok(match (inst.op, a) {
                (0, _) => Inst::Label(a),
                (1, _) => Inst::Split(a, b),
                (2, _) => Inst::Jump(a),
                (3, 0) => Inst::Anchor(Anchor::Start),
                (3, 1) => Inst::Anchor(Anchor::End),
                (4, _) => Inst::Match,
                (op, _) => return Err(invalid(format!("the instruction {op}"))),
            })
        })
        .collect::<Result<_>>()?;
    Ok(PartitionMatch {
        position: decoded.position as usize,
        variables: usizes(&decoded.variables),
        names: decoded.names,
        program: Program { insts },
        skip: match decoded.skip_to_next_row {
            true => Skip::ToNextRow,
            false => Skip::PastLastRow,
        },
        longest: decoded.longest,
    })
}

#[cfg(test)]
mod tests {
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
        for spec in specs {
            let decoded = impute_of(&impute_bytes(&spec).unwrap()).unwrap();
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
        let decoded = match_of(&match_bytes(&matcher)).unwrap();
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
    }
}
