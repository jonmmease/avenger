//! A hit-testing tree pairs its instances with the rows behind their marks, from the build that
//! made the tree.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch};
use avenger_geometry::data::{SceneData, SceneDataError};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::bundled_label_engine;

/// `len` points named `name`, 20 pixels apart along x.
fn points(name: &str, len: u32) -> SceneMark {
    let x: Vec<f32> = (0..len).map(|i| 10.0 + 20.0 * i as f32).collect();
    SceneSymbolMark {
        name: name.into(),
        len,
        x: x.into(),
        y: 10.0.into(),
        size: 100.0.into(),
        ..Default::default()
    }
    .into()
}

/// Rows with an `id` column, from 0 up to `len`.
fn rows(len: i32) -> Arc<RecordBatch> {
    let ids: ArrayRef = Arc::new(Int32Array::from_iter_values(0..len));
    Arc::new(RecordBatch::try_from_iter([("id", ids)]).unwrap())
}

/// The tree of a scene of `marks`, with `len` rows for the mark named `name`.
fn tree(marks: Vec<SceneMark>, name: &str, len: i32) -> Result<SceneGraphRTree, SceneDataError> {
    let scene = SceneGraph {
        marks,
        width: 100.0,
        height: 20.0,
        origin: [0.0; 2],
    };
    let mut data = SceneData::new();
    data.insert(name, rows(len));
    SceneGraphRTree::from_scene_graph_with_data(&scene, &data, &bundled_label_engine())
}

/// Each instance's datum is its mark's row at the instance's index, and an instance of a mark
/// without rows has none.
#[test]
fn a_tree_pairs_each_instance_with_its_row() {
    let tree = tree(vec![points("points", 3), points("other", 1)], "points", 3).unwrap();
    let mut paired = 0;
    for instance in tree.iter() {
        let mark = &instance.mark_instance;
        let datum = tree.datum(mark);
        if mark.name == "points" {
            assert_eq!(datum.map(|datum| datum.row), mark.instance_index);
            paired += 1;
        } else {
            assert!(datum.is_none());
        }
    }
    assert_eq!(paired, 3);
}

/// Rows that don't match their scene are errors: rows for a name that several marks share, and
/// rows that don't number their mark's instances. Rows for a mark that the scene leaves out pair
/// with nothing.
#[test]
fn rows_must_match_their_scene() {
    let shared = tree(vec![points("points", 2), points("points", 2)], "points", 2);
    assert_eq!(
        shared.err(),
        Some(SceneDataError::SharedName("points".to_string()))
    );
    let short = tree(vec![points("points", 3)], "points", 2);
    assert_eq!(
        short.err(),
        Some(SceneDataError::RowCount {
            mark: "points".to_string(),
            instances: 3,
            rows: 2,
        })
    );
    let undrawn = tree(vec![points("points", 3)], "countries", 5).unwrap();
    assert!((undrawn.iter()).all(|instance| undrawn.datum(&instance.mark_instance).is_none()));
}
