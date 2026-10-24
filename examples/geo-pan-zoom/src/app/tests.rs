//! The maps' readouts, as the cursor hovers and drags a map.

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

/// The readout of the map at `index`.
fn readout(app: &AvengerApp<AppState>, index: usize) -> String {
    let SceneMark::Text(text) = &app.scenes()[1].marks[index] else {
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
    assert_eq!(hover.scenes, [1, 2]);
    let place = readout(&app, 1);
    assert_ne!(place, app.state().maps[1].spec.hint);

    let end = [start[0] + 25.0, start[1] + 15.0];
    update(&mut app, [mouse(ElementState::Pressed), moved(end)]);
    assert_eq!(readout(&app, 1), place);
    update(&mut app, [mouse(ElementState::Released)]);
    assert_eq!(readout(&app, 1), place);
}
