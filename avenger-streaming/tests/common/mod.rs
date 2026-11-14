//! Helpers of the tests: the table `events`, sessions of pointer events, and the batch plan of a
//! query over every event so far.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_sql::{
    analysis::{Options, analyze},
    catalog::MemoryCatalog,
    library::Library,
};
use avenger_streaming::EventLog;
use datafusion::{
    arrow::{
        array::{
            ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray,
            TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit},
        util::pretty::pretty_format_batches,
    },
    logical_expr::LogicalPlan,
    prelude::{SessionConfig, SessionContext},
};

/// The columns of a pointer event, as a chart records it, without `seq`, which the log assigns.
pub fn events_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("type", DataType::Utf8, false),
        Field::new("x", DataType::Float64, true),
        Field::new("y", DataType::Float64, true),
        Field::new("target", DataType::Utf8, true),
        Field::new("inside", DataType::Boolean, false),
    ]))
}

/// An event of a session.
#[derive(Clone)]
pub struct Event {
    pub ts: i64,
    pub kind: &'static str,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub target: Option<String>,
    pub inside: bool,
}

/// A xorshift generator.
pub struct Random(pub u64);

impl Random {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A session at 60 frames a second, as frames of the events of each 16.7 ms: hovering moves,
/// drags of 20 to 120 moves, clicks and double clicks on 20 targets, and pauses. With `odd`, a
/// position can be `NULL`, `NaN`, an infinity, or zero of either sign.
pub fn session(count: usize, seed: u64, odd: bool) -> Vec<Vec<Event>> {
    let mut random = Random(seed);
    let mut frames: Vec<Vec<Event>> = Vec::new();
    let mut frame = 0i64;
    let (mut x, mut y) = (400.0f64, 300.0f64);
    let mut total = 0;
    let push = |frames: &mut Vec<Vec<Event>>, frame: i64, event: Event| {
        let index = frame as usize;
        while frames.len() <= index {
            frames.push(Vec::new());
        }
        frames[index].push(event);
    };
    let at = |frame: i64| frame * 16_667;
    let position = |random: &mut Random, x: f64| -> Option<f64> {
        if !odd {
            return Some(x);
        }
        match random.below(40) {
            0 => None,
            1 => Some(f64::NAN),
            2 => Some(f64::INFINITY),
            3 => Some(f64::NEG_INFINITY),
            4 => Some(-0.0),
            5 => Some(0.0),
            _ => Some(x.round()),
        }
    };
    while total < count {
        let target = |random: &mut Random| Some(format!("t{}", random.below(20)));
        match random.below(100) {
            0..=7 => {
                let (px, py) = (position(&mut random, x), position(&mut random, y));
                push(
                    &mut frames,
                    frame,
                    Event {
                        ts: at(frame),
                        kind: "pointerdown",
                        x: px,
                        y: py,
                        target: None,
                        inside: true,
                    },
                );
                let moves = 20 + random.below(100);
                for _ in 0..moves {
                    frame += 1;
                    x += random.unit() * 10.0 - 5.0;
                    y += random.unit() * 10.0 - 5.0;
                    let (px, py) = (position(&mut random, x), position(&mut random, y));
                    push(
                        &mut frames,
                        frame,
                        Event {
                            ts: at(frame),
                            kind: "pointermove",
                            x: px,
                            y: py,
                            target: None,
                            inside: true,
                        },
                    );
                }
                frame += 1;
                let (px, py) = (position(&mut random, x), position(&mut random, y));
                push(
                    &mut frames,
                    frame,
                    Event {
                        ts: at(frame),
                        kind: "pointerup",
                        x: px,
                        y: py,
                        target: None,
                        inside: true,
                    },
                );
                total += moves as usize + 2;
            }
            8..=12 => {
                let t = target(&mut random);
                let double = random.below(4) == 0;
                for kind in ["pointerdown", "pointerup", "click"] {
                    let (px, py) = (position(&mut random, x), position(&mut random, y));
                    push(
                        &mut frames,
                        frame,
                        Event {
                            ts: at(frame),
                            kind,
                            x: px,
                            y: py,
                            target: t.clone(),
                            inside: true,
                        },
                    );
                }
                total += 3;
                if double {
                    frame += 10;
                    for kind in ["pointerdown", "pointerup", "click", "dblclick"] {
                        let (px, py) = (position(&mut random, x), position(&mut random, y));
                        push(
                            &mut frames,
                            frame,
                            Event {
                                ts: at(frame),
                                kind,
                                x: px,
                                y: py,
                                target: t.clone(),
                                inside: true,
                            },
                        );
                    }
                    total += 4;
                }
            }
            13..=14 => frame += 30 + random.below(60) as i64,
            _ => {
                x += random.unit() * 20.0 - 10.0;
                y += random.unit() * 20.0 - 10.0;
                let (px, py) = (position(&mut random, x), position(&mut random, y));
                let inside = random.below(10) > 0;
                push(
                    &mut frames,
                    frame,
                    Event {
                        ts: at(frame),
                        kind: "pointermove",
                        x: px,
                        y: py,
                        target: None,
                        inside,
                    },
                );
                total += 1;
            }
        }
        frame += 1;
    }
    frames
}

/// The frames as batches of events, which the log numbers.
pub fn batches(frames: &[Vec<Event>]) -> Vec<RecordBatch> {
    frames
        .iter()
        .map(|events| {
            let columns: Vec<ArrayRef> = vec![
                Arc::new(
                    TimestampMicrosecondArray::from_iter_values(events.iter().map(|e| e.ts))
                        .with_timezone("UTC"),
                ),
                Arc::new(StringArray::from_iter_values(events.iter().map(|e| e.kind))),
                Arc::new(Float64Array::from_iter(events.iter().map(|e| e.x))),
                Arc::new(Float64Array::from_iter(events.iter().map(|e| e.y))),
                Arc::new(StringArray::from_iter(
                    events.iter().map(|e| e.target.clone()),
                )),
                Arc::new(BooleanArray::from_iter(
                    events.iter().map(|e| Some(e.inside)),
                )),
            ];
            RecordBatch::try_new(events_schema(), columns).unwrap()
        })
        .collect()
}

/// A session of one partition, with the transforms.
pub fn context() -> SessionContext {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
    avenger_sql::register(&ctx).unwrap();
    ctx
}

/// A new log of pointer events.
pub fn log() -> EventLog {
    EventLog::new(events_schema()).unwrap()
}

/// The plan of `query` over `log` as `events`.
pub fn plan(query: &str, log: &EventLog) -> LogicalPlan {
    let catalog = MemoryCatalog::new().with_table(&["events"], log.table());
    let parse = avenger_sql_syntax::parse(query);
    assert!(parse.ok(), "{query}: {:?}", parse.diagnostics());
    let analysis = analyze(
        &parse.syntax(),
        &catalog,
        &Library::new(),
        &Options::default(),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{query}: {:?}",
        analysis.diagnostics
    );
    analysis.plan.expect("a plan")
}

/// The batch plan's rows of `plan` over the log's events so far, with `params`.
pub async fn batch_rows(
    ctx: &SessionContext,
    plan: &LogicalPlan,
    params: Option<datafusion::common::ParamValues>,
) -> Vec<String> {
    let plan = match params {
        Some(params) => plan.clone().with_param_values(params).unwrap(),
        None => plan.clone(),
    };
    rows(
        &ctx.execute_logical_plan(plan)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    )
}

/// The rows of `batches` as sorted lines, which compare results without their order.
pub fn rows(batches: &[RecordBatch]) -> Vec<String> {
    if batches.iter().all(|b| b.num_rows() == 0) {
        return Vec::new();
    }
    let text = pretty_format_batches(batches).unwrap().to_string();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| !line.starts_with('+'))
        .map(String::from)
        .collect();
    let header = lines.remove(0);
    lines.sort();
    lines.insert(0, header);
    lines
}
