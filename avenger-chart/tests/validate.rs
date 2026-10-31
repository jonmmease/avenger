//! The rules of the format: charts that keep them pass, and a chart that breaks one fails with
//! the path of the part that breaks it.

use std::{cell::RefCell, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use avenger_chart::{
    payload,
    proto::{
        self, axis, category_fit, condition, coord, domain, domain_fit, encoding, facet, geo,
        guide, mark, node, predicate, range, tuple_field, Channel,
    },
    Chart, ChartError, FORMAT_VERSION,
};
use avenger_scales::scales::domain_summary::{DomainSummary, SampleSummary};
use bytes::Bytes;

thread_local! {
    /// The values that the messages of a test refer to, which `chart` takes.
    static VALUES: RefCell<Vec<ArrayRef>> = const { RefCell::new(Vec::new()) };
}

/// The index of `value`, added to the values that the next chart takes.
fn push(value: ArrayRef) -> u32 {
    VALUES.with_borrow_mut(|values| {
        values.push(value);
        (values.len() - 1) as u32
    })
}

fn number(value: f64) -> u32 {
    push(Arc::new(Float64Array::from(vec![value])))
}

fn text(value: &str) -> u32 {
    push(Arc::new(StringArray::from(vec![value])))
}

fn numbers(values: &[f64]) -> u32 {
    push(payload::list(Arc::new(Float64Array::from(values.to_vec()))))
}

fn texts(values: &[&str]) -> u32 {
    push(payload::list(Arc::new(StringArray::from(values.to_vec()))))
}

fn data_domain() -> Option<proto::Domain> {
    Some(proto::Domain {
        domain: Some(domain::Domain::Data(proto::DataDomain {})),
    })
}

fn explicit(index: u32) -> Option<proto::Domain> {
    Some(proto::Domain {
        domain: Some(domain::Domain::Explicit(index)),
    })
}

fn interval_fit() -> Option<proto::DomainFit> {
    Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Interval(proto::IntervalFit::default())),
    })
}

fn category_fit() -> Option<proto::DomainFit> {
    Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Categories(proto::CategoryFit {
            order: Some(category_fit::Order::Sort(
                proto::CategorySort::Ascending as i32,
            )),
        })),
    })
}

/// A spec of `scale_type` with a data domain and the fit that it needs.
fn spec(scale_type: &str) -> proto::ScaleSpec {
    let fit = match scale_type {
        "band" | "point" | "ordinal" | "nested_band" => category_fit(),
        "quantile" | "threshold" => None,
        _ => interval_fit(),
    };
    proto::ScaleSpec {
        r#type: scale_type.into(),
        domain: data_domain(),
        fit,
        ..Default::default()
    }
}

/// `channel` from the column `name` without a scale.
fn raw(channel: Channel, name: &str) -> proto::Encoding {
    proto::Encoding {
        channel: channel as i32,
        value: Some(encoding::Value::Column(name.into())),
        ..Default::default()
    }
}

/// `channel` from the column `name` through a scale of `scale_type`.
fn scaled(channel: Channel, name: &str, scale_type: &str) -> proto::Encoding {
    proto::Encoding {
        scale: Some(spec(scale_type)),
        ..raw(channel, name)
    }
}

fn constant(channel: Channel, value: u32) -> proto::Encoding {
    proto::Encoding {
        channel: channel as i32,
        value: Some(encoding::Value::Constant(value)),
        ..Default::default()
    }
}

fn unit(mark: mark::Mark, encodings: Vec<proto::Encoding>) -> proto::Unit {
    proto::Unit {
        mark: Some(proto::Mark { mark: Some(mark) }),
        encodings,
        ..Default::default()
    }
}

fn symbol() -> mark::Mark {
    mark::Mark::Symbol(proto::SymbolMark::default())
}

/// Symbols at `x` and `y`, colored by `group`.
fn scatter() -> proto::Unit {
    unit(
        symbol(),
        vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            scaled(Channel::Color, "group", "ordinal"),
        ],
    )
}

fn polar() -> Option<proto::Coord> {
    Some(proto::Coord {
        coord: Some(coord::Coord::Polar(proto::Polar::default())),
    })
}

fn map() -> Option<proto::Coord> {
    Some(proto::Coord {
        coord: Some(coord::Coord::Geo(proto::Geo {
            surface: Some(geo::Surface::Spherical(proto::Spherical {
                projection: "mercator".into(),
                ..Default::default()
            })),
            fit: Some(geo::Fit::FitData(proto::PlotData {})),
            ..Default::default()
        })),
    })
}

fn leaf(unit: proto::Unit) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Unit(unit)),
    }
}

fn overlay(children: Vec<proto::Node>) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Overlay(proto::Overlay {
            children,
            ..Default::default()
        })),
    }
}

fn concat(direction: proto::Direction, children: Vec<proto::Node>) -> proto::Concat {
    proto::Concat {
        direction: direction as i32,
        children,
        ..Default::default()
    }
}

fn concat_node(concat: proto::Concat) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Concat(concat)),
    }
}

fn partition(column: &str, value: &str) -> proto::PartitionValue {
    proto::PartitionValue {
        column: column.into(),
        value: text(value),
    }
}

fn select(name: &str) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Selection(proto::SelectionTest {
            selection: name.into(),
            ..Default::default()
        })),
    }
}

fn selection(name: &str, initial: Vec<proto::Contribution>) -> proto::Selection {
    proto::Selection {
        name: name.into(),
        resolution: proto::SelectionResolution::Union as i32,
        initial,
    }
}

/// A contribution of `owner` that selects the group `a`.
fn contribution(owner: &str, partition: Vec<proto::PartitionValue>) -> proto::Contribution {
    proto::Contribution {
        owner: owner.into(),
        tuples: vec![proto::Tuple {
            fields: vec![proto::TupleField {
                test: Some(tuple_field::Test::Set(proto::ValueSet {
                    column: "group".into(),
                    channel: Channel::Color as i32,
                    values: texts(&["a"]),
                })),
            }],
        }],
        partition,
    }
}

/// A table of four rows in the groups `a` and `b`.
fn table() -> RecordBatch {
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("x", Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0]))),
        ("x2", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]))),
        ("y", Arc::new(Float64Array::from(vec![4.0, 2.0, 5.0, 1.0]))),
        (
            "group",
            Arc::new(StringArray::from(vec!["a", "a", "b", "b"])),
        ),
        (
            "css",
            Arc::new(StringArray::from(vec!["red", "red", "blue", "blue"])),
        ),
        (
            "width",
            Arc::new(Float64Array::from(vec![1.0, 1.0, 2.0, 2.0])),
        ),
        ("count", Arc::new(Int64Array::from(vec![3, 1, 4, 1]))),
        (
            "lon",
            Arc::new(Float64Array::from(vec![-70.0, -60.0, 0.0, 10.0])),
        ),
        (
            "lat",
            Arc::new(Float64Array::from(vec![40.0, 45.0, 50.0, 55.0])),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

fn chart(root: proto::Node, selections: Vec<proto::Selection>) -> Result<Chart, ChartError> {
    let message = proto::Chart {
        version: FORMAT_VERSION,
        root: Some(root),
        selections,
        ..Default::default()
    };
    Chart::new(message, vec![table()], VALUES.take())
}

/// Asserts that `root` with `selections` fails at `path` for a reason that contains `reason`.
#[track_caller]
fn fails(root: proto::Node, selections: Vec<proto::Selection>, path: &str, reason: &str) {
    match chart(root, selections) {
        Err(ChartError::Invalid {
            path: actual,
            reason: actual_reason,
        }) => {
            assert_eq!(actual, path, "{actual_reason}");
            assert!(
                actual_reason.contains(reason),
                "`{actual_reason}` does not contain `{reason}`"
            );
        }
        other => panic!("expected an invalid chart at {path}, got {other:?}"),
    }
}

/// Asserts that a unit, after `change`, fails at `path`.
#[track_caller]
fn unit_fails(change: impl FnOnce(&mut proto::Unit), path: &str, reason: &str) {
    let mut unit = scatter();
    change(&mut unit);
    fails(leaf(unit), vec![], path, reason);
}

#[test]
fn charts_that_keep_the_rules_pass() {
    chart(leaf(scatter()), vec![]).unwrap();

    // A line whose stroke width holds one value in each series of its color.
    chart(
        leaf(unit(
            mark::Mark::Line(proto::LineMark::default()),
            vec![
                scaled(Channel::X, "x", "linear"),
                scaled(Channel::Y, "y", "linear"),
                scaled(Channel::Color, "group", "ordinal"),
                raw(Channel::StrokeWidth, "width"),
            ],
        )),
        vec![],
    )
    .unwrap();

    // Slices of a pie: rects from `theta` to `theta2` that span the range of `radius`.
    let mut pie = unit(
        mark::Mark::Rect(proto::RectMark::default()),
        vec![
            raw(Channel::Fill, "css"),
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Theta2, "x2"),
        ],
    );
    pie.coord = polar();
    chart(leaf(pie), vec![]).unwrap();

    // Symbols and a rule on a map.
    let mut places = unit(
        symbol(),
        vec![
            raw(Channel::Longitude, "lon"),
            raw(Channel::Latitude, "lat"),
        ],
    );
    places.coord = map();
    let mut route = unit(
        mark::Mark::Rule(proto::RuleMark::default()),
        vec![
            raw(Channel::Longitude, "lon"),
            raw(Channel::Latitude, "lat"),
        ],
    );
    route.coord = map();
    chart(overlay(vec![leaf(places), leaf(route)]), vec![]).unwrap();

    // An overlay takes the coordinate system that its children set.
    let mut ring = unit(
        symbol(),
        vec![
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Radius, "width"),
        ],
    );
    let spokes = ring.clone();
    ring.coord = polar();
    chart(overlay(vec![leaf(ring), leaf(spokes)]), vec![]).unwrap();
}

#[test]
fn facets_partitions_and_selections_that_keep_the_rules_pass() {
    // A row of the facets `left` and `right`, whose plots share an id, with a selection that the
    // left plot owns and that colors the rows outside it gray.
    let panel = |value: &str| {
        let mut unit = scatter();
        unit.id = "plot".into();
        unit.partition = vec![partition("panel", value)];
        unit.interactions = vec![proto::Interaction {
            name: "avenger.select".into(),
            version: 1,
            payload: Bytes::new(),
        }];
        unit.encodings[2].conditions = vec![proto::Condition {
            predicate: Some(proto::Predicate {
                predicate: Some(predicate::Predicate::Not(Box::new(select("brush")))),
            }),
            value: Some(condition::Value::Constant(text("gray"))),
        }];
        leaf(unit)
    };
    let mut row = concat(proto::Direction::Row, vec![panel("left"), panel("right")]);
    row.facet = Some(proto::Facet {
        column: "panel".into(),
        header: Some(facet::Header::Shown(proto::FacetHeader {
            side: proto::Side::Bottom as i32,
            ..Default::default()
        })),
    });
    row.partition = vec![partition("dataset", "cars")];
    let selections = vec![selection(
        "brush",
        vec![contribution(
            "plot",
            vec![partition("dataset", "cars"), partition("panel", "left")],
        )],
    )];
    chart(concat_node(row), selections).unwrap();
}

#[test]
fn encodings_follow_the_rules_of_their_channels() {
    unit_fails(
        |unit| unit.encodings.swap(0, 1),
        "root/encodings[1]",
        "in the order of `Channel`",
    );
    unit_fails(
        |unit| unit.encodings[0].scale = None,
        "root/encodings[0]",
        "`x` and `y` have a scale",
    );
    unit_fails(
        |unit| unit.encodings[0] = constant(Channel::X, number(1.0)),
        "root/encodings[0]",
        "a Cartesian position is a column or a scaled constant",
    );
    unit_fails(
        |unit| unit.encodings[2] = raw(Channel::Color, "count"),
        "root/encodings[2]",
        "without a scale cannot hold values of type Int64",
    );
    unit_fails(
        |unit| {
            unit.encodings[2] = proto::Encoding {
                scale: Some(spec("ordinal")),
                ..constant(Channel::Color, text("red"))
            }
        },
        "root/encodings[2]",
        "all constants has no scale",
    );
    unit_fails(
        |unit| {
            unit.encodings[2] = proto::Encoding {
                value: Some(encoding::Value::ScaledConstant(text("a"))),
                ..raw(Channel::Color, "group")
            }
        },
        "root/encodings[2]",
        "goes through the channel's scale",
    );
    unit_fails(
        |unit| unit.encodings[1].value = Some(encoding::Value::Column("missing".into())),
        "root/encodings[1]",
        "no column `missing`",
    );
    unit_fails(
        |unit| unit.encodings.push(raw(Channel::Detail, "group")),
        "root/encodings[3]",
        "a symbol mark in a Cartesian plot does not take `CHANNEL_DETAIL`",
    );
    unit_fails(
        |unit| unit.encodings[0].conditions = vec![proto::Condition::default()],
        "root/encodings[0]",
        "have no conditions",
    );
    unit_fails(
        |unit| {
            unit.encodings[2].conditions = vec![proto::Condition {
                predicate: Some(select("missing")),
                value: Some(condition::Value::Constant(text("gray"))),
            }]
        },
        "root/encodings[2]/conditions[0]",
        "no selection `missing`",
    );
    unit_fails(
        |unit| {
            unit.encodings[2] = proto::Encoding {
                title: Some("Fill".into()),
                ..raw(Channel::Color, "css")
            }
        },
        "root/encodings[2]",
        "an encoding without a scale has no guide, so it has no title",
    );
    unit_fails(
        |unit| unit.encodings[0].selection_field = "missing".into(),
        "root/encodings[0]",
        "no column `missing`",
    );

    // The second channel of a pair: after its first, without a scale.
    let bars = |encodings| unit(mark::Mark::Bar(proto::BarMark::default()), encodings);
    fails(
        leaf(bars(vec![
            raw(Channel::X2, "x2"),
            scaled(Channel::Y, "y", "linear"),
        ])),
        vec![],
        "root/encodings[0]",
        "the second channel of `CHANNEL_X`",
    );
    fails(
        leaf(bars(vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            scaled(Channel::X2, "x2", "linear"),
        ])),
        vec![],
        "root/encodings[2]",
        "`CHANNEL_X2` takes no scale",
    );
}

#[test]
fn marks_draw_in_their_coordinate_systems_with_their_channels() {
    let mut shape = unit(
        mark::Mark::Geoshape(proto::GeoshapeMark::default()),
        vec![raw(Channel::Fill, "css")],
    );
    fails(
        leaf(shape.clone()),
        vec![],
        "root",
        "a Cartesian plot does not draw a geoshape mark",
    );
    shape.coord = map();
    fails(leaf(shape), vec![], "root", "needs `CHANNEL_GEOMETRY`");

    fails(
        leaf(unit(
            mark::Mark::Text(proto::TextMark::default()),
            vec![
                scaled(Channel::X, "x", "linear"),
                scaled(Channel::Y, "y", "linear"),
            ],
        )),
        vec![],
        "root",
        "needs `CHANNEL_TEXT`",
    );

    let mut rule = unit(
        mark::Mark::Rule(proto::RuleMark::default()),
        vec![raw(Channel::Longitude, "lon")],
    );
    rule.coord = map();
    fails(leaf(rule), vec![], "root", "needs `CHANNEL_LATITUDE`");

    // A rect spans a band only of a band scale.
    fails(
        leaf(unit(
            mark::Mark::Rect(proto::RectMark::default()),
            vec![
                scaled(Channel::X, "x", "linear"),
                scaled(Channel::Y, "y", "linear"),
                raw(Channel::X2, "x2"),
            ],
        )),
        vec![],
        "root",
        "a rect with `CHANNEL_Y` and without `CHANNEL_Y2`",
    );

    // A line's stroke width holds one value in each series.
    fails(
        leaf(unit(
            mark::Mark::Line(proto::LineMark::default()),
            vec![
                scaled(Channel::X, "x", "linear"),
                scaled(Channel::Y, "y", "linear"),
                raw(Channel::StrokeWidth, "x"),
            ],
        )),
        vec![],
        "root",
        "`CHANNEL_STROKE_WIDTH` takes one value for each series of the line",
    );
}

#[test]
fn scale_specs_follow_their_scale_types() {
    let scale = |change: fn(&mut proto::ScaleSpec)| {
        move |unit: &mut proto::Unit| change(unit.encodings[0].scale.as_mut().unwrap())
    };
    unit_fails(
        scale(|spec| spec.r#type = "linaer".into()),
        "root/encodings[0]/scale",
        "no scale type `linaer`",
    );
    unit_fails(
        scale(|spec| spec.r#type = "band".into()),
        "root/encodings[0]/scale/fit",
        "not of the kind that a band scale fits",
    );
    unit_fails(
        |unit| unit.encodings[0] = scaled(Channel::X, "group", "linear"),
        "root/encodings[0]/scale",
        "a linear scale does not take values of type Utf8",
    );
    unit_fails(
        scale(|spec| {
            spec.options = vec![proto::NamedOption {
                name: "nonsense".into(),
                value: Some(
                    payload::encode_option(&avenger_scales::scalar::Scalar::from_bool(true))
                        .unwrap(),
                ),
            }]
        }),
        "root/encodings[0]/scale",
        "nonsense",
    );
    unit_fails(
        scale(|spec| spec.fit = None),
        "root/encodings[0]/scale",
        "has a fit",
    );
    unit_fails(
        scale(|spec| spec.domain = explicit(numbers(&[0.0, 10.0, 20.0]))),
        "root/encodings[0]/scale/domain",
        "holds its two ends",
    );
    unit_fails(
        scale(|spec| {
            spec.domain = explicit(numbers(&[10.0, 0.0]));
            spec.fit = None;
        }),
        "root/encodings[0]/scale/domain",
        "ascending order",
    );
    unit_fails(
        scale(|spec| spec.domain = explicit(numbers(&[0.0, 10.0]))),
        "root/encodings[0]/scale",
        "has none",
    );
    unit_fails(
        scale(|spec| spec.domain = None),
        "root/encodings[0]/scale/domain",
        "a scale has a domain",
    );
    unit_fails(
        scale(|spec| {
            spec.range = Some(proto::Range {
                range: Some(range::Range::Values(numbers(&[0.0, 100.0]))),
                reverse: false,
            })
        }),
        "root/encodings[0]/scale/range",
        "sets only `reverse`",
    );
    unit_fails(
        scale(|spec| {
            spec.domain = Some(proto::Domain {
                domain: Some(domain::Domain::Summary(push(Arc::new(
                    DomainSummary::Sample(SampleSummary::new(&[1.0, 2.0]).unwrap()).to_arrow(),
                )))),
            })
        }),
        "root/encodings[0]/scale/domain",
        "the summary is not of the kind that a linear scale fits",
    );
    unit_fails(
        |unit| unit.encodings[2] = scaled(Channel::Color, "x", "threshold"),
        "root/encodings[2]/scale/domain",
        "a threshold scale takes an explicit domain",
    );
    unit_fails(
        |unit| {
            let spec = unit.encodings[2].scale.as_mut().unwrap();
            spec.r#type = "nested_band".into();
            spec.fit = Some(proto::DomainFit {
                fit: Some(domain_fit::Fit::Categories(proto::CategoryFit {
                    order: Some(category_fit::Order::Preferred(texts(&["b"]))),
                })),
            });
        },
        "root/encodings[2]/scale",
        "a nested_band scale does not take values of type Utf8",
    );

    // A selection domain reads a selection of the chart and falls back to another domain.
    let selection_domain = |fallback: Option<proto::Domain>| {
        let mut unit = scatter();
        unit.encodings[0].scale.as_mut().unwrap().domain = Some(proto::Domain {
            domain: Some(domain::Domain::Selection(Box::new(
                proto::SelectionDomain {
                    selection: "brush".into(),
                    channel: Channel::X as i32,
                    fallback: fallback.map(Box::new),
                },
            ))),
        });
        leaf(unit)
    };
    chart(
        selection_domain(data_domain()),
        vec![selection("brush", vec![])],
    )
    .unwrap();
    fails(
        selection_domain(data_domain()),
        vec![],
        "root/encodings[0]/scale/domain",
        "no selection `brush`",
    );
    fails(
        selection_domain(explicit(numbers(&[0.0, 1.0]))),
        vec![selection("brush", vec![])],
        "root/encodings[0]/scale",
        "has none",
    );
}

#[test]
fn guides_suit_their_channels_and_scales() {
    let guide = |channel: usize, guide: guide::Guide| {
        move |unit: &mut proto::Unit| {
            unit.encodings[channel].scale.as_mut().unwrap().guide =
                Some(proto::Guide { guide: Some(guide) })
        }
    };
    unit_fails(
        guide(2, guide::Guide::Axis(proto::Axis::default())),
        "root/encodings[2]/scale/guide",
        "an axis is the guide of",
    );
    unit_fails(
        guide(0, guide::Guide::Legend(proto::Legend::default())),
        "root/encodings[0]/scale/guide",
        "other than a position",
    );
    unit_fails(
        guide(
            2,
            guide::Guide::Legend(proto::Legend {
                tick_count: Some(5.0),
                ..Default::default()
            }),
        ),
        "root/encodings[2]/scale/guide",
        "takes a tick count or values",
    );
    unit_fails(
        guide(2, guide::Guide::Colorbar(proto::Colorbar::default())),
        "root/encodings[2]/scale/guide",
        "a colorbar is the guide of",
    );
    unit_fails(
        guide(
            0,
            guide::Guide::Axis(proto::Axis {
                ticks: Some(axis::Ticks::TickValues(texts(&["a"]))),
                ..Default::default()
            }),
        ),
        "root/encodings[0]/scale/guide",
        "the values have type Utf8",
    );
    unit_fails(
        guide(
            0,
            guide::Guide::Axis(proto::Axis {
                tick_count: Some(4.0),
                ticks: Some(axis::Ticks::TickValues(numbers(&[1.0]))),
                ..Default::default()
            }),
        ),
        "root/encodings[0]/scale/guide",
        "takes no tick count",
    );
    unit_fails(
        guide(
            0,
            guide::Guide::Axis(proto::Axis {
                angle: Some(30.0),
                ..Default::default()
            }),
        ),
        "root/encodings[0]/scale/guide",
        "only an axis of `radius` in a polar plot takes an angle",
    );

    // A polar axis draws no title.
    let mut rose = unit(
        symbol(),
        vec![
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Radius, "width"),
        ],
    );
    rose.coord = polar();
    rose.encodings[0].scale.as_mut().unwrap().guide = Some(proto::Guide {
        guide: Some(guide::Guide::Axis(proto::Axis {
            title: Some("angle".into()),
            ..Default::default()
        })),
    });
    fails(
        leaf(rose),
        vec![],
        "root/encodings[0]/scale/guide",
        "a polar axis has no title",
    );
}

#[test]
fn coordinate_systems_have_valid_options() {
    let with_coord = |coord: proto::Coord| {
        let mut unit = unit(
            symbol(),
            vec![
                raw(Channel::Longitude, "lon"),
                raw(Channel::Latitude, "lat"),
            ],
        );
        unit.coord = Some(coord);
        leaf(unit)
    };
    fails(
        with_coord(proto::Coord {
            coord: Some(coord::Coord::Polar(proto::Polar {
                start_angle: Some(90.0),
                end_angle: Some(45.0),
                inner_radius: None,
            })),
        }),
        vec![],
        "root/coord",
        "more than its start angle",
    );
    fails(
        with_coord(proto::Coord {
            coord: Some(coord::Coord::Polar(proto::Polar {
                inner_radius: Some(1.0),
                ..Default::default()
            })),
        }),
        vec![],
        "root/coord",
        "less than 1",
    );
    fails(
        with_coord(proto::Coord {
            coord: Some(coord::Coord::Geo(proto::Geo {
                surface: Some(geo::Surface::Planar(proto::Planar::default())),
                fit: Some(geo::Fit::FitData(proto::PlotData {})),
                graticule: Some(proto::Graticule::default()),
                ..Default::default()
            })),
        }),
        vec![],
        "root/coord",
        "a planar map has no graticule",
    );
    let left = proto::Side::Left as i32;
    fails(
        with_coord(proto::Coord {
            coord: Some(coord::Coord::Geo(proto::Geo {
                surface: Some(geo::Surface::Spherical(proto::Spherical {
                    projection: "mercator".into(),
                    ..Default::default()
                })),
                fit: Some(geo::Fit::FitSphere(proto::WholeSphere {})),
                graticule: Some(proto::Graticule {
                    label_sides: Some(proto::LabelSides {
                        sides: vec![left, left],
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }),
        vec![],
        "root/coord",
        "each at most once",
    );
}

#[test]
fn nodes_follow_the_rules_of_the_tree() {
    // Identity: an id and a partition name at most one node.
    let named = |id: &str| {
        let mut unit = scatter();
        unit.id = id.into();
        leaf(unit)
    };
    fails(
        overlay(vec![named("a"), named("a")]),
        vec![],
        "root/children[1]",
        "two nodes have the id `a` and one partition",
    );
    unit_fails(
        |unit| {
            unit.interactions = vec![proto::Interaction {
                name: "avenger.navigate".into(),
                ..Default::default()
            }]
        },
        "root",
        "declares interactions has an id",
    );

    // Overlays hold untitled units and overlays in one coordinate system.
    fails(
        overlay(vec![concat_node(concat(
            proto::Direction::Row,
            vec![leaf(scatter())],
        ))]),
        vec![],
        "root/children[0]",
        "an overlay holds units and overlays",
    );
    let mut titled = scatter();
    titled.title = Some(proto::Title {
        text: "title".into(),
        ..Default::default()
    });
    fails(
        overlay(vec![leaf(titled)]),
        vec![],
        "root/children[0]",
        "no titles",
    );
    let mut polar_child = scatter();
    polar_child.coord = polar();
    let mut cartesian_child = scatter();
    cartesian_child.coord = Some(proto::Coord {
        coord: Some(coord::Coord::Cartesian(proto::Cartesian::default())),
    });
    fails(
        overlay(vec![leaf(cartesian_child), leaf(polar_child)]),
        vec![],
        "root/children[1]",
        "the coordinate system of the overlay",
    );

    // Concats and wraps.
    fails(
        concat_node(concat(proto::Direction::Unspecified, vec![leaf(scatter())])),
        vec![],
        "root",
        "a row or a column",
    );
    let mut tracks = concat(proto::Direction::Row, vec![leaf(scatter())]);
    tracks.layout = Some(proto::ConcatLayout {
        tracks: vec![proto::Track::default(), proto::Track::default()],
        ..Default::default()
    });
    fails(
        concat_node(tracks),
        vec![],
        "root/layout",
        "2 tracks for 1 children",
    );
    let mut inner = concat(proto::Direction::Column, vec![leaf(scatter())]);
    inner.layout = Some(proto::ConcatLayout {
        horizontal_distribute: proto::Distribute::Center as i32,
        ..Default::default()
    });
    fails(
        concat_node(concat(proto::Direction::Row, vec![concat_node(inner)])),
        vec![],
        "root/children[0]/layout",
        "only the outermost concat",
    );
    let mut rules = concat(proto::Direction::Row, vec![leaf(scatter())]);
    rules.layout = Some(proto::ConcatLayout {
        guide_rules: vec![
            proto::GuideRule {
                part: proto::GuidePart::AxisTitle as i32,
                rule: proto::GuideRuleKind::Keep as i32,
            },
            proto::GuideRule {
                part: proto::GuidePart::AxisTicks as i32,
                rule: proto::GuideRuleKind::Keep as i32,
            },
        ],
        ..Default::default()
    });
    fails(
        concat_node(rules),
        vec![],
        "root/layout/guide_rules[1]",
        "in the order of `GuidePart`",
    );
    fails(
        proto::Node {
            node: Some(node::Node::Empty(proto::Empty::default())),
        },
        vec![],
        "root",
        "an empty child is a child of a concat or a wrap",
    );
}

#[test]
fn partitions_and_facets_follow_their_rules() {
    unit_fails(
        |unit| unit.partition = vec![partition("b", "1"), partition("a", "1")],
        "root/partition[1]",
        "in the order of their columns",
    );
    unit_fails(
        |unit| unit.partition = vec![partition("group", "a")],
        "root",
        "holds other values than the partition value",
    );
    let mut child = scatter();
    child.partition = vec![partition("panel", "left")];
    let mut parent = concat(proto::Direction::Row, vec![leaf(child)]);
    parent.partition = vec![partition("panel", "all")];
    fails(
        concat_node(parent),
        vec![],
        "root/children[0]/partition[0]",
        "an ancestor declares the partition column `panel`",
    );

    let panel = |value: u32| {
        let mut unit = scatter();
        unit.partition = vec![proto::PartitionValue {
            column: "panel".into(),
            value,
        }];
        leaf(unit)
    };
    let faceted = |children, header| {
        let mut row = concat(proto::Direction::Row, children);
        row.facet = Some(proto::Facet {
            column: "panel".into(),
            header,
        });
        concat_node(row)
    };
    fails(
        faceted(vec![panel(text("left")), leaf(scatter())], None),
        vec![],
        "root/facet",
        "child 1 declares no value of the facet's column `panel`",
    );
    fails(
        faceted(vec![panel(text("left")), panel(number(2.0))], None),
        vec![],
        "root/facet",
        "have one type",
    );
    fails(
        faceted(
            vec![panel(text("left"))],
            Some(facet::Header::Shown(proto::FacetHeader {
                side: proto::Side::Left as i32,
                ..Default::default()
            })),
        ),
        vec![],
        "root/facet/shown",
        "the header of a row stands on the top or the bottom",
    );
}

#[test]
fn selections_follow_their_rules() {
    let owned = || {
        let mut unit = scatter();
        unit.id = "plot".into();
        leaf(unit)
    };
    fails(
        leaf(scatter()),
        vec![selection("b", vec![]), selection("a", vec![])],
        "selections[1]",
        "in the order of their names",
    );
    fails(
        leaf(scatter()),
        vec![proto::Selection {
            name: "brush".into(),
            ..Default::default()
        }],
        "selections[0]",
        "a selection has a resolution",
    );
    fails(
        owned(),
        vec![selection("brush", vec![contribution("other", vec![])])],
        "selections[0]/initial[0]",
        "no unit or overlay with the id `other`",
    );
    fails(
        owned(),
        vec![selection(
            "brush",
            vec![contribution("plot", vec![partition("panel", "left")])],
        )],
        "selections[0]/initial[0]",
        "no unit or overlay with the id `plot`",
    );
    fails(
        owned(),
        vec![selection(
            "brush",
            vec![contribution("plot", vec![]), contribution("plot", vec![])],
        )],
        "selections[0]/initial[1]",
        "each owner at most once",
    );
    let mut descending = contribution("plot", vec![]);
    descending.tuples[0].fields[0].test = Some(tuple_field::Test::Range(proto::ValueRange {
        column: "x".into(),
        channel: Channel::X as i32,
        ends: numbers(&[3.0, 1.0]),
        exclude_end: false,
    }));
    fails(
        owned(),
        vec![selection("brush", vec![descending])],
        "selections[0]/initial[0]/tuples[0]/fields[0]",
        "ascending order",
    );
    let mut unwrapped = contribution("plot", vec![]);
    unwrapped.tuples[0].fields[0].test = Some(tuple_field::Test::CyclicRange(proto::CyclicRange {
        column: "x".into(),
        channel: Channel::Longitude as i32,
        start: 170.0,
        end: -170.0,
        period: 0.0,
    }));
    fails(
        owned(),
        vec![selection("brush", vec![unwrapped])],
        "selections[0]/initial[0]/tuples[0]/fields[0]",
        "a positive, finite period",
    );
}

#[test]
fn fields_refer_to_values_of_their_kind() {
    unit_fails(
        |unit| unit.encodings[2] = constant(Channel::Color, 7),
        "root/encodings[2]",
        "the field refers to value 7, and the chart has 0 values",
    );
    unit_fails(
        |unit| unit.encodings[0].scale.as_mut().unwrap().domain = explicit(number(5.0)),
        "root/encodings[0]/scale/domain",
        "the field holds a list",
    );
    unit_fails(
        |unit| {
            let pair = push(Arc::new(Float64Array::from(vec![0.5, 0.6])));
            unit.encodings.push(constant(Channel::Opacity, pair));
        },
        "values[0]",
        "a value is one element, not 2",
    );
}
