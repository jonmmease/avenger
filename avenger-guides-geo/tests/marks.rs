use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_guides_geo::{geometry_marks, GeometryStyle};
use avenger_scenegraph::marks::{mark::SceneMark, path::ScenePathMark, symbol::SceneSymbolMark};
use geo_types::{Geometry, GeometryCollection, LineString, MultiPoint, Point, Polygon};
use geoarrow_array::builder::GeometryBuilder;
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::{GeometryType, Metadata};
use std::sync::Arc;

/// A mixed screen-space array, with `None` as a null row.
fn mixed(rows: &[Option<Geometry<f64>>]) -> Arc<dyn GeoArrowArray> {
    let mut builder = GeometryBuilder::new(GeometryType::new(Arc::new(Metadata::default())));
    for row in rows {
        builder.push_geometry(row.as_ref()).unwrap();
    }
    Arc::new(builder.finish())
}

fn square() -> Geometry<f64> {
    Geometry::Polygon(Polygon::new(
        LineString::from(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
        vec![],
    ))
}

fn line() -> Geometry<f64> {
    Geometry::LineString(LineString::from(vec![(0.0, 20.0), (10.0, 30.0)]))
}

fn colors(count: usize) -> ScalarOrArray<ColorOrGradient> {
    ScalarOrArray::new_array(
        (0..count)
            .map(|row| ColorOrGradient::Color([row as f32 / 10.0, 0.0, 0.0, 1.0]))
            .collect(),
    )
}

/// The path marks, in drawing order, and the symbol mark, which draws last.
fn split(marks: &[SceneMark]) -> (Vec<&ScenePathMark>, Option<&SceneSymbolMark>) {
    let mut paths = Vec::new();
    let mut symbol = None;
    for mark in marks {
        match mark {
            SceneMark::Path(mark) => {
                assert!(symbol.is_none(), "the symbols draw on top");
                paths.push(mark);
            }
            SceneMark::Symbol(mark) => symbol = Some(mark),
            other => panic!("unexpected {other:?}"),
        }
    }
    (paths, symbol)
}

const TRANSPARENT: ColorOrGradient = ColorOrGradient::Color([0.0; 4]);

#[test]
fn rows_split_into_a_path_mark_and_a_symbol_mark_by_row() {
    let rows = [
        Some(square()),
        Some(line()),
        Some(Geometry::Point(Point::new(5.0, 5.0))),
        None,
        Some(Geometry::Point(Point::new(f64::NAN, f64::NAN))),
    ];
    let style = GeometryStyle {
        fill: colors(rows.len()),
        ..Default::default()
    };
    let group = geometry_marks(mixed(&rows).as_ref(), &style).unwrap();
    let (paths, Some(symbol)) = split(&group.marks) else {
        panic!("a symbol mark");
    };
    let [polygons, lines] = paths[..] else {
        panic!("a polygon mark and a line mark");
    };
    assert_eq!(polygons.len, 5);
    assert_eq!(polygons.indices.as_deref(), Some(&vec![0]));
    assert_eq!(polygons.fill, style.fill);
    assert_eq!(lines.len, 5);
    assert_eq!(lines.indices.as_deref(), Some(&vec![1]));
    // The line isn't filled.
    assert_eq!(lines.fill.as_vec(5, None)[1], TRANSPARENT);

    // One point per row keeps the symbols row-aligned. The empty point draws nothing.
    assert_eq!(symbol.len, 5);
    assert_eq!(symbol.indices.as_deref(), Some(&vec![2]));
    assert_eq!(symbol.x.as_vec(5, None)[2], 5.0);
    assert_eq!(
        symbol.fill.as_vec(5, None)[2],
        ColorOrGradient::Color([0.2, 0.0, 0.0, 1.0])
    );
}

#[test]
fn a_row_with_several_points_gives_a_symbol_each() {
    let rows = [
        Some(Geometry::Point(Point::new(1.0, 1.0))),
        Some(Geometry::MultiPoint(MultiPoint::from(vec![
            (2.0, 2.0),
            (3.0, 3.0),
        ]))),
    ];
    let style = GeometryStyle {
        fill: colors(rows.len()),
        point_size: ScalarOrArray::new_array(vec![10.0, 20.0]),
        ..Default::default()
    };
    let group = geometry_marks(mixed(&rows).as_ref(), &style).unwrap();
    let (paths, Some(symbol)) = split(&group.marks) else {
        panic!("a symbol mark");
    };
    assert!(paths.is_empty());
    // The instances are points, with their rows' styles.
    assert_eq!(symbol.len, 3);
    assert!(symbol.indices.is_none());
    assert_eq!(symbol.x.as_vec(3, None), [1.0, 2.0, 3.0]);
    assert_eq!(symbol.size.as_vec(3, None), [10.0, 20.0, 20.0]);
}

#[test]
fn rows_take_their_own_point_shapes() {
    let square = SymbolShape::from_vega_str("square").unwrap();
    let rows = [
        Some(Geometry::Point(Point::new(1.0, 1.0))),
        Some(Geometry::Point(Point::new(2.0, 2.0))),
        Some(Geometry::Point(Point::new(3.0, 3.0))),
    ];
    let style = GeometryStyle {
        point_shape: ScalarOrArray::new_array(vec![
            square.clone(),
            SymbolShape::Circle,
            square.clone(),
        ]),
        ..Default::default()
    };
    let group = geometry_marks(mixed(&rows).as_ref(), &style).unwrap();
    let (_, Some(symbol)) = split(&group.marks) else {
        panic!("a symbol mark");
    };
    // Each shape appears once, in the order of its first row.
    assert_eq!(symbol.shapes, [square.clone(), SymbolShape::Circle]);
    assert_eq!(symbol.shape_index.as_vec(3, None), [0, 1, 0]);

    // A row with several points gives each of its symbols the row's shape.
    let rows = [
        Some(Geometry::MultiPoint(MultiPoint::from(vec![
            (1.0, 1.0),
            (2.0, 2.0),
        ]))),
        Some(Geometry::Point(Point::new(3.0, 3.0))),
    ];
    let style = GeometryStyle {
        point_shape: ScalarOrArray::new_array(vec![SymbolShape::Circle, square.clone()]),
        ..Default::default()
    };
    let group = geometry_marks(mixed(&rows).as_ref(), &style).unwrap();
    let (_, Some(symbol)) = split(&group.marks) else {
        panic!("a symbol mark");
    };
    assert_eq!(symbol.shapes, [SymbolShape::Circle, square]);
    assert_eq!(symbol.shape_index.as_vec(3, None), [0, 0, 1]);
}

#[test]
fn polygons_alone_keep_their_fill_and_empty_arrays_draw_nothing() {
    let style = GeometryStyle::default();
    let group = geometry_marks(mixed(&[Some(square())]).as_ref(), &style).unwrap();
    let (paths, None) = split(&group.marks) else {
        panic!("no symbols");
    };
    let [path] = paths[..] else {
        panic!("only a path mark");
    };
    assert_eq!(path.fill, style.fill);
    let group = geometry_marks(mixed(&[None]).as_ref(), &style).unwrap();
    assert!(group.marks.is_empty());
}

#[test]
fn style_arrays_need_a_value_for_each_row() {
    let geometry = mixed(&[Some(square()), Some(line())]);
    let short = GeometryStyle {
        fill: colors(1),
        ..Default::default()
    };
    assert!(geometry_marks(geometry.as_ref(), &short).is_err());
    let sizes = GeometryStyle {
        point_size: ScalarOrArray::new_array(vec![4.0; 3]),
        ..Default::default()
    };
    assert!(geometry_marks(geometry.as_ref(), &sizes).is_err());
    let shapes = GeometryStyle {
        point_shape: ScalarOrArray::new_array(vec![SymbolShape::Circle]),
        ..Default::default()
    };
    assert!(geometry_marks(geometry.as_ref(), &shapes).is_err());
    let fitting = GeometryStyle {
        fill: colors(2),
        ..Default::default()
    };
    assert!(geometry_marks(geometry.as_ref(), &fitting).is_ok());
}

/// A row of a polygon and a line fills the polygon and not the line.
#[test]
fn a_row_of_a_polygon_and_a_line_fills_only_the_polygon() {
    let row = Geometry::GeometryCollection(GeometryCollection(vec![square(), line()]));
    let style = GeometryStyle::default();
    let group = geometry_marks(mixed(&[Some(row)]).as_ref(), &style).unwrap();
    let (paths, None) = split(&group.marks) else {
        panic!("no symbols");
    };
    let [polygons, lines] = paths[..] else {
        panic!("a polygon mark and a line mark");
    };
    assert_eq!(polygons.indices.as_deref(), Some(&vec![0]));
    assert_eq!(polygons.fill, style.fill);
    assert_eq!(lines.indices.as_deref(), Some(&vec![0]));
    assert_eq!(lines.fill.as_vec(1, None), [TRANSPARENT]);
}
