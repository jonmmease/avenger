//! Render selection, composition, and panning at two raster scales.
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, PngCanvas};
use avenger_widgets::text_edit::Action;
use std::path::PathBuf;
use winit_annotation_editor::{
    scene,
    state::{Sample, State},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&output)?;
    let engine = avenger_typst_label::bundled_label_engine();
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
                Default::default(),
            ))?;
            canvas.set_scene(&scene, &state.engine)?;
            let image = pollster::block_on(canvas.render())?;
            assert_eq!(image.width(), (state.size[0] * scale) as u32);
            assert_eq!(image.height(), (state.size[1] * scale) as u32);
            image.save(output.join(format!("annotation-editor-{kind}-{scale}x.png")))?;
        }
    }
    Ok(())
}
