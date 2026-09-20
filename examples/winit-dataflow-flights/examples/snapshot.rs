//! Render the native scene and optionally replay an empty selection without a window.
use anyhow::Result;
use avenger_common::{canvas::CanvasDimensions, time::Instant};
use avenger_eventstream::{
    runtime::RuntimeWakeEvent,
    window::{ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput},
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use winit_dataflow_flights::{
    config::Config,
    controller::{State, make_app},
    worker::completion_key,
};
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let output = args.get(1).cloned().unwrap_or("/tmp/flights.png".into());
    let size = if args.len() > 3 {
        [args[2].parse()?, args[3].parse()?]
    } else {
        [1280., 940.]
    };
    let engine = avenger_text::default_text_engine();
    let (state, worker) = State::load_at(Config::default(), engine.clone(), size).await?;
    let mut app = make_app(state).await?;
    if args.get(4).is_some_and(|s| s == "empty") {
        let r = app
            .app_state_mut()
            .widgets
            .semantics()
            .iter()
            .find(|s| s.target.widget.as_str() == "none")
            .unwrap()
            .bounds;
        for event in [
            WindowEvent::CursorMoved(WindowCursorMoved {
                position: [r.x + r.width / 2., r.y + r.height / 2.],
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            WindowEvent::MouseInput(WindowMouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
            }),
        ] {
            app.update(&event, Instant::now()).await?;
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while worker.completed.lock().unwrap().is_none() {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
        .await?;
        let generation = worker.completed.lock().unwrap().as_ref().unwrap().0;
        app.update(
            &WindowEvent::RuntimeWake(RuntimeWakeEvent {
                key: completion_key(),
                generation,
            }),
            Instant::now(),
        )
        .await?;
        anyhow::ensure!(
            app.app_state_mut().rendered.points.count == 0,
            "empty control failed"
        );
    }
    let mut canvas = PngCanvas::new(
        CanvasDimensions { size, scale: 1. },
        CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        },
    )
    .await?;
    canvas.set_scene(app.scene_graph())?;
    canvas.render().await?.save(&output)?;
    worker.shutdown();
    println!("Saved {output}");
    Ok(())
}
