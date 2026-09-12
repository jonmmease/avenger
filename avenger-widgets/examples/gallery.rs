//! Render deterministic widget states with the same text engine as measurement.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use avenger_color::ColorOrGradient;
    use avenger_common::{canvas::CanvasDimensions, time::Instant};
    use avenger_scenegraph::{
        marks::{rect::SceneRectMark, text::SceneTextMark},
        scene_graph::SceneGraph,
    };
    use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
    use avenger_widgets::prelude::*;
    let engine = avenger_text::default_text_engine();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "widgets-gallery.png".into());
    let themes = match std::env::args().nth(2).as_deref() {
        Some("light") => vec![false],
        Some("dark") => vec![true],
        None => vec![false, true],
        _ => return Err("theme must be light or dark".into()),
    };
    let width = themes.len() as f32 * 420.0;
    let mut marks = Vec::new();
    for (col, dark) in themes.into_iter().enumerate() {
        let x = col as f32 * 420.0;
        let mut theme = if dark {
            WidgetTheme::dark()
        } else {
            WidgetTheme::light()
        };
        theme.button.text.font = "Lato".into();
        theme.checkbox.text.font = "Lato".into();
        theme.radio.text.font = "Lato".into();
        theme.group.text.font = "Lato".into();
        theme.slider.text.font = "Lato".into();
        theme.text_input.text.font = "Lato".into();
        let background = if dark {
            [0.06, 0.08, 0.11, 1.0]
        } else {
            [1.0; 4]
        };
        let ink = if dark {
            [0.9, 0.94, 1.0, 1.0]
        } else {
            [0.07, 0.12, 0.19, 1.0]
        };
        marks.push(
            SceneRectMark {
                interactive: false,
                x: x.into(),
                y: 0.0.into(),
                width: Some(420.0.into()),
                height: Some(790.0.into()),
                fill: ColorOrGradient::Color(background).into(),
                ..Default::default()
            }
            .into(),
        );
        marks.push(
            SceneTextMark {
                interactive: false,
                text: if dark {
                    "Dark theme".into()
                } else {
                    "Light theme".into()
                },
                x: (x + 28.0).into(),
                y: 42.0.into(),
                font: "Lato".to_string().into(),
                font_size: 22.0.into(),
                color: ColorOrGradient::Color(ink).into(),
                ..Default::default()
            }
            .into(),
        );
        let items = vec![
            ChoiceItem::new("a", "Grid lines"),
            ChoiceItem::new("b", "Point marks"),
            ChoiceItem::new("c", "Annotations").enabled(false),
        ];
        let specs: Vec<WidgetSpec> = vec![
            Button::new("apply", "Apply style")
                .variant(ButtonVariant::Accent)
                .into(),
            Button::new("disabled", "Disabled action")
                .enabled(false)
                .into(),
            Checkbox::new("lock", "Lock styling", true).into(),
            CheckboxGroup::new("layers", items, ["a".into(), "c".into()])
                .label("Visible layers")
                .into(),
            RadioGroup::new(
                "palette",
                vec![
                    ChoiceItem::new("o", "Ocean"),
                    ChoiceItem::new("s", "Sunset"),
                    ChoiceItem::new("m", "Mono"),
                ],
                Some("o".into()),
            )
            .label("Palette")
            .orientation(ChoiceOrientation::Horizontal)
            .into(),
            Slider::new("size", SliderDomain::stepped(0.0, 10.0, 3.0)?, 6.0)
                .value_label("6 px")
                .into(),
            Slider::new("opacity", SliderDomain::continuous(0.0, 1.0)?, 0.72)
                .value_label("72%")
                .into(),
            TextInput::new("source", "*Radius* $sqrt(x^2+y^2)$")
                .semantic_name("Annotation source")
                .into(),
            TextInput::new("readonly", "Read-only · copy this value")
                .read_only(true)
                .into(),
            TextInput::new(
                "narrow",
                "A narrow allocation clips and scrolls this source",
            )
            .into(),
            TextInput::new("invalid", "$sqrt(x")
                .semantic_name("Invalid source")
                .invalid(true)
                .into(),
        ];
        let mut runtime = WidgetRuntime::new();
        for pass in 0..2 {
            let mut prepared = runtime.prepare(&specs, &theme, &engine)?;
            let mut y = 68.0;
            for spec in &specs {
                let size = prepared.metrics(spec.id().clone()).unwrap().preferred;
                prepared.place(
                    spec.id().clone(),
                    Rect::new(
                        x + 28.0,
                        y,
                        if spec.id().as_str() == "narrow" {
                            180.0
                        } else {
                            364.0
                        },
                        size.height,
                    ),
                    None,
                )?;
                y += size.height + 16.0;
            }
            let frame = prepared.finish()?;
            if pass == 1 {
                marks.push(frame.scene.clone().into());
            }
            runtime.install(frame)?;
            if pass == 0 {
                if dark {
                    runtime.request_focus(Some(WidgetTarget::new("source")), Instant::now())?;
                    runtime.set_text_selection(
                        "source",
                        avenger_text::text_edit::SelectionState {
                            anchor: avenger_text::text_edit::Cursor::new(
                                1,
                                avenger_text::text_edit::Affinity::Downstream,
                            ),
                            head: avenger_text::text_edit::Cursor::new(
                                7,
                                avenger_text::text_edit::Affinity::Upstream,
                            ),
                            granularity: avenger_text::text_edit::Granularity::Char,
                        },
                        Instant::now(),
                    )?;
                } else {
                    runtime
                        .request_focus(Some(WidgetTarget::item("layers", "a")), Instant::now())?;
                }
            }
        }
    }
    let scene = SceneGraph {
        marks,
        width,
        height: 790.0,
        origin: [0.0; 2],
    };
    match std::path::Path::new(&path)
        .extension()
        .and_then(|s| s.to_str())
    {
        Some("svg") => {
            std::fs::write(
                &path,
                avenger_svg::SvgRenderer::new()
                    .with_text_engine(engine)
                    .render_scene_graph(&scene)?,
            )?;
            return Ok(());
        }
        Some("pdf") => {
            std::fs::write(
                &path,
                avenger_pdf::PdfRenderer::new()
                    .with_text_engine(engine)
                    .render_scene_graph(&scene)?,
            )?;
            return Ok(());
        }
        _ => {}
    }
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [width, 790.0],
            scale: 1.0,
        },
        CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        },
    ))?;
    canvas.set_scene(&scene)?;
    let image = pollster::block_on(canvas.render())?;
    image.save(&path)?;
    println!("{path}");
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
