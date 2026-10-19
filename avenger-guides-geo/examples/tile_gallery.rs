//! Draw OpenStreetMap raster tiles on three projections, a globe, a world map turned so that its
//! antimeridian cuts through tiles, and a framed conic of the Alps, each with the tiles' credit
//! in its corner, and export them with `avenger-svg`. The tiles come from the render fixtures,
//! which hold zooms 0 to 2 and the Alps at zoom 5. Run with an optional output directory
//! (default: target/geo-gallery).

use std::error::Error;
use std::path::PathBuf;

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, TextBaseline};
use avenger_geo::projections::{ConicConformal, EqualEarth, Orthographic};
use avenger_geo::tiles::visible_tiles;
use avenger_geo::{Projection, Projector};
use avenger_guides::attribution::{Attribution, AttributionConfig};
use avenger_guides::BackgroundStyle;
use avenger_guides_geo::{make_sphere_marks, SphereStyle, TileLayer};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, text::SceneTextMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine};

const PANEL: f32 = 360.0;
/// The panel's side in output units, for projections.
const SIDE: f64 = PANEL as f64;
const GAP: f32 = 24.0;
const TOP: f32 = 36.0;
const INK: [f32; 4] = [0.09, 0.17, 0.26, 1.0];
/// OpenStreetMap's water, which fills the sphere above the tiles' ±85.05° of latitude.
const WATER: [f32; 4] = [170.0 / 255.0, 211.0 / 255.0, 223.0 / 255.0, 1.0];

/// A map: its title, its projection fitted to its rectangle, the tiles' zoom, and its size.
struct Panel {
    title: &'static str,
    projector: Projector,
    zoom: u8,
    size: [f32; 2],
}

fn panels() -> Result<Vec<Panel>, Box<dyn Error>> {
    let mut world = Projection::new(EqualEarth).with_rotate([150.0, 0.0, 0.0]);
    let summary = world.summarize_sphere()?;
    world.fit([[1.0, 1.0], [SIDE - 1.0, SIDE / 2.0 - 1.0]], &summary)?;
    Ok(vec![
        Panel {
            title: "Orthographic, zoom 2",
            projector: Projection::new(Orthographic)
                .with_rotate([-30.0, -35.0, 0.0])
                .with_view(SIDE / 2.0 - 1.0, [SIDE / 2.0, SIDE / 2.0])
                .build()?,
            zoom: 2,
            size: [PANEL, PANEL],
        },
        Panel {
            title: "Equal Earth turned 150°, zoom 2",
            projector: world.build()?,
            zoom: 2,
            size: [PANEL, PANEL / 2.0],
        },
        Panel {
            title: "Conformal conic, zoom 5",
            projector: Projection::new(ConicConformal::new([40.0, 52.0]))
                .with_rotate([-12.0, 0.0, 0.0])
                .with_center(0.0, 48.0)
                .with_view(1030.0, [SIDE / 2.0, 120.0])
                .with_clip_extent(Some([[0.0, 0.0], [SIDE, 240.0]]))
                .build()?,
            zoom: 5,
            size: [PANEL, 240.0],
        },
    ])
}

/// A layer of the tiles that `projector` shows at `zoom`. An application fetches and caches them
/// from its tile server and keeps one layer from frame to frame; this example reads them from
/// the render fixtures.
fn tiles(projector: &Projector, zoom: u8) -> Result<TileLayer, Box<dyn Error>> {
    let tiles = avenger_render_fixtures::tiles::osm_tiles(&visible_tiles(projector, zoom)?);
    Ok(tiles.into_iter().collect())
}

fn gallery(engine: &LabelEngine) -> Result<SceneGraph, Box<dyn Error>> {
    let credits = Attribution::new(
        &["© OpenStreetMap contributors".to_string()],
        &AttributionConfig {
            font_family: "Lato".to_string(),
            font_size: 9.0,
            background: BackgroundStyle {
                fill: ColorOrGradient::Color([1.0, 1.0, 1.0, 0.75]),
                padding: 3.0,
                ..Default::default()
            },
            ..Default::default()
        },
        engine,
    )?;
    let mut marks: Vec<SceneMark> = Vec::new();
    let mut x = GAP;
    for panel in panels()? {
        let projector = &panel.projector;
        let y = TOP + (PANEL - panel.size[1]) / 2.0;
        marks.push(
            SceneTextMark {
                text: panel.title.to_string().into(),
                x: x.into(),
                y: (TOP - 12.0).into(),
                baseline: TextBaseline::Alphabetic.into(),
                font: "Lato".to_string().into(),
                font_size: 13.0.into(),
                font_weight: FontWeight::BOLD.into(),
                color: INK.into(),
                ..Default::default()
            }
            .into(),
        );
        let fill = SphereStyle {
            fill: WATER,
            stroke: [0.0; 4],
            stroke_width: 0.0,
        };
        let outline = SphereStyle {
            fill: [0.0; 4],
            ..Default::default()
        };
        marks.push(
            SceneGroup {
                origin: [x, y],
                marks: vec![
                    make_sphere_marks(projector, &fill).into(),
                    tiles(projector, panel.zoom)?
                        .group(projector, panel.zoom)?
                        .into(),
                    make_sphere_marks(projector, &outline).into(),
                    credits.marks(panel.size).into(),
                ],
                ..Default::default()
            }
            .into(),
        );
        x += PANEL + GAP;
    }
    Ok(SceneGraph {
        width: x,
        height: TOP + PANEL + GAP,
        origin: [0.0, 0.0],
        marks,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/geo-gallery"));
    std::fs::create_dir_all(&output)?;
    // The bundled fonts, without the system's, keep the images the same on every machine.
    let engine = LabelEngine::new(EngineOptions {
        fonts: FontOptions {
            load_system_fonts: false,
            ..avenger_typst_label::bundled_font_options()
        },
    });
    let scene = gallery(&engine)?;
    let svg = avenger_svg::SvgRenderer::new().render_scene_graph(&scene, &engine)?;
    std::fs::write(output.join("tiles.svg"), &svg)?;
    avenger_render_fixtures::raster::svg_to_png(&svg, 2.0).save(output.join("tiles.png"))?;
    println!("Wrote tiles.svg and tiles.png to {}", output.display());
    Ok(())
}
