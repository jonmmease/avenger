//! Render selection, composition, and panning at two raster scales.
use avenger_common::canvas::CanvasDimensions;
use avenger_text::text_edit::Action;
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
        state.focused = true;
        state.caret_visible = true;
        state.apply_action(Action::SelectAll);
        if kind == "composition" {
            state.apply_action(Action::Preedit {
                text: "café".into(),
                cursor: Some((3, 5)),
            });
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
