//! Focus strategies find the data under the pointer, with the rows that the scene data pairs with
//! each instance, and anchors on the instances' drawn edges.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch};
use avenger_geometry::data::{SceneData, SceneDataError};
use avenger_geometry::focus::{Focus, FocusQuery, FocusStrategy, Side};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{
    line::{LineHit, SceneLineMark},
    mark::{MarkInstance, SceneMark},
    rect::SceneRectMark,
    symbol::SceneSymbolMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::bundled_label_engine;

fn scene(marks: Vec<SceneMark>) -> SceneGraph {
    SceneGraph {
        width: 200.0,
        height: 200.0,
        origin: [0.0, 0.0],
        marks,
    }
}

/// Circles named `name`, of area 100 so their radius is 5, at `points`.
fn points(name: &str, points: &[[f32; 2]]) -> SceneMark {
    SceneSymbolMark {
        name: name.into(),
        len: points.len() as u32,
        x: points.iter().map(|p| p[0]).collect::<Vec<_>>().into(),
        y: points.iter().map(|p| p[1]).collect::<Vec<_>>().into(),
        size: 100.0.into(),
        ..Default::default()
    }
    .into()
}

fn rows(values: &[f64]) -> Arc<RecordBatch> {
    let values: ArrayRef = Arc::new(Float64Array::from(values.to_vec()));
    Arc::new(RecordBatch::try_from_iter([("v", values)]).unwrap())
}

fn data(name: &str, values: &[f64]) -> SceneData {
    let mut data = SceneData::new();
    data.insert(name, rows(values));
    data
}

fn build(marks: Vec<SceneMark>, data: &SceneData) -> Result<SceneGraphRTree, SceneDataError> {
    SceneGraphRTree::from_scene_graph_with_data(&scene(marks), data, &bundled_label_engine())
}

/// The instance index of the focus's one member.
fn focused(tree: &SceneGraphRTree, pointer: [f32; 2], strategy: FocusStrategy) -> Option<usize> {
    let focus = tree.focus(FocusQuery::at(pointer, strategy))?;
    assert_eq!(focus.members.len(), 1);
    focus.members[0].mark().instance_index
}

/// Scene data pairs each instance with its row, by mark name, skips the rows of a mark that the
/// scene doesn't draw, and fails when a name names several marks, or the rows don't number the
/// mark's instances.
#[test]
fn scene_data_pairs_rows_with_instances() {
    let three = [[10.0, 10.0], [50.0, 10.0], [90.0, 10.0]];
    let tree = build(
        vec![points("points", &three)],
        &data("points", &[1.0, 2.0, 3.0]),
    )
    .unwrap();
    let focus = tree
        .focus(FocusQuery::at([52.0, 12.0], FocusStrategy::default()))
        .unwrap();
    let datum = &focus.members[0].datum;
    assert_eq!(datum.row, 1);
    let values = datum.rows.column(0).as_any().downcast_ref::<Float64Array>();
    assert_eq!(values.unwrap().value(datum.row), 2.0);
    let instance = MarkInstance {
        name: "points".into(),
        mark_path: vec![0],
        instance_index: Some(2),
    };
    assert_eq!(tree.datum(&instance).unwrap().row, 2);

    // Rows for a mark that the scene doesn't draw, as when a view leaves a layer out whole,
    // pair with nothing, and the marks that it draws still pair with theirs.
    let mut culled = data("points", &[1.0, 2.0, 3.0]);
    culled.insert("other", rows(&[1.0]));
    let tree = build(vec![points("points", &three)], &culled).unwrap();
    assert_eq!(tree.datum(&instance).unwrap().row, 2);

    let error = |marks, data| build(marks, &data).unwrap_err();
    assert_eq!(
        error(
            vec![points("points", &three), points("points", &three)],
            data("points", &[1.0, 2.0, 3.0])
        ),
        SceneDataError::SharedName("points".into())
    );
    assert_eq!(
        error(vec![points("points", &three)], data("points", &[1.0, 2.0])),
        SceneDataError::RowCount {
            mark: "points".into(),
            instances: 3,
            rows: 2
        }
    );
}

/// `Nearest` takes the instance nearest the pointer within its radius, `NearestX` the nearest
/// along x whatever the y distance, and `Exact` the topmost instance under the pointer. Marks
/// without rows, such as an axis's, are never in a focus.
#[test]
fn strategies_find_the_instance_they_describe() {
    let tree = build(
        vec![
            points("points", &[[10.0, 190.0], [40.0, 10.0]]),
            points("axis", &[[12.0, 12.0]]),
        ],
        &data("points", &[1.0, 2.0]),
    )
    .unwrap();
    let nearest = FocusStrategy::Nearest { max_radius: 40.0 };
    assert_eq!(focused(&tree, [12.0, 10.0], nearest), Some(1));
    assert_eq!(focused(&tree, [120.0, 100.0], nearest), None);
    let nearest_x = FocusStrategy::NearestX { max_radius: 40.0 };
    assert_eq!(focused(&tree, [12.0, 10.0], nearest_x), Some(0));

    let rects = SceneRectMark {
        name: "rects".into(),
        len: 2,
        x: vec![10.0, 20.0].into(),
        y: 10.0.into(),
        width: Some(30.0.into()),
        height: Some(30.0.into()),
        ..Default::default()
    };
    let tree = build(vec![rects.into()], &data("rects", &[1.0, 2.0])).unwrap();
    assert_eq!(focused(&tree, [25.0, 25.0], FocusStrategy::Exact), Some(1));
    assert_eq!(focused(&tree, [60.0, 25.0], FocusStrategy::Exact), None);
}

/// A circle's anchor on each side lies on its drawn edge, and a rect's on the middle of the
/// side's edge.
#[test]
fn anchors_lie_on_drawn_edges() {
    let tree = build(
        vec![points("points", &[[50.0, 50.0]])],
        &data("points", &[1.0]),
    )
    .unwrap();
    let member = &tree
        .focus(FocusQuery::at([50.0, 50.0], FocusStrategy::Exact))
        .unwrap()
        .members[0];
    assert_eq!(member.anchor(Side::Right), [55.0, 50.0]);
    assert_eq!(member.anchor(Side::Above), [50.0, 45.0]);

    let rect = SceneRectMark {
        name: "rect".into(),
        x: 10.0.into(),
        y: 10.0.into(),
        width: Some(20.0.into()),
        height: Some(10.0.into()),
        ..Default::default()
    };
    let tree = build(vec![rect.into()], &data("rect", &[1.0])).unwrap();
    let member = &tree
        .focus(FocusQuery::at([20.0, 15.0], FocusStrategy::Exact))
        .unwrap()
        .members[0];
    assert_eq!(member.anchor(Side::Right), [30.0, 15.0]);
    assert_eq!(member.anchor(Side::Below), [20.0, 20.0]);
}

/// A line named `name` at height `y`, with a vertex at each of `xs`, which hits find.
fn line(name: &str, xs: &[f32], y: f32) -> SceneMark {
    SceneLineMark {
        name: name.into(),
        len: xs.len() as u32,
        x: xs.to_vec().into(),
        y: y.into(),
        hit: LineHit::Vertices,
        ..Default::default()
    }
    .into()
}

/// The lines and their rows: dense `a` and `b`, sparse `c`, and `d`, which ends at x = 50.
fn series() -> SceneGraphRTree {
    let dense: Vec<f32> = (0..100).map(|i| i as f32 * 2.0).collect();
    let sparse: Vec<f32> = (0..7).map(|i| i as f32 * 30.0).collect();
    let short: Vec<f32> = (0..6).map(|i| i as f32 * 10.0).collect();
    let mut data = SceneData::new();
    for (name, xs) in [("a", &dense), ("b", &dense), ("c", &sparse), ("d", &short)] {
        let values: Vec<f64> = xs.iter().map(|x| f64::from(*x)).collect();
        data.insert(name, rows(&values));
    }
    let marks = vec![
        line("a", &dense, 20.0),
        line("b", &dense, 60.0),
        line("c", &sparse, 100.0),
        line("d", &short, 140.0),
    ];
    build(marks, &data).unwrap()
}

/// The names and x positions of a focus's members.
fn members(focus: &Focus) -> Vec<(&str, f32)> {
    focus
        .members
        .iter()
        .map(|member| (member.mark().name.as_str(), member.position[0]))
        .collect()
}

/// `GroupX` takes each line's vertex nearest the pointer's x, even a sparse line's, leaves out
/// a line beyond its radius, and lists the members down the scene. A vertex's row is the
/// vertex's.
#[test]
fn group_x_takes_each_lines_vertex_nearest_the_pointer() {
    let tree = series();
    let strategy = FocusStrategy::GroupX {
        max_radius: 20.0,
        limit: None,
    };
    let focus = tree.focus(FocusQuery::at([115.4, 70.0], strategy)).unwrap();
    assert_eq!(members(&focus), [("a", 116.0), ("b", 116.0), ("c", 120.0)]);
    let datum = &focus.members[2].datum;
    assert_eq!(datum.row, 4);
}

/// With a limit, `GroupX` keeps the lines nearest the pointer's y, in their order down the
/// scene, and a shown line keeps its place until another is nearer by more than a pixel. A
/// query among some marks finds only their lines.
#[test]
fn group_x_keeps_the_lines_nearest_the_pointer_in_y() {
    let tree = series();
    let strategy = FocusStrategy::GroupX {
        max_radius: 20.0,
        limit: Some(2),
    };
    let names = |focus: Focus| -> Vec<String> {
        focus
            .members
            .iter()
            .map(|m| m.mark().name.clone())
            .collect()
    };
    let near = |y| tree.focus(FocusQuery::at([115.0, y], strategy)).unwrap();
    assert_eq!(names(near(70.0)), ["b", "c"]);
    assert_eq!(names(near(50.0)), ["a", "b"]);

    // `c` is nearer than `a` at y = 60.2, by less than a pixel, so a shown `a` stays.
    let shown = near(50.0);
    let showing = |y| {
        tree.focus(FocusQuery::at([115.0, y], strategy).showing(&shown))
            .unwrap()
    };
    assert_eq!(names(near(60.2)), ["b", "c"]);
    assert_eq!(names(showing(60.2)), ["a", "b"]);
    assert_eq!(names(showing(61.5)), ["b", "c"]);

    // Among `a` and `c` alone, the two lines are those two, wherever the pointer is.
    let marks = ["a".to_string(), "c".to_string()];
    let among = tree.focus(FocusQuery::at([115.0, 70.0], strategy).among(&marks));
    assert_eq!(names(among.unwrap()), ["a", "c"]);
}

/// A datum finds its row in its own rows by index, and in other rows only by a key that a row
/// holds: not without a key, nor by a null key.
#[test]
fn a_datum_finds_its_row_by_its_key() {
    use arrow_array::Int64Array;
    use avenger_geometry::data::Datum;
    let rows = |ids: Vec<Option<i64>>| {
        let ids: ArrayRef = Arc::new(Int64Array::from(ids));
        Arc::new(RecordBatch::try_from_iter([("id", ids)]).unwrap())
    };
    let own = rows(vec![Some(7), Some(8), None]);
    let datum = |row| Datum {
        rows: own.clone(),
        row,
    };
    assert_eq!(datum(1).row_in(&own, None), Some(1));
    let reordered = rows(vec![Some(8), None, Some(7)]);
    assert_eq!(datum(0).row_in(&reordered, None), None);
    assert_eq!(datum(0).row_in(&reordered, Some("id")), Some(2));
    assert_eq!(datum(2).row_in(&reordered, Some("id")), None);
    assert_eq!(datum(0).row_in(&reordered, Some("name")), None);
    assert_eq!(datum(0).row_in(&rows(vec![Some(9)]), Some("id")), None);
}
