use avenger_common::time::Instant;
use avenger_eventstream::window::*;
#[test]
fn keyboard_updates_the_shared_application_model_before_pointer_movement() {
    let mut app = pollster::block_on(winit_widgets::make_app(winit_widgets::state::State::new(
        avenger_text::default_text_engine(),
    )))
    .unwrap();
    for expected in ["lock", "layers", "layers", "layers", "palette", "size"] {
        let update = pollster::block_on(app.update_with_status(
            &WindowEvent::KeyboardInput(WindowKeyboardInput {
                key: Key::Named(NamedKey::Tab),
                state: ElementState::Pressed,
                text: None,
                repeat: false,
            }),
            Instant::now(),
        ))
        .unwrap();
        assert!(update.status.rerender);
        assert_eq!(
            app.app_state_mut()
                .widgets
                .focused()
                .unwrap()
                .widget
                .as_str(),
            expected
        );
    }
    pollster::block_on(app.update_with_status(
        &WindowEvent::KeyboardInput(WindowKeyboardInput {
            key: Key::Named(NamedKey::End),
            state: ElementState::Pressed,
            text: None,
            repeat: false,
        }),
        Instant::now(),
    ))
    .unwrap();
    assert_eq!(app.app_state_mut().marker_size, 15.0);
}
