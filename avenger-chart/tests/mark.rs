//! The marks of each coordinate system: their kinds, types, names, and channels, and the view of
//! their properties that the marks of one type share.

use avenger_chart::{
    proto::{self, encoding, mark::Mark, Channel},
    BarProps, CoordKind, MarkType, MarkView,
};

fn column(name: &str) -> Option<proto::Encoding> {
    Some(proto::Encoding {
        value: Some(encoding::Value::Column(name.into())),
        ..Default::default()
    })
}

#[test]
fn each_mark_names_its_kind_and_its_type() {
    let marks = [
        (
            Mark::from(proto::SymbolMark::default()),
            CoordKind::Cartesian,
            MarkType::Symbol,
            "symbol",
        ),
        (
            proto::PolarRectMark::default().into(),
            CoordKind::Polar,
            MarkType::Rect,
            "polar_rect",
        ),
        (
            proto::GeoShapeMark::default().into(),
            CoordKind::Geo,
            MarkType::GeoShape,
            "geo_shape",
        ),
    ];
    for (mark, kind, mark_type, name) in marks {
        assert_eq!(mark.kind(), kind);
        assert_eq!(mark.mark_type(), mark_type);
        assert_eq!(mark.name(), name);
    }
}

#[test]
fn a_mark_takes_the_channels_of_its_fields_in_the_order_of_channel() {
    let line = Mark::from(proto::PolarLineMark::default());
    assert_eq!(
        line.channels(),
        [
            Channel::Theta,
            Channel::Radius,
            Channel::Color,
            Channel::Stroke,
            Channel::Opacity,
            Channel::StrokeOpacity,
            Channel::StrokeWidth,
            Channel::StrokeDash,
            Channel::Detail,
        ]
    );
    let mut symbol = Mark::from(proto::SymbolMark {
        color: column("group"),
        x: column("x"),
        ..Default::default()
    });
    let channels: Vec<Channel> = symbol.encodings().map(|(channel, _)| channel).collect();
    assert_eq!(channels, [Channel::X, Channel::Color]);
    assert!(symbol.encoding_mut(Channel::Theta).is_none());
    *symbol.encoding_mut(Channel::Y).unwrap() = column("y");
    assert_eq!(symbol.encodings().count(), 3);
    assert_eq!(Channel::StrokeDash.field_name(), "stroke_dash");
}

#[test]
fn the_marks_of_one_type_read_alike_in_every_kind() {
    let cartesian = Mark::from(proto::BarMark {
        orient: proto::Orient::Horizontal as i32,
        corner_radius: Some(2.0),
        ..Default::default()
    });
    let polar = Mark::from(proto::PolarBarMark {
        orient: proto::Orient::Horizontal as i32,
        corner_radius: Some(2.0),
        route: proto::Route::Canvas as i32,
        ..Default::default()
    });
    assert_eq!(
        cartesian.view(),
        MarkView::Bar(BarProps {
            orient: proto::Orient::Horizontal,
            corner_radius: Some(2.0),
            route: None,
        })
    );
    assert_eq!(
        polar.view(),
        MarkView::Bar(BarProps {
            orient: proto::Orient::Horizontal,
            corner_radius: Some(2.0),
            route: Some(proto::Route::Canvas),
        })
    );
}
