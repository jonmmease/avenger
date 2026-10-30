//! The rules of the format: charts that keep them pass, and a chart that breaks one fails with
//! the path of the part that breaks it.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use arrow::array::{ArrayRef, BinaryArray, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart::{
    payload,
    proto::{
        self, axis, category_fit, condition, conditional_value, coord, domain, domain_fit,
        encoding, facet, geo, guide, mark, node, predicate, range, Channel,
    },
    Chart, ChartError, FORMAT_VERSION,
};
use avenger_scales::scales::domain_summary::{DomainSummary, SampleSummary};
use bytes::Bytes;

thread_local! {
    /// The values that the messages of a test refer to, which `chart` takes.
    static VALUES: RefCell<Vec<ArrayRef>> = const { RefCell::new(Vec::new()) };
}

/// A reference to `value`, added to the values that the next chart takes.
fn push(value: ArrayRef) -> proto::ValueRef {
    VALUES.with_borrow_mut(|values| {
        values.push(value);
        proto::ValueRef {
            index: (values.len() - 1) as u32,
        }
    })
}

fn number(value: f64) -> proto::ValueRef {
    push(Arc::new(Float64Array::from(vec![value])))
}

fn text(value: &str) -> proto::ValueRef {
    push(Arc::new(StringArray::from(vec![value])))
}

fn numbers(values: &[f64]) -> proto::ValueRef {
    push(payload::list(Arc::new(Float64Array::from(values.to_vec()))))
}

fn texts(values: &[&str]) -> proto::ValueRef {
    push(payload::list(Arc::new(StringArray::from(values.to_vec()))))
}

fn data_domain() -> Option<proto::Domain> {
    Some(proto::Domain {
        domain: Some(domain::Domain::Data(proto::DataDomain {})),
    })
}

fn explicit(value: proto::ValueRef) -> Option<proto::Domain> {
    Some(proto::Domain {
        domain: Some(domain::Domain::Explicit(value)),
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

/// A binding of a channel.
type Bound = (Channel, proto::Encoding);

/// `channel` from the column `name` without a scale.
fn raw(channel: Channel, name: &str) -> Bound {
    let encoding = proto::Encoding {
        value: Some(encoding::Value::Column(name.into())),
        ..Default::default()
    };
    (channel, encoding)
}

/// `channel` from the column `name` through a scale of `scale_type`.
fn scaled(channel: Channel, name: &str, scale_type: &str) -> Bound {
    let (channel, encoding) = raw(channel, name);
    let encoding = proto::Encoding {
        scale: Some(spec(scale_type)),
        ..encoding
    };
    (channel, encoding)
}

fn constant(channel: Channel, value: proto::ValueRef) -> Bound {
    let encoding = proto::Encoding {
        value: Some(encoding::Value::Constant(value)),
        ..Default::default()
    };
    (channel, encoding)
}

fn unit(mark: mark::Mark, encodings: Vec<Bound>) -> proto::Unit {
    let mut unit = proto::Unit {
        mark: Some(proto::Mark { mark: Some(mark) }),
        table: Some(proto::TableRef { index: 0 }),
        ..Default::default()
    };
    for bound in encodings {
        set(&mut unit, bound);
    }
    unit
}

/// The field of `channel` in the mark of `unit`, which takes the channel.
fn field(unit: &mut proto::Unit, channel: Channel) -> &mut Option<proto::Encoding> {
    unit.mark
        .as_mut()
        .and_then(|mark| mark.mark.as_mut())
        .and_then(|mark| mark.encoding_mut(channel))
        .expect("the mark takes the channel")
}

/// Binds a channel of the mark of `unit`, in place of its encoding.
fn set(unit: &mut proto::Unit, (channel, encoding): Bound) {
    *field(unit, channel) = Some(encoding);
}

/// The encoding of `channel` in the mark of `unit`.
fn enc(unit: &mut proto::Unit, channel: Channel) -> &mut proto::Encoding {
    field(unit, channel)
        .as_mut()
        .expect("the mark encodes the channel")
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
        value: Some(text(value)),
    }
}

fn select(name: &str) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(predicate::Predicate::Selected(proto::Selected {
            selection: name.into(),
            ..Default::default()
        })),
    }
}

fn selection(name: &str, initial: Vec<proto::Clause>) -> proto::Selection {
    proto::Selection {
        name: name.into(),
        resolution: proto::SelectionResolution::Union as i32,
        initial,
    }
}

/// A clause of `owner` that selects the group `a`.
fn clause(owner: &str, partition: Vec<proto::PartitionValue>) -> proto::Clause {
    proto::Clause {
        owner: owner.into(),
        conditions: vec![proto::Condition {
            kind: Some(condition::Kind::Set(proto::ValueSet {
                columns: vec!["group".into()],
                channels: vec![Channel::Color as i32],
                values: vec![texts(&["a"])],
            })),
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
    let table = RecordBatch::try_from_iter(columns).unwrap();
    // The places as WKB points, in a field of the GeoArrow extension type `geoarrow.wkb`.
    let points: Vec<Vec<u8>> = [
        (-70.0_f64, 40.0_f64),
        (-60.0, 45.0),
        (0.0, 50.0),
        (10.0, 55.0),
    ]
    .iter()
    .map(|(lon, lat)| {
        let mut point = vec![1, 1, 0, 0, 0];
        point.extend(lon.to_le_bytes());
        point.extend(lat.to_le_bytes());
        point
    })
    .collect();
    let geometry: ArrayRef = Arc::new(BinaryArray::from_iter_values(points));
    let field = Field::new("geometry", DataType::Binary, false).with_metadata(HashMap::from([(
        "ARROW:extension:name".to_string(),
        "geoarrow.wkb".to_string(),
    )]));
    let mut fields: Vec<Field> = table
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    fields.push(field);
    let mut columns = table.columns().to_vec();
    columns.push(geometry);
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
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
        mark::Mark::PolarRect(proto::PolarRectMark::default()),
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
        mark::Mark::GeoSymbol(proto::GeoSymbolMark::default()),
        vec![
            raw(Channel::Longitude, "lon"),
            raw(Channel::Latitude, "lat"),
        ],
    );
    places.coord = map();
    let mut route = unit(
        mark::Mark::GeoRule(proto::GeoRuleMark::default()),
        vec![
            raw(Channel::Longitude, "lon"),
            raw(Channel::Latitude, "lat"),
        ],
    );
    route.coord = map();
    chart(overlay(vec![leaf(places), leaf(route)]), vec![]).unwrap();

    // An overlay takes the options of the coordinate system that its children set, and a polar
    // plot without options takes the defaults.
    let mut ring = unit(
        mark::Mark::PolarSymbol(proto::PolarSymbolMark::default()),
        vec![
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Radius, "width"),
        ],
    );
    let spokes = ring.clone();
    ring.coord = polar();
    chart(overlay(vec![leaf(ring), leaf(spokes.clone())]), vec![]).unwrap();
    chart(leaf(spokes), vec![]).unwrap();
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
        enc(&mut unit, Channel::Color).conditional = vec![proto::ConditionalValue {
            predicate: Some(proto::Predicate {
                predicate: Some(predicate::Predicate::Not(Box::new(select("brush")))),
            }),
            value: Some(conditional_value::Value::Constant(text("gray"))),
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
        vec![clause(
            "plot",
            vec![partition("dataset", "cars"), partition("panel", "left")],
        )],
    )];
    chart(concat_node(row), selections).unwrap();
}

#[test]
fn encodings_follow_the_rules_of_their_channels() {
    unit_fails(
        |unit| enc(unit, Channel::X).scale = None,
        "root/mark/x",
        "`x` and `y` have a scale",
    );
    unit_fails(
        |unit| set(unit, constant(Channel::X, number(1.0))),
        "root/mark/x",
        "a Cartesian position is a column or a scaled constant",
    );
    unit_fails(
        |unit| set(unit, raw(Channel::Color, "count")),
        "root/mark/color",
        "without a scale cannot hold values of type Int64",
    );
    unit_fails(
        |unit| {
            set(unit, constant(Channel::Color, text("red")));
            enc(unit, Channel::Color).scale = Some(spec("ordinal"));
        },
        "root/mark/color",
        "all constants has no scale",
    );
    unit_fails(
        |unit| {
            enc(unit, Channel::Color).scale = None;
            enc(unit, Channel::Color).value = Some(encoding::Value::ScaledConstant(text("a")));
        },
        "root/mark/color",
        "goes through the channel's scale",
    );
    unit_fails(
        |unit| enc(unit, Channel::Y).value = Some(encoding::Value::Column("missing".into())),
        "root/mark/y",
        "no column `missing`",
    );
    unit_fails(
        |unit| enc(unit, Channel::X).conditional = vec![proto::ConditionalValue::default()],
        "root/mark/x",
        "have no conditional values",
    );
    unit_fails(
        |unit| {
            enc(unit, Channel::Color).conditional = vec![proto::ConditionalValue {
                predicate: Some(select("missing")),
                value: Some(conditional_value::Value::Constant(text("gray"))),
            }]
        },
        "root/mark/color/conditional[0]",
        "no selection `missing`",
    );
    unit_fails(
        |unit| {
            set(unit, raw(Channel::Color, "css"));
            enc(unit, Channel::Color).title = Some("Fill".into());
        },
        "root/mark/color",
        "an encoding without a scale has no guide, so it has no title",
    );
    unit_fails(
        |unit| enc(unit, Channel::X).selection_field = "missing".into(),
        "root/mark/x",
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
        "root/mark/x2",
        "the second channel of `CHANNEL_X`",
    );
    fails(
        leaf(bars(vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            scaled(Channel::X2, "x2", "linear"),
        ])),
        vec![],
        "root/mark/x2",
        "`CHANNEL_X2` takes no scale",
    );
}

#[test]
fn marks_draw_in_their_coordinate_systems_with_their_channels() {
    // The mark gives the kind of the plot, which the options of its coordinate system match.
    let mut shape = unit(
        mark::Mark::GeoShape(proto::GeoShapeMark::default()),
        vec![raw(Channel::Fill, "css")],
    );
    fails(
        leaf(shape.clone()),
        vec![],
        "root",
        "a map plot sets the options of its projection",
    );
    shape.coord = polar();
    fails(
        leaf(shape.clone()),
        vec![],
        "root/coord",
        "of the kind of its marks, a map plot",
    );
    shape.coord = map();
    fails(
        leaf(shape.clone()),
        vec![],
        "root",
        "a `geo_shape` mark needs `CHANNEL_GEOMETRY`",
    );
    // A geo shape draws a column of GeoArrow geometries.
    shape.mark = unit(
        mark::Mark::GeoShape(proto::GeoShapeMark::default()),
        vec![raw(Channel::Geometry, "geometry")],
    )
    .mark;
    chart(leaf(shape.clone()), vec![]).unwrap();
    set(&mut shape, raw(Channel::Geometry, "css"));
    fails(
        leaf(shape),
        vec![],
        "root/mark/geometry",
        "a `geometry` encoding is a column of GeoArrow geometries",
    );

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
        mark::Mark::GeoRule(proto::GeoRuleMark::default()),
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
        "`CHANNEL_STROKE_WIDTH` takes one value for each series of a `line` mark",
    );
}

/// A map plot of the places of the table that fits `fit`, in the share family `family`.
fn places(fit: geo::Fit, family: &str) -> proto::Node {
    let mut places = unit(
        mark::Mark::GeoSymbol(proto::GeoSymbolMark::default()),
        vec![
            raw(Channel::Longitude, "lon"),
            raw(Channel::Latitude, "lat"),
        ],
    );
    places.coord = Some(proto::Coord {
        coord: Some(coord::Coord::Geo(proto::Geo {
            surface: Some(geo::Surface::Spherical(proto::Spherical {
                projection: "equal_earth".into(),
                ..Default::default()
            })),
            fit: Some(fit),
            family: family.into(),
            ..Default::default()
        })),
    });
    leaf(places)
}

#[test]
fn the_map_plots_of_a_shared_projection_agree() {
    let data = || geo::Fit::FitData(proto::PlotData {});
    let sphere = || geo::Fit::FitSphere(proto::WholeSphere {});
    // A row keeps a projection for each child, so its maps can fit different things.
    let mut row = concat(
        proto::Direction::Row,
        vec![places(data(), ""), places(sphere(), "")],
    );
    chart(concat_node(row.clone()), vec![]).unwrap();
    // Maps that share a projection fit one view.
    row.projection = proto::Sharing::Shared as i32;
    fails(
        concat_node(row),
        vec![],
        "root/children[1]",
        "the map plots `root/children[0]` and `root/children[1]` share a projection, so they fit \
         one view, and their fits differ",
    );
    // A share family joins the projections of maps that their nodes keep apart.
    fails(
        concat_node(concat(
            proto::Direction::Row,
            vec![places(data(), "world"), places(sphere(), "world")],
        )),
        vec![],
        "root/children[1]",
        "and their fits differ",
    );
    chart(
        concat_node(concat(
            proto::Direction::Row,
            vec![places(data(), "world"), places(data(), "world")],
        )),
        vec![],
    )
    .unwrap();
}

#[test]
fn scale_specs_follow_their_scale_types() {
    let scale = |change: fn(&mut proto::ScaleSpec)| {
        move |unit: &mut proto::Unit| change(enc(unit, Channel::X).scale.as_mut().unwrap())
    };
    unit_fails(
        scale(|spec| spec.r#type = "linaer".into()),
        "root/mark/x/scale",
        "no scale type `linaer`",
    );
    unit_fails(
        scale(|spec| spec.r#type = "band".into()),
        "root/mark/x/scale/fit",
        "not of the kind that a band scale fits",
    );
    unit_fails(
        |unit| set(unit, scaled(Channel::X, "group", "linear")),
        "root/mark/x/scale",
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
        "root/mark/x/scale",
        "nonsense",
    );
    unit_fails(
        scale(|spec| spec.fit = None),
        "root/mark/x/scale",
        "has a fit",
    );
    unit_fails(
        scale(|spec| spec.domain = explicit(numbers(&[0.0, 10.0, 20.0]))),
        "root/mark/x/scale/domain",
        "holds its two ends",
    );
    unit_fails(
        scale(|spec| {
            spec.domain = explicit(numbers(&[10.0, 0.0]));
            spec.fit = None;
        }),
        "root/mark/x/scale/domain",
        "ascending order",
    );
    unit_fails(
        scale(|spec| spec.domain = explicit(numbers(&[0.0, 10.0]))),
        "root/mark/x/scale",
        "has none",
    );
    unit_fails(
        scale(|spec| spec.domain = None),
        "root/mark/x/scale/domain",
        "a scale has a domain",
    );
    unit_fails(
        scale(|spec| {
            spec.range = Some(proto::Range {
                range: Some(range::Range::Values(numbers(&[0.0, 100.0]))),
                reverse: false,
            })
        }),
        "root/mark/x/scale/range",
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
        "root/mark/x/scale/domain",
        "the summary is not of the kind that a linear scale fits",
    );
    unit_fails(
        |unit| set(unit, scaled(Channel::Color, "x", "threshold")),
        "root/mark/color/scale/domain",
        "a threshold scale takes an explicit domain",
    );
    unit_fails(
        |unit| {
            let spec = enc(unit, Channel::Color).scale.as_mut().unwrap();
            spec.r#type = "nested_band".into();
            spec.fit = Some(proto::DomainFit {
                fit: Some(domain_fit::Fit::Categories(proto::CategoryFit {
                    order: Some(category_fit::Order::Preferred(texts(&["b"]))),
                })),
            });
        },
        "root/mark/color/scale",
        "a nested_band scale does not take values of type Utf8",
    );

    // A selection domain reads a selection of the chart and falls back to another domain.
    let selection_domain = |fallback: Option<proto::Domain>| {
        let mut unit = scatter();
        enc(&mut unit, Channel::X).scale.as_mut().unwrap().domain = Some(proto::Domain {
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
        "root/mark/x/scale/domain",
        "no selection `brush`",
    );
    fails(
        selection_domain(explicit(numbers(&[0.0, 1.0]))),
        vec![selection("brush", vec![])],
        "root/mark/x/scale",
        "has none",
    );
}

#[test]
fn guides_suit_their_channels_and_scales() {
    let guide = |channel: Channel, guide: guide::Guide| {
        move |unit: &mut proto::Unit| {
            enc(unit, channel).scale.as_mut().unwrap().guide =
                Some(proto::Guide { guide: Some(guide) })
        }
    };
    unit_fails(
        guide(Channel::Color, guide::Guide::Axis(proto::Axis::default())),
        "root/mark/color/scale/guide",
        "an axis is the guide of",
    );
    unit_fails(
        guide(Channel::X, guide::Guide::Legend(proto::Legend::default())),
        "root/mark/x/scale/guide",
        "other than a position",
    );
    unit_fails(
        guide(
            Channel::Color,
            guide::Guide::Legend(proto::Legend {
                tick_count: Some(5.0),
                ..Default::default()
            }),
        ),
        "root/mark/color/scale/guide",
        "takes a tick count or values",
    );
    unit_fails(
        guide(
            Channel::Color,
            guide::Guide::Colorbar(proto::Colorbar::default()),
        ),
        "root/mark/color/scale/guide",
        "a colorbar is the guide of",
    );
    unit_fails(
        guide(
            Channel::X,
            guide::Guide::Axis(proto::Axis {
                ticks: Some(axis::Ticks::TickValues(texts(&["a"]))),
                ..Default::default()
            }),
        ),
        "root/mark/x/scale/guide",
        "the values have type Utf8",
    );
    unit_fails(
        guide(
            Channel::X,
            guide::Guide::Axis(proto::Axis {
                tick_count: Some(4.0),
                ticks: Some(axis::Ticks::TickValues(numbers(&[1.0]))),
                ..Default::default()
            }),
        ),
        "root/mark/x/scale/guide",
        "takes no tick count",
    );
    unit_fails(
        guide(
            Channel::X,
            guide::Guide::PolarAxis(proto::PolarAxis::default()),
        ),
        "root/mark/x/scale/guide",
        "a polar axis is the guide of `theta` or `radius`",
    );

    // The axes of a polar plot are polar axes, and only the axis of `radius` takes an angle.
    let rose = |guide: guide::Guide| {
        let mut rose = unit(
            mark::Mark::PolarSymbol(proto::PolarSymbolMark::default()),
            vec![
                scaled(Channel::Theta, "x", "linear"),
                raw(Channel::Radius, "width"),
            ],
        );
        enc(&mut rose, Channel::Theta).scale.as_mut().unwrap().guide =
            Some(proto::Guide { guide: Some(guide) });
        leaf(rose)
    };
    chart(
        rose(guide::Guide::PolarAxis(proto::PolarAxis::default())),
        vec![],
    )
    .unwrap();
    fails(
        rose(guide::Guide::Axis(proto::Axis::default())),
        vec![],
        "root/mark/theta/scale/guide",
        "an axis is the guide of `x` or `y`",
    );
    fails(
        rose(guide::Guide::PolarAxis(proto::PolarAxis {
            angle: Some(30.0),
            ..Default::default()
        })),
        vec![],
        "root/mark/theta/scale/guide",
        "only a polar axis of `radius` takes an angle",
    );
}

#[test]
fn coordinate_systems_have_valid_options() {
    let with_coord = |coord: proto::Coord| {
        let mut unit = match coord.coord {
            Some(coord::Coord::Polar(_)) => unit(
                mark::Mark::PolarSymbol(proto::PolarSymbolMark::default()),
                vec![
                    scaled(Channel::Theta, "x", "linear"),
                    raw(Channel::Radius, "width"),
                ],
            ),
            _ => unit(
                mark::Mark::GeoSymbol(proto::GeoSymbolMark::default()),
                vec![
                    raw(Channel::Longitude, "lon"),
                    raw(Channel::Latitude, "lat"),
                ],
            ),
        };
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
    // A map without a fit fits its data.
    chart(
        with_coord(proto::Coord {
            coord: Some(coord::Coord::Geo(proto::Geo {
                surface: Some(geo::Surface::Planar(proto::Planar::default())),
                ..Default::default()
            })),
        }),
        vec![],
    )
    .unwrap();
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
    let polar_child = unit(
        mark::Mark::PolarSymbol(proto::PolarSymbolMark::default()),
        vec![
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Radius, "width"),
        ],
    );
    fails(
        overlay(vec![leaf(scatter()), leaf(polar_child)]),
        vec![],
        "root/children[1]",
        "the marks inside an overlay draw in one coordinate system, and the overlay is a \
         Cartesian plot",
    );
    let aspect = |ratio: f32| {
        let mut unit = scatter();
        unit.coord = Some(proto::Coord {
            coord: Some(coord::Coord::Cartesian(proto::Cartesian {
                aspect_ratio: Some(ratio),
            })),
        });
        leaf(unit)
    };
    fails(
        overlay(vec![leaf(scatter()), aspect(2.0), aspect(3.0)]),
        vec![],
        "root/children[2]",
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

    let panel = |value: proto::ValueRef| {
        let mut unit = scatter();
        unit.partition = vec![proto::PartitionValue {
            column: "panel".into(),
            value: Some(value),
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
        vec![selection("brush", vec![clause("other", vec![])])],
        "selections[0]/initial[0]",
        "no unit or overlay with the id `other`",
    );
    fails(
        owned(),
        vec![selection(
            "brush",
            vec![clause("plot", vec![partition("panel", "left")])],
        )],
        "selections[0]/initial[0]",
        "no unit or overlay with the id `plot`",
    );
    fails(
        owned(),
        vec![selection(
            "brush",
            vec![clause("plot", vec![]), clause("plot", vec![])],
        )],
        "selections[0]/initial[1]",
        "each owner at most once",
    );
    let mut descending = clause("plot", vec![]);
    descending.conditions[0].kind = Some(condition::Kind::Range(proto::ValueRange {
        column: "x".into(),
        channel: Channel::X as i32,
        ends: Some(numbers(&[3.0, 1.0])),
        exclude_end: false,
    }));
    fails(
        owned(),
        vec![selection("brush", vec![descending])],
        "selections[0]/initial[0]/conditions[0]",
        "ascending order",
    );
    let mut unwrapped = clause("plot", vec![]);
    unwrapped.conditions[0].kind = Some(condition::Kind::CyclicRange(proto::CyclicRange {
        column: "x".into(),
        channel: Channel::Longitude as i32,
        start: 170.0,
        end: -170.0,
        period: 0.0,
    }));
    fails(
        owned(),
        vec![selection("brush", vec![unwrapped])],
        "selections[0]/initial[0]/conditions[0]",
        "a positive, finite period",
    );
    let mut uneven = clause("plot", vec![]);
    uneven.conditions[0].kind = Some(condition::Kind::Set(proto::ValueSet {
        columns: vec!["group".into(), "x".into()],
        channels: vec![Channel::Color as i32, Channel::X as i32],
        values: vec![texts(&["a", "b"]), numbers(&[1.0])],
    }));
    fails(
        owned(),
        vec![selection("brush", vec![uneven])],
        "selections[0]/initial[0]/conditions[0]",
        "one length",
    );
    let mut unlabeled = clause("plot", vec![]);
    unlabeled.conditions[0].kind = Some(condition::Kind::Set(proto::ValueSet {
        columns: vec!["group".into(), "x".into()],
        channels: vec![Channel::Color as i32],
        values: vec![texts(&["a"]), numbers(&[1.0])],
    }));
    fails(
        owned(),
        vec![selection("brush", vec![unlabeled])],
        "selections[0]/initial[0]/conditions[0]",
        "a channel and a list of values for each of its columns",
    );
    let mut no_polygon = clause("plot", vec![]);
    no_polygon.conditions[0].kind = Some(condition::Kind::Polygon(proto::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        vertices: vec![],
    }));
    fails(
        owned(),
        vec![selection("brush", vec![no_polygon])],
        "selections[0]/initial[0]/conditions[0]",
        "a polygon or more",
    );
}

#[test]
fn fields_refer_to_values_of_their_kind() {
    unit_fails(
        |unit| set(unit, constant(Channel::Color, proto::ValueRef { index: 7 })),
        "root/mark/color",
        "the field refers to value 7, and the chart has 0 values",
    );
    unit_fails(
        |unit| {
            unit.partition = vec![proto::PartitionValue {
                column: "panel".into(),
                value: None,
            }]
        },
        "root/partition[0]",
        "the field refers to a value",
    );
    unit_fails(|unit| unit.table = None, "root", "a unit refers to a table");
    unit_fails(
        |unit| enc(unit, Channel::X).scale.as_mut().unwrap().domain = explicit(number(5.0)),
        "root/mark/x/scale/domain",
        "the field holds a list",
    );
    unit_fails(
        |unit| {
            let pair = push(Arc::new(Float64Array::from(vec![0.5, 0.6])));
            set(unit, constant(Channel::Opacity, pair));
        },
        "values[0]",
        "a value is one element, not 2",
    );
}

#[test]
fn fonts_and_image_sizes_take_their_values_as_they_are() {
    let labels = |encodings: Vec<Bound>| {
        let mut all = vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            raw(Channel::Text, "group"),
        ];
        all.extend(encodings);
        leaf(unit(mark::Mark::Text(proto::TextMark::default()), all))
    };
    // A weight from numbers or names, a style from names, and a weight that a selection sets.
    let mut bold_when_selected = raw(Channel::FontWeight, "width");
    bold_when_selected.1.conditional = vec![proto::ConditionalValue {
        predicate: Some(select("brush")),
        value: Some(conditional_value::Value::Constant(text("bold"))),
    }];
    chart(
        labels(vec![bold_when_selected, raw(Channel::FontStyle, "css")]),
        vec![selection("brush", vec![])],
    )
    .unwrap();
    chart(labels(vec![raw(Channel::FontWeight, "group")]), vec![]).unwrap();
    fails(
        labels(vec![raw(Channel::FontStyle, "count")]),
        vec![],
        "root/mark/font_style",
        "a `CHANNEL_FONT_STYLE` column without a scale cannot hold values of type Int64",
    );
    fails(
        labels(vec![scaled(Channel::FontWeight, "width", "linear")]),
        vec![],
        "root/mark/font_weight",
        "`CHANNEL_FONT_WEIGHT` takes no scale",
    );

    let image = |encodings: Vec<Bound>| {
        let mut all = vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            raw(Channel::Image, "css"),
        ];
        all.extend(encodings);
        leaf(unit(mark::Mark::Image(proto::ImageMark::default()), all))
    };
    chart(
        image(vec![
            raw(Channel::Width, "width"),
            raw(Channel::Height, "count"),
        ]),
        vec![],
    )
    .unwrap();
    fails(
        image(vec![raw(Channel::Height, "group")]),
        vec![],
        "root/mark/height",
        "cannot hold values of type Utf8",
    );
}

#[test]
fn texts_wrap_at_their_width() {
    let wrapped = |text: proto::TextMark, width: bool| {
        let mut encodings = vec![
            scaled(Channel::X, "x", "linear"),
            scaled(Channel::Y, "y", "linear"),
            raw(Channel::Text, "group"),
        ];
        if width {
            encodings.push(raw(Channel::Width, "width"));
        }
        leaf(unit(mark::Mark::Text(text), encodings))
    };
    chart(
        wrapped(
            proto::TextMark {
                width_mode: proto::TextWidthMode::Fixed as i32,
                max_lines: Some(2),
                ellipsis: Some(true),
                ..Default::default()
            },
            true,
        ),
        vec![],
    )
    .unwrap();
    // A text without a width still keeps its first lines.
    chart(
        wrapped(
            proto::TextMark {
                max_lines: Some(1),
                ellipsis: Some(true),
                ..Default::default()
            },
            false,
        ),
        vec![],
    )
    .unwrap();
    fails(
        wrapped(
            proto::TextMark {
                max_lines: Some(0),
                ..Default::default()
            },
            true,
        ),
        vec![],
        "root/mark",
        "a text keeps one line or more",
    );
    fails(
        wrapped(
            proto::TextMark {
                wrap: Some(false),
                ..Default::default()
            },
            false,
        ),
        vec![],
        "root/mark",
        "apply to its `width`, which it does not encode",
    );
    fails(
        wrapped(
            proto::TextMark {
                ellipsis: Some(true),
                ..Default::default()
            },
            false,
        ),
        vec![],
        "root/mark",
        "it has neither",
    );
}

/// Bars of a rose chart, along `theta` through a scale of `theta_type` out to their `radius`,
/// with `orient`, in the part of a circle of `polar`.
fn rose(theta_type: &str, orient: proto::Orient, polar: proto::Polar) -> proto::Unit {
    let mut rose = unit(
        mark::Mark::PolarBar(proto::PolarBarMark {
            orient: orient as i32,
            ..Default::default()
        }),
        vec![
            scaled(Channel::Theta, "group", theta_type),
            scaled(Channel::Radius, "y", "linear"),
        ],
    );
    if theta_type == "linear" {
        enc(&mut rose, Channel::Theta).value = Some(encoding::Value::Column("x".into()));
    }
    rose.coord = Some(proto::Coord {
        coord: Some(coord::Coord::Polar(polar)),
    });
    rose
}

#[test]
fn polar_plots_keep_their_rules() {
    let vertical = proto::Orient::Vertical;
    chart(leaf(rose("band", vertical, Default::default())), vec![]).unwrap();
    // A bar has no thickness in pixels along an angle or a radius, so a bar without the second
    // channel of its key spans a band.
    fails(
        leaf(rose("linear", vertical, Default::default())),
        vec![],
        "root",
        "a bar in a polar plot with `CHANNEL_THETA` and without `CHANNEL_THETA2` spans a band",
    );
    // A horizontal bar runs along a band of `radius`, out to its `theta`.
    let mut radial = rose("linear", proto::Orient::Horizontal, Default::default());
    set(&mut radial, scaled(Channel::Radius, "group", "band"));
    chart(leaf(radial), vec![]).unwrap();

    // The plots of a shared scale draw one part of a circle.
    let half = proto::Polar {
        start_angle: Some(-90.0),
        end_angle: Some(90.0),
        ..Default::default()
    };
    let mut shared = concat(
        proto::Direction::Row,
        vec![
            leaf(rose("band", vertical, Default::default())),
            leaf(rose("band", vertical, half)),
        ],
    );
    shared.resolve = vec![proto::Resolve {
        channel: Channel::Radius as i32,
        sharing: proto::Sharing::Shared as i32,
    }];
    fails(
        concat_node(shared),
        vec![],
        "root/children[1]",
        "share `CHANNEL_RADIUS`, so they draw one part of a circle",
    );

    // A polar plot has one scale of each position.
    let mut independent = proto::Overlay {
        children: vec![
            leaf(rose("band", vertical, Default::default())),
            leaf(rose("band", vertical, Default::default())),
        ],
        ..Default::default()
    };
    independent.resolve = vec![proto::Resolve {
        channel: Channel::Radius as i32,
        sharing: proto::Sharing::Independent as i32,
    }];
    fails(
        proto::Node {
            node: Some(node::Node::Overlay(independent)),
        },
        vec![],
        "root",
        "a polar plot has one scale of `CHANNEL_RADIUS`, and it has 2",
    );
    let mut independent = proto::Overlay {
        children: vec![
            leaf(rose("band", vertical, Default::default())),
            leaf(rose("band", vertical, Default::default())),
        ],
        ..Default::default()
    };
    independent.resolve = vec![proto::Resolve {
        channel: Channel::Theta as i32,
        sharing: proto::Sharing::Independent as i32,
    }];
    fails(
        proto::Node {
            node: Some(node::Node::Overlay(independent)),
        },
        vec![],
        "root",
        "a polar plot has one scale of `CHANNEL_THETA`, and it has 2",
    );

    // The children of an overlay draw one part of one circle.
    fails(
        overlay(vec![
            leaf(rose("band", vertical, Default::default())),
            leaf(rose("band", vertical, half)),
        ]),
        vec![],
        "root/children[1]",
        "the nodes inside an overlay set the coordinate system of the overlay or none",
    );
}

/// A text in a polar plot that runs along its spoke or its circle takes no `angle`, which its
/// orient replaces.
#[test]
fn a_polar_text_with_an_orient_takes_no_angle() {
    let mut ring = unit(
        mark::Mark::PolarText(proto::PolarTextMark {
            orient: proto::PolarTextOrient::Tangential as i32,
            ..Default::default()
        }),
        vec![
            raw(Channel::Text, "group"),
            scaled(Channel::Theta, "x", "linear"),
            raw(Channel::Radius, "width"),
        ],
    );
    ring.coord = polar();
    chart(leaf(ring.clone()), vec![]).unwrap();
    set(&mut ring, raw(Channel::Angle, "width"));
    fails(
        leaf(ring),
        vec![],
        "root",
        "a text with an orient does not encode `angle`",
    );
}
