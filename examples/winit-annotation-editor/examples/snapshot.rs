//! Render selection, composition, and panning at two raster scales.
use avenger_common::canvas::CanvasDimensions;
use avenger_common::time::Instant;
use avenger_eventstream::{
    scene::SceneGraphEvent,
    window::{ImeEvent, SessionInputEvent, TextInputEvent},
};
use avenger_text::text_edit::{Affinity, Cursor, Granularity, SelectionState};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use std::path::PathBuf;
use winit_annotation_editor::{
    scene,
    state::{Sample, State},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&output)?;
    let engine = avenger_text::default_text_engine();
    for kind in ["selection", "composition", "panned"] {
        let mut state = State::new(Sample::A, 0, engine.clone());
        let initial = scene::build(&mut state)?;
        state.widgets.request_focus(
            Some(avenger_widgets::WidgetTarget::new("source")),
            Instant::now(),
        )?;
        state.widgets.set_text_selection(
            "source",
            SelectionState {
                anchor: Cursor::new(0, Affinity::Downstream),
                head: Cursor::new(state.draft.len(), Affinity::Upstream),
                granularity: Granularity::Char,
            },
            Instant::now(),
        )?;
        if kind == "composition" {
            let input = SessionInputEvent {
                session: state.widgets.active_input_session().unwrap().clone(),
                event: TextInputEvent::Ime(ImeEvent::Preedit {
                    text: "café".into(),
                    cursor: Some((3, 5)),
                }),
            };
            state.widgets.handle(
                &SceneGraphEvent::TextInput {
                    input,
                    modifiers: Default::default(),
                },
                &avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&initial),
                Instant::now(),
            )?;
        }
        if kind == "panned" {
            state.pan = [-46.25, 31.5];
        }
        let scene = scene::build(&mut state)?;
        for scale in [1.0, 2.0] {
            let mut canvas = pollster::block_on(PngCanvas::new(
                CanvasDimensions {
                    size: state.size,
                    scale,
                },
                CanvasConfig {
                    text_engine: Some(engine.clone()),
                    ..Default::default()
                },
            ))?;
            canvas.set_scene(&scene)?;
            let image = pollster::block_on(canvas.render())?;
            assert_eq!(image.width(), (state.size[0] * scale) as u32);
            assert_eq!(image.height(), (state.size[1] * scale) as u32);
            image.save(output.join(format!("annotation-editor-{kind}-{scale}x.png")))?;
        }
    }
    Ok(())
}
