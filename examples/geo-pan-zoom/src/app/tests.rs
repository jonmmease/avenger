//! The maps' readouts, as the cursor hovers and drags a map, and the earthquakes that a box
//! selects.

use super::*;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput, WindowResizeEvent,
};

/// Sends `events` to `app` a frame apart, so that no throttle drops one.
fn update(app: &mut AvengerApp<AppState>, events: impl IntoIterator<Item = WindowEvent>) {
    for event in events {
        std::thread::sleep(Duration::from_millis(20));
        pollster::block_on(app.update(&event, Instant::now()));
    }
}

fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

/// The scene indices of the brushes, the readouts, and the tooltips, in z-order after the maps.
const BRUSHES_SCENE: usize = 1;
const READOUTS_SCENE: usize = 2;
const TOOLTIPS_SCENE: usize = 4;

/// The readout of the map at `index`.
fn readout(app: &AvengerApp<AppState>, index: usize) -> String {
    let SceneMark::Text(text) = &app.scenes()[READOUTS_SCENE].marks[index] else {
        panic!("each map has a readout");
    };
    text.text.as_vec(1, None).remove(0)
}

/// Hover updates readouts and tooltips without rebuilding the maps.
/// Dragging preserves the geographic location under the cursor and its readout,
/// including after release.
#[test]
fn a_drag_keeps_the_readout_on_its_place() {
    let mut app = pollster::block_on(builder(AppState::new(None)).build()).unwrap();
    let size = [1400.0, 900.0];
    update(
        &mut app,
        [WindowEvent::WindowResize(WindowResizeEvent { size })],
    );
    let rect = app.state().maps[1].rect;
    let start = [rect.x + rect.width / 2.0, rect.y + rect.height / 2.0];
    std::thread::sleep(Duration::from_millis(20));
    let hover = pollster::block_on(app.update(&moved(start), Instant::now()));
    assert_eq!(hover.scenes, [READOUTS_SCENE, TOOLTIPS_SCENE]);
    let place = readout(&app, 1);
    assert_ne!(place, app.state().maps[1].spec.hint);

    let end = [start[0] + 25.0, start[1] + 15.0];
    update(&mut app, [mouse(ElementState::Pressed), moved(end)]);
    assert_eq!(readout(&app, 1), place);
    update(&mut app, [mouse(ElementState::Released)]);
    assert_eq!(readout(&app, 1), place);
}

/// The app in a window large enough for both maps.
fn large_app() -> AvengerApp<AppState> {
    let mut app = pollster::block_on(builder(AppState::new(None)).build()).unwrap();
    let size = [1400.0, 900.0];
    update(
        &mut app,
        [WindowEvent::WindowResize(WindowResizeEvent { size })],
    );
    app
}

/// The number of rings that the maps draw around selected earthquakes.
fn rings(app: &AvengerApp<AppState>) -> usize {
    fn count(marks: &[SceneMark]) -> usize {
        (marks.iter())
            .map(|mark| match mark {
                SceneMark::Symbol(rings) if rings.name == "selected earthquakes" => {
                    rings.len as usize
                }
                SceneMark::Group(group) => count(&group.marks),
                _ => 0,
            })
            .sum()
    }
    count(&app.scenes()[0].marks)
}

/// In the Box mode, a drag on the world map selects the earthquakes inside its box, leaves the
/// map's projection where it was, and rings them on the maps. Both maps draw the box.
#[test]
fn a_box_selects_earthquakes() {
    let mut app = large_app();
    app.state_mut().drag = DragMode::Box;
    let before = app.state().maps[1].projection.clone().unwrap();
    let rect = app.state().maps[1].rect;
    let start = [rect.x + rect.width * 0.3, rect.y + rect.height * 0.3];
    let end = [rect.x + rect.width * 0.7, rect.y + rect.height * 0.6];
    let (pressed, released) = (mouse(ElementState::Pressed), mouse(ElementState::Released));
    update(&mut app, [moved(start), pressed, moved(end), released]);
    let after = app.state().maps[1].projection.clone().unwrap();
    assert!(same_view(&before, &after));
    let Some(brush) = app.state().brush else {
        panic!("the maps have a box");
    };
    // Both maps draw the box, each through its own projection.
    let boxes = app.scenes()[BRUSHES_SCENE].marks.iter().filter(|mark| {
        let SceneMark::Group(group) = mark else {
            return false;
        };
        (group.marks.iter())
            .any(|mark| matches!(mark, SceneMark::Path(path) if path.name == "brush_box"))
    });
    assert_eq!(boxes.count(), 2);
    let selected = app.state().selected();
    assert!(!selected.is_empty());
    let places = &app.state().layers.quake_places;
    assert!(selected.iter().all(|&row| brush.contains(places[row])));
    assert!(rings(&app) >= selected.len());
}
