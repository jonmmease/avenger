//! The crosshair over the chart, as a scroll on macOS drives it: each wheel turn comes after a
//! move to where the pointer already is. The Select and Lasso modes select flowers, and the Zoom
//! mode zooms to a box.

use super::*;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseInput,
    WindowMouseWheel,
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

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

/// The number of flowers that the chart draws at full opacity, as it draws the selected ones.
fn opaque(app: &AvengerApp<ChartState>) -> usize {
    fn find(marks: &[SceneMark]) -> Option<&SceneSymbolMark> {
        marks.iter().find_map(|mark| match mark {
            SceneMark::Symbol(symbol) if symbol.name == FLOWERS => Some(symbol),
            SceneMark::Group(group) => find(&group.marks),
            _ => None,
        })
    }
    let flowers = find(&app.scenes()[0].marks).unwrap();
    (flowers.fill.as_vec(flowers.len as usize, None).iter())
        .filter(|fill| matches!(fill, ColorOrGradient::Color([.., alpha]) if *alpha == 1.0))
        .count()
}

/// In the Select mode, a drag selects flowers without panning and a click picks one. In the
/// Lasso mode, a drag selects the flowers inside its path. The chart draws selected flowers at
/// full opacity. In the Zoom mode, a drag zooms the scales to its box, and a double-click
/// returns to the first scales. With the zoom aspect checked, a zoom scales both axes by one
/// factor.
#[test]
fn the_selection_modes_select_flowers() {
    pollster::block_on(async {
        let mut app = app().await;
        let plot = app.state().plot;
        let at = |fx: f32, fy: f32| [plot.x + plot.width * fx, plot.y + plot.height * fy];
        let now = Instant::now();
        let (pressed, released) = (mouse(ElementState::Pressed), mouse(ElementState::Released));

        app.state_mut().drag = DragMode::Select;
        let domain = app.state().x_scale.numeric_interval_domain().unwrap();
        for event in [moved(at(0.25, 0.25)), pressed.clone(), moved(at(0.6, 0.7))] {
            app.update(&event, now).await;
        }
        app.update(&released, now).await;
        assert_eq!(
            app.state().x_scale.numeric_interval_domain().unwrap(),
            domain
        );
        assert!(app.state().brush.is_some() && opaque(&app) > 0);

        let tree = drawn(&app);
        let nearest = Nearest::new(at(0.5, 0.5), f32::INFINITY).filter(|m| m.name == FLOWERS);
        let Geometry::Point(point) = tree.nearest(nearest).next().unwrap().instance.geometry else {
            panic!("a flower's geometry is its center");
        };
        for event in [
            moved([point.x(), point.y()]),
            pressed.clone(),
            released.clone(),
        ] {
            app.update(&event, now).await;
        }
        assert_eq!(app.state().picked.len(), 1);

        app.state_mut().drag = DragMode::Lasso;
        let mut events = vec![moved(at(0.3, 0.3)), pressed.clone()];
        for step in 0..=40 {
            let angle = step as f32 / 40.0 * std::f32::consts::TAU;
            events.push(moved(at(0.5 - 0.2 * angle.cos(), 0.5 - 0.2 * angle.sin())));
        }
        events.push(released.clone());
        for event in events {
            app.update(&event, now).await;
        }
        assert!(!app.state().lasso.keys.is_empty());

        app.state_mut().drag = DragMode::Zoom;
        let (low, high) = app.state().x_scale.numeric_interval_domain().unwrap();
        let zoom = [
            moved(at(0.25, 0.25)),
            pressed.clone(),
            moved(at(0.6, 0.7)),
            released.clone(),
        ];
        for event in zoom.clone() {
            app.update(&event, now).await;
        }
        let (zoomed_low, zoomed_high) = app.state().x_scale.numeric_interval_domain().unwrap();
        assert!(zoomed_low > low && zoomed_high < high);
        assert!(app.state().zoom_box.is_none());

        // A double-click returns to the first scales.
        for event in [pressed.clone(), released.clone(), pressed, released] {
            app.update(&event, now).await;
        }
        let first = app.state().base_x_scale.numeric_interval_domain().unwrap();
        assert_eq!(
            app.state().x_scale.numeric_interval_domain().unwrap(),
            first
        );

        // With the zoom aspect checked, a zoom scales both axes by one factor.
        app.state_mut().zoom_aspect = true;
        let span = |scale: &ConfiguredScale| {
            let (low, high) = scale.numeric_interval_domain().unwrap();
            high - low
        };
        let before = [span(&app.state().x_scale), span(&app.state().y_scale)];
        for event in zoom {
            app.update(&event, now).await;
        }
        let after = [span(&app.state().x_scale), span(&app.state().y_scale)];
        let [x, y] = [after[0] / before[0], after[1] / before[1]];
        assert!(
            x < 1.0 && (x - y).abs() < 1e-6,
            "x zooms by {x} and y by {y}"
        );
    });
}

/// With the crosshair hidden, a move near a flower draws no crosshair and builds nothing.
#[test]
fn a_hidden_crosshair_draws_nothing() {
    pollster::block_on(async {
        let mut app = app().await;
        app.state_mut().show_crosshair = false;
        let at = near_a_flower(&mut app);
        assert!(app
            .update(&moved(at), Instant::now())
            .await
            .scenes
            .is_empty());
        assert_eq!(ring(&app), None);
    });
}
