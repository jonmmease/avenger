//! The crosshair over the chart, as a scroll on macOS drives it: each wheel turn comes after a
//! move to where the pointer already is.

use super::*;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseWheel,
};
use avenger_geometry::query::Extent;
use avenger_geometry::rtree::SceneGraphRTree;

async fn app() -> AvengerApp<ChartState> {
    builder().build().await.unwrap()
}

/// The tree of the scene that the chart draws now.
fn drawn(app: &AvengerApp<ChartState>) -> SceneGraphRTree {
    SceneGraphRTree::from_scene_graph(&app.scenes()[0], app.text_engine())
}

/// How far `at` lies from the center of the nearest flower that the chart draws now.
fn gap_to_a_drawn_flower(app: &AvengerApp<ChartState>, at: [f32; 2]) -> f32 {
    let tree = drawn(app);
    let nearest = Nearest::new(at, f32::INFINITY)
        .extent(Extent::Geometry)
        .filter(|mark| mark.name == FLOWERS);
    let gap = tree.nearest(nearest).next().map(|flower| flower.distance);
    gap.unwrap()
}

/// A position 4 pixels off the drawn flower nearest the plot's center.
fn near_a_flower(app: &mut AvengerApp<ChartState>) -> [f32; 2] {
    let plot = app.state_mut().plot;
    let center = [plot.x + plot.width / 2.0, plot.y + plot.height / 2.0];
    let tree = drawn(app);
    let nearest = Nearest::new(center, f32::INFINITY).filter(|mark| mark.name == FLOWERS);
    let flower = tree.nearest(nearest).next().unwrap();
    let Geometry::Point(point) = flower.instance.geometry else {
        panic!("a flower's geometry is its center");
    };
    [point.x() + 4.0, point.y() + 4.0]
}

/// The center of the crosshair's ring, when it snaps to a flower.
fn ring(app: &AvengerApp<ChartState>) -> Option<[f32; 2]> {
    app.scenes()[1].marks.iter().find_map(|mark| match mark {
        SceneMark::Symbol(ring) => Some([*ring.x.first()?, *ring.y.first()?]),
        _ => None,
    })
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

fn wheel() -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::PixelDelta(0.0, 20.0),
    })
}

/// A repeated pointer position after zoom does not schedule a render that would
/// cause the host to drop the next wheel event.
#[test]
fn a_scrolls_move_after_a_zoom_builds_nothing() {
    pollster::block_on(async {
        let mut app = app().await;
        let now = Instant::now();
        let at = near_a_flower(&mut app);
        app.update(&moved(at), now).await;
        assert_eq!(app.update(&wheel(), now).await.scenes, [0, 1]);
        assert!(app.update(&moved(at), now).await.scenes.is_empty());
    });
}

/// A zoom moves the flowers, and the crosshair draws no ring until the zoom settles and the
/// chart's tree builds again, when the ring circles a flower where the chart draws it.
#[test]
fn the_ring_waits_for_a_zoom_to_settle() {
    pollster::block_on(async {
        let mut app = app().await;
        let now = Instant::now();
        let at = near_a_flower(&mut app);
        app.update(&moved(at), now).await;
        let snapped = ring(&app).unwrap();
        assert!(gap_to_a_drawn_flower(&app, snapped) < 0.01);

        app.update(&wheel(), now).await;
        assert_eq!(ring(&app), None);
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert_eq!(app.refresh().await.scenes, [0, 1]);
        let snapped = ring(&app).unwrap();
        assert!(gap_to_a_drawn_flower(&app, snapped) < 0.01);
    });
}
