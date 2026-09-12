//! Render the same scene builder used by the native and browser hosts.
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use winit_panels::{scene, state::State};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/panels-gallery".into()),
    );
    std::fs::create_dir_all(&output)?;
    let engine = d3_text_engine();
    for name in ["panels", "wrapped", "independent", "units"] {
        let mut state = State::new(engine.clone());
        match name {
            "wrapped" => {
                state.size = [940.0, 1000.0];
                state.missing = 2;
                state.overlay = true;
                state.legend_bottom = true;
            }
            "independent" => {
                state.preset = 1;
                state.y_scope = 0;
            }
            "units" => {
                state.preset = 2;
                state.overlay = true;
            }
            _ => {}
        }
        let result = scene::build(&state)?;
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: state.size,
                scale: 1.0,
            },
            CanvasConfig {
                text_engine: Some(engine.clone()),
                ..Default::default()
            },
        ))?;
        canvas.set_scene(&result.scene)?;
        pollster::block_on(canvas.render())?.save(output.join(format!("{name}.png")))?;
        println!(
            "{name}: {} passes, {} guides, fallback={}",
            result.iterations,
            result.plan.instances().count(),
            result.fallback
        );
    }
    Ok(())
}

fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine()
        .with_number_formatting(
            avenger_text::NumberFormatConfig::new("d3"),
            std::sync::Arc::new(registry),
        )
        .with_datetime_formatting(
            avenger_text::DateTimeFormatConfig::new("d3"),
            std::sync::Arc::new({
                let mut registry = avenger_text::DateTimeFormatRegistry::default();
                registry.register(
                    "d3",
                    std::sync::Arc::new(avenger_format_datetime_d3::D3DateTimeFormatProvider),
                );
                registry
            }),
        )
}
