use crate::{dataflow::*, *};
use crate::{Error, Result};
use dataflow::datafusion::common::ScalarValue;
use prost::Message;
use std::{num::NonZeroUsize, sync::Arc};

/// Versioned wire messages. Decode into `ChartDefinition` before evaluation.
pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/avenger.chart.rs"));
}
use protobuf as w;

fn artifact(e: impl std::fmt::Display) -> Error {
    Error::Artifact(e.to_string())
}
fn required<T>(v: Option<T>) -> Result<T> {
    v.ok_or_else(|| artifact("missing required chart field"))
}
fn scalar(v: &ScalarValue) -> Result<dataflow::protobuf::common::ScalarValue> {
    v.try_into().map_err(artifact)
}
fn unscalar(v: &dataflow::protobuf::common::ScalarValue) -> Result<ScalarValue> {
    v.try_into().map_err(artifact)
}
fn reference(v: dataflow::Reference) -> w::Reference {
    w::Reference {
        scope: v.scope,
        name: v.name,
    }
}
struct Encode(DataflowInterface);
struct Decode(DataflowInterface);

impl ChartDefinition {
    /// Encode the definition and dataflow, excluding runtime resources and caches.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.encode(self.dataflow.to_proto()?)
    }
    /// Encode application functions using the same codec as the dataflow.
    pub fn to_bytes_with_codec(
        &self,
        codec: Arc<dyn dataflow::LogicalExtensionCodec>,
    ) -> Result<Vec<u8>> {
        self.encode(self.dataflow.to_proto_with_codec(codec)?)
    }
    fn encode(&self, flow: dataflow::protobuf::DataflowArtifact) -> Result<Vec<u8>> {
        self.validate()?;
        let e = Encode(self.dataflow.interface());
        let parameters = self
            .parameters
            .iter()
            .map(|p| {
                Ok(w::Parameter {
                    name: p.name.clone(),
                    input: Some(reference(e.0.scalar_input_reference(&p.input)?)),
                    initial: p.initial.as_ref().map(scalar).transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(w::ChartArtifact {
            version: 1,
            dataflow: Some(flow),
            root: Some(e.group(&self.root)?),
            parameters,
        }
        .encode_to_vec())
    }
    /// Decode native descriptors and plans with the supplied dataflow runtime registry.
    pub fn from_bytes(bytes: &[u8], runtime: &Runtime) -> Result<Self> {
        let wire = w::ChartArtifact::decode(bytes).map_err(artifact)?;
        if wire.version != 1 {
            return Err(artifact(format!(
                "unsupported chart artifact version {}",
                wire.version
            )));
        }
        let dataflow = runtime.decode_dataflow_proto(required(wire.dataflow)?)?;
        let d = Decode(dataflow.interface());
        let parameters = wire
            .parameters
            .into_iter()
            .map(|p| {
                let r = required(p.input)?;
                Ok(Parameter {
                    name: p.name,
                    input: d.0.scope_at(&r.scope)?.scalar_input(&r.name)?,
                    initial: p.initial.as_ref().map(unscalar).transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        let definition = Self {
            root: d.group(required(wire.root)?, None)?,
            dataflow,
            parameters,
        };
        definition.validate()?;
        Ok(definition)
    }
}
impl Encode {
    fn table(&self, v: &TableOutput) -> Result<w::Reference> {
        Ok(reference(self.0.table_metadata(v)?.reference))
    }
    fn scalar(&self, v: &ScalarOutput) -> Result<w::Reference> {
        Ok(reference(self.0.scalar_metadata(v)?.reference))
    }
    fn text(&self, t: &Text) -> Result<w::Text> {
        Ok(w::Text {
            text: Some(match t {
                Text::Literal(v) => w::text::Text::Literal(v.clone()),
                Text::Key(v) => w::text::Text::Key(v.clone()),
                Text::Scalar(v) => w::text::Text::Scalar(self.scalar(v)?),
            }),
        })
    }
    fn group(&self, g: &Group) -> Result<w::Group> {
        Ok(w::Group {
            name: g.name.clone(),
            arrangement: Some(arrangement(&g.arrangement)?),
            title: g.title.as_ref().map(|v| self.text(v)).transpose()?,
            children: g
                .children
                .iter()
                .map(|n| {
                    Ok(w::Node {
                        node: Some(match n {
                            Node::Plot(p) => w::node::Node::Plot(self.plot(p)?),
                            Node::Group(g) => w::node::Node::Group(self.group(g)?),
                            Node::Facet {
                                name,
                                scope,
                                arrangement: a,
                                template,
                            } => w::node::Node::Facet(w::Facet {
                                name: name.clone(),
                                scope: self.0.scope_path(scope)?,
                                arrangement: Some(arrangement(a)?),
                                template: Some(self.group(template)?),
                            }),
                        }),
                    })
                })
                .collect::<Result<_>>()?,
            discovery: g.discovery.as_ref().map(|v| self.scalar(v)).transpose()?,
            descending_keys: matches!(g.key_order, KeyOrder::Descending),
            explicit_keys: match &g.key_order {
                KeyOrder::Explicit(v) => {
                    v.iter().map(|k| key(k.values())).collect::<Result<_>>()?
                }
                _ => vec![],
            },
        })
    }
    fn value(&self, v: &Value) -> Result<w::Value> {
        Ok(w::Value {
            value: Some(match v {
                Value::Constant(x) => w::value::Value::Constant(*x),
                Value::Field(x) => w::value::Value::Field(x.clone()),
                Value::Scalar(x) => w::value::Value::Scalar(self.scalar(x)?),
                Value::Scaled(s, x) => w::value::Value::Scaled(Box::new(w::Scaled {
                    scale: s.name.clone(),
                    input: Some(Box::new(self.value(x)?)),
                })),
                Value::Bandwidth(s) => w::value::Value::Bandwidth(s.name.clone()),
                Value::PlotWidth => w::value::Value::PlotWidth(true),
                Value::PlotHeight => w::value::Value::PlotHeight(true),
            }),
        })
    }
    fn optional_value(&self, v: &Option<Value>) -> Result<Option<w::Value>> {
        v.as_ref().map(|v| self.value(v)).transpose()
    }
    fn scale(&self, s: &Scale) -> Result<w::Scale> {
        Ok(w::Scale {
            kind: match s.kind {
                ScaleKind::Linear => 0,
                ScaleKind::Band => 1,
            },
            domain: Some(w::Domain {
                domain: Some(match &s.domain {
                    Domain::Column(t, c) => w::domain::Domain::Column(w::Column {
                        table: Some(self.table(t)?),
                        name: c.clone(),
                    }),
                    Domain::Extent(x) => w::domain::Domain::Extent(self.scalar(x)?),
                    Domain::Bounds(a, b) => w::domain::Domain::Bounds(w::Bounds {
                        min: Some(self.scalar(a)?),
                        max: Some(self.scalar(b)?),
                    }),
                    Domain::Values(v) => w::domain::Domain::Values(key(v)?),
                }),
            }),
            range: Some(w::Range {
                range: Some(match s.range {
                    Range::PlotWidth => w::range::Range::PlotWidth(true),
                    Range::PlotHeightReversed => w::range::Range::PlotHeightReversed(true),
                    Range::Fixed(start, end) => w::range::Range::Fixed(w::Interval { start, end }),
                }),
            }),
            zero: s.zero,
            nice: s.nice,
            clamp: s.clamp,
            padding_inner: s.padding_inner,
            padding_outer: s.padding_outer,
            empty_min: s.empty_domain[0],
            empty_max: s.empty_domain[1],
            sharing: s
                .sharing
                .as_ref()
                .map(|(name, s)| {
                    Ok::<_, Error>(w::Sharing {
                        name: name.clone(),
                        scope: Some(scope(s)?),
                    })
                })
                .transpose()?,
        })
    }
    fn plot(&self, p: &Plot) -> Result<w::Plot> {
        Ok(w::Plot {
            name: p.name.clone(),
            width: p.size.width,
            height: p.size.height,
            clip: p.clip,
            scales: p
                .scales
                .iter()
                .map(|(name, s)| {
                    Ok(w::NamedScale {
                        name: name.clone(),
                        scale: Some(self.scale(s)?),
                    })
                })
                .collect::<Result<_>>()?,
            marks: p
                .marks
                .iter()
                .map(|m| {
                    Ok(w::Mark {
                        name: m.name.clone(),
                        table: Some(self.table(&m.table)?),
                        encoding: Some(match &m.encoding {
                            Encoding::Rect(r) => w::mark::Encoding::Rect(w::Rect {
                                x: self.optional_value(&r.x)?,
                                y: self.optional_value(&r.y)?,
                                x2: self.optional_value(&r.x2)?,
                                y2: self.optional_value(&r.y2)?,
                                width: self.optional_value(&r.width)?,
                                height: self.optional_value(&r.height)?,
                                fill: r.fill.clone(),
                                interactive: r.interactive,
                            }),
                            Encoding::Symbol(s) => w::mark::Encoding::Symbol(w::Symbol {
                                x: self.optional_value(&s.x)?,
                                y: self.optional_value(&s.y)?,
                                size: Some(self.value(&s.size)?),
                                fill: s.fill.clone(),
                                interactive: s.interactive,
                            }),
                        }),
                    })
                })
                .collect::<Result<_>>()?,
            axes: p
                .axes
                .iter()
                .map(|a| {
                    Ok(w::Axis {
                        scale: a.scale.name.clone(),
                        side: match a.side {
                            Side::Bottom => 0,
                            Side::Top => 1,
                            Side::Left => 2,
                            Side::Right => 3,
                        },
                        title: a.title.clone(),
                        format: a.format.clone(),
                        tick_count: a.tick_count,
                        grid: a.grid,
                        labels: match a.labels {
                            LabelVisibility::All => 0,
                            LabelVisibility::Outer => 1,
                            LabelVisibility::None => 2,
                        },
                        sharing: Some(scope(&a.sharing)?),
                        shared_title: a.shared_title,
                    })
                })
                .collect::<Result<_>>()?,
            guide_reservations: p.guide_reservations.map(|e| w::Edges {
                top: e.top,
                right: e.right,
                bottom: e.bottom,
                left: e.left,
            }),
        })
    }
}
impl Decode {
    fn table(&self, r: w::Reference) -> Result<TableOutput> {
        Ok(self.0.scope_at(&r.scope)?.table_output(&r.name)?)
    }
    fn scalar(&self, r: w::Reference) -> Result<ScalarOutput> {
        Ok(self.0.scope_at(&r.scope)?.scalar_output(&r.name)?)
    }
    fn text(&self, t: w::Text) -> Result<Text> {
        Ok(match required(t.text)? {
            w::text::Text::Literal(v) => Text::Literal(v),
            w::text::Text::Key(v) => Text::Key(v),
            w::text::Text::Scalar(v) => Text::Scalar(self.scalar(v)?),
        })
    }
    fn group(&self, g: w::Group, current: Option<&ScopeHandle>) -> Result<Group> {
        Ok(Group {
            name: g.name,
            arrangement: unarrangement(required(g.arrangement)?)?,
            title: g.title.map(|v| self.text(v)).transpose()?,
            discovery: g.discovery.map(|v| self.scalar(v)).transpose()?,
            key_order: if !g.explicit_keys.is_empty() {
                KeyOrder::Explicit(
                    g.explicit_keys
                        .iter()
                        .map(|k| {
                            Ok(current
                                .ok_or_else(|| artifact("key ordering requires a facet scope"))?
                                .key(unkey(k)?)?)
                        })
                        .collect::<Result<_>>()?,
                )
            } else if g.descending_keys {
                KeyOrder::Descending
            } else {
                KeyOrder::Ascending
            },
            children: g
                .children
                .into_iter()
                .map(|n| {
                    Ok(match required(n.node)? {
                        w::node::Node::Plot(p) => Node::Plot(self.plot(p)?),
                        w::node::Node::Group(g) => Node::Group(self.group(g, current)?),
                        w::node::Node::Facet(f) => Node::Facet {
                            name: f.name,
                            scope: self
                                .0
                                .scope_at(&f.scope)?
                                .handle()
                                .ok_or_else(|| artifact("root cannot be a facet scope"))?
                                .clone(),
                            arrangement: unarrangement(required(f.arrangement)?)?,
                            template: self.group(
                                required(f.template)?,
                                self.0.scope_at(&f.scope)?.handle(),
                            )?,
                        },
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
    fn value(&self, v: w::Value, owner: u64) -> Result<Value> {
        Ok(match required(v.value)? {
            w::value::Value::Constant(x) => Value::Constant(x),
            w::value::Value::Field(x) => Value::Field(x),
            w::value::Value::Scalar(x) => Value::Scalar(self.scalar(x)?),
            w::value::Value::Scaled(s) => Value::Scaled(
                ScaleHandle {
                    owner,
                    name: s.scale,
                },
                Box::new(self.value(*required(s.input)?, owner)?),
            ),
            w::value::Value::Bandwidth(name) => Value::Bandwidth(ScaleHandle { owner, name }),
            w::value::Value::PlotWidth(_) => Value::PlotWidth,
            w::value::Value::PlotHeight(_) => Value::PlotHeight,
        })
    }
    fn optional_value(&self, v: Option<w::Value>, owner: u64) -> Result<Option<Value>> {
        v.map(|v| self.value(v, owner)).transpose()
    }
    fn scale(&self, s: w::Scale) -> Result<Scale> {
        Ok(Scale {
            kind: match s.kind {
                0 => ScaleKind::Linear,
                1 => ScaleKind::Band,
                _ => return Err(artifact("unknown scale kind")),
            },
            domain: match required(required(s.domain)?.domain)? {
                w::domain::Domain::Column(c) => {
                    Domain::Column(self.table(required(c.table)?)?, c.name)
                }
                w::domain::Domain::Extent(x) => Domain::Extent(self.scalar(x)?),
                w::domain::Domain::Bounds(b) => Domain::Bounds(
                    self.scalar(required(b.min)?)?,
                    self.scalar(required(b.max)?)?,
                ),
                w::domain::Domain::Values(k) => Domain::Values(unkey(&k)?),
            },
            range: match required(required(s.range)?.range)? {
                w::range::Range::PlotWidth(_) => Range::PlotWidth,
                w::range::Range::PlotHeightReversed(_) => Range::PlotHeightReversed,
                w::range::Range::Fixed(v) => Range::Fixed(v.start, v.end),
            },
            zero: s.zero,
            nice: s.nice,
            clamp: s.clamp,
            padding_inner: s.padding_inner,
            padding_outer: s.padding_outer,
            empty_domain: [s.empty_min, s.empty_max],
            sharing: s
                .sharing
                .map(|s| Ok::<_, Error>((s.name, unscope(required(s.scope)?)?)))
                .transpose()?,
        })
    }
    fn plot(&self, p: w::Plot) -> Result<Plot> {
        let owner = crate::model::plot_identity();
        Ok(Plot {
            identity: owner,
            name: p.name,
            size: Size::new(p.width, p.height),
            clip: p.clip,
            scales: p
                .scales
                .into_iter()
                .map(|s| Ok((s.name, self.scale(required(s.scale)?)?)))
                .collect::<Result<_>>()?,
            marks: p
                .marks
                .into_iter()
                .map(|m| {
                    Ok(Mark {
                        name: m.name,
                        table: self.table(required(m.table)?)?,
                        encoding: match required(m.encoding)? {
                            w::mark::Encoding::Rect(r) => Encoding::Rect(RectEncoding {
                                x: self.optional_value(r.x, owner)?,
                                y: self.optional_value(r.y, owner)?,
                                x2: self.optional_value(r.x2, owner)?,
                                y2: self.optional_value(r.y2, owner)?,
                                width: self.optional_value(r.width, owner)?,
                                height: self.optional_value(r.height, owner)?,
                                fill: r.fill,
                                interactive: r.interactive,
                            }),
                            w::mark::Encoding::Symbol(s) => Encoding::Symbol(SymbolEncoding {
                                x: self.optional_value(s.x, owner)?,
                                y: self.optional_value(s.y, owner)?,
                                size: self.value(required(s.size)?, owner)?,
                                fill: s.fill,
                                interactive: s.interactive,
                            }),
                        },
                    })
                })
                .collect::<Result<_>>()?,
            axes: p
                .axes
                .into_iter()
                .map(|a| {
                    Ok(Axis {
                        scale: ScaleHandle {
                            owner,
                            name: a.scale,
                        },
                        side: match a.side {
                            0 => Side::Bottom,
                            1 => Side::Top,
                            2 => Side::Left,
                            3 => Side::Right,
                            _ => return Err(artifact("unknown axis side")),
                        },
                        title: a.title,
                        format: a.format,
                        tick_count: a.tick_count,
                        grid: a.grid,
                        labels: match a.labels {
                            0 => LabelVisibility::All,
                            1 => LabelVisibility::Outer,
                            2 => LabelVisibility::None,
                            _ => return Err(artifact("unknown label visibility")),
                        },
                        sharing: unscope(required(a.sharing)?)?,
                        shared_title: a.shared_title,
                    })
                })
                .collect::<Result<_>>()?,
            guide_reservations: p.guide_reservations.map(|e| Edges {
                top: e.top,
                right: e.right,
                bottom: e.bottom,
                left: e.left,
            }),
        })
    }
}
fn key(values: &[ScalarValue]) -> Result<w::Key> {
    Ok(w::Key {
        values: values.iter().map(scalar).collect::<Result<_>>()?,
    })
}
fn unkey(k: &w::Key) -> Result<Vec<ScalarValue>> {
    k.values.iter().map(unscalar).collect()
}
fn scope(s: &PanelScope) -> Result<w::Scope> {
    Ok(w::Scope {
        scope: Some(match s {
            PanelScope::Panel => w::scope::Scope::Panel(true),
            PanelScope::Root => w::scope::Scope::Root(true),
            PanelScope::Ancestor(n) => {
                w::scope::Scope::Ancestor(n.get().try_into().map_err(artifact)?)
            }
            PanelScope::Group(_) => return Err(artifact("runtime group IDs are not portable")),
        }),
    })
}
fn unscope(s: w::Scope) -> Result<PanelScope> {
    Ok(match required(s.scope)? {
        w::scope::Scope::Panel(_) => PanelScope::Panel,
        w::scope::Scope::Root(_) => PanelScope::Root,
        w::scope::Scope::Ancestor(n) => PanelScope::Ancestor(
            NonZeroUsize::new(n as usize).ok_or_else(|| artifact("ancestor must be positive"))?,
        ),
    })
}
fn arrangement(a: &Arrangement) -> Result<w::Arrangement> {
    let (kind, row_count, column_count, slots) = match &a.kind {
        ArrangementKind::Column => (0, 0, 0, vec![]),
        ArrangementKind::Row => (1, 0, 0, vec![]),
        ArrangementKind::Wrap(n) => (2, 0, *n, vec![]),
        ArrangementKind::Grid {
            rows,
            columns,
            slots,
        } => (
            3,
            *rows,
            *columns,
            slots
                .iter()
                .map(|(name, s)| {
                    Ok(w::Slot {
                        name: name.clone(),
                        row: s.row.try_into().map_err(artifact)?,
                        column: s.column.try_into().map_err(artifact)?,
                        row_span: s.row_span.try_into().map_err(artifact)?,
                        column_span: s.column_span.try_into().map_err(artifact)?,
                    })
                })
                .collect::<Result<_>>()?,
        ),
    };
    let tracks = |v: &[TrackSize]| {
        v.iter()
            .map(|t| w::Track {
                track: Some(match t {
                    TrackSize::Auto => w::track::Track::Auto(true),
                    TrackSize::Fixed(v) => w::track::Track::Fixed(*v),
                    TrackSize::Flex(v) => w::track::Track::Flex(*v),
                }),
            })
            .collect()
    };
    Ok(w::Arrangement {
        kind,
        row_count: row_count.try_into().map_err(artifact)?,
        column_count: column_count.try_into().map_err(artifact)?,
        slots,
        gap: a.gap,
        margin: a.margin,
        uniform_columns: a.uniform_columns,
        uniform_rows: a.uniform_rows,
        share: a.share.clone(),
        columns: tracks(&a.columns),
        rows: tracks(&a.rows),
    })
}
fn unarrangement(a: w::Arrangement) -> Result<Arrangement> {
    let tracks = |v: Vec<w::Track>| {
        v.into_iter()
            .map(|t| {
                Ok(match required(t.track)? {
                    w::track::Track::Auto(_) => TrackSize::Auto,
                    w::track::Track::Fixed(v) => TrackSize::Fixed(v),
                    w::track::Track::Flex(v) => TrackSize::Flex(v),
                })
            })
            .collect::<Result<_>>()
    };
    Ok(Arrangement {
        kind: match a.kind {
            0 => ArrangementKind::Column,
            1 => ArrangementKind::Row,
            2 => ArrangementKind::Wrap(a.column_count as usize),
            3 => ArrangementKind::Grid {
                rows: a.row_count as usize,
                columns: a.column_count as usize,
                slots: a
                    .slots
                    .into_iter()
                    .map(|s| {
                        (
                            s.name,
                            GridSlot {
                                row: s.row as usize,
                                column: s.column as usize,
                                row_span: s.row_span as usize,
                                column_span: s.column_span as usize,
                            },
                        )
                    })
                    .collect(),
            },
            _ => return Err(artifact("unknown arrangement kind")),
        },
        gap: a.gap,
        margin: a.margin,
        uniform_columns: a.uniform_columns,
        uniform_rows: a.uniform_rows,
        share: a.share,
        columns: tracks(a.columns)?,
        rows: tracks(a.rows)?,
    })
}
