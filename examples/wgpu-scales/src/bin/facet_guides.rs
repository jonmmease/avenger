//! Render a nested facet grid that facet guides label, placed with avenger-layout: the islands
//! above, the species on each island under them, and sex beside the rows. The species guides of
//! an island's rows are equal, so they collapse to the top row, and the sex guides beside the
//! islands collapse to the last island.
use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray};
use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_guides::{
    facet::{FacetGuide, FacetGuideConfig, FacetGuidePart, FacetGuideStyle},
    GuideSide,
};
use avenger_layout::{
    Column, Edges, Gap, Layout, Leaf, Rect, RegionKind, Row, Side, Size, Slab, Solution,
    SolveOptions,
};
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark},
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

const PANEL: Size = Size::new(150.0, 110.0);
/// The space between the panels of one island.
const PANEL_GAP: f32 = 12.0;
const ISLAND_GAP: f32 = 28.0;
const ISLANDS: [(&str, &[&str]); 3] = [
    ("Biscoe", &["Adelie", "Gentoo"]),
    ("Dream", &["Adelie", "Chinstrap"]),
    ("Torgersen", &["Adelie"]),
];
const SEXES: [&str; 2] = ["Female", "Male"];

/// The regions that the layout places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Panel {
        island: usize,
        sex: usize,
        species: usize,
    },
    /// The slab of one part of a guide.
    Guide(Guide, FacetGuidePart),
}

/// Where a facet guide stands: above the islands, above each row of an island's species, or
/// beside an island's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Guide {
    Islands,
    Species { island: usize, sex: usize },
    Sexes { island: usize },
}

struct Guides {
    islands: FacetGuide,
    /// Each island's species.
    species: Vec<FacetGuide>,
    sexes: FacetGuide,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "facet-guides.png".into());
    let text_engine = avenger_typst_label::bundled_label_engine();
    let guide = |values: &[&str], title: &str, config: FacetGuideConfig| {
        let values: ArrayRef = Arc::new(StringArray::from(values.to_vec()));
        FacetGuide::new(&values, title, &config, &text_engine)
    };
    // Labels are a little smaller and lighter than the titles.
    let style = FacetGuideStyle {
        label_color: [0.35, 0.35, 0.35, 1.0],
        label_font_size: 11.0,
        ..Default::default()
    };
    let side = |side| FacetGuideConfig {
        side,
        style: style.clone(),
        ..Default::default()
    };
    let islands: Vec<&str> = ISLANDS.iter().map(|(island, _)| *island).collect();
    // The islands' labels stand farther out than the default, past the species guides' titles,
    // so that the two levels read apart.
    let outer = FacetGuideConfig {
        style: FacetGuideStyle {
            label_padding: 14.0,
            ..style.clone()
        },
        ..side(GuideSide::Top)
    };
    let guides = Guides {
        islands: guide(&islands, "Island", outer)?,
        species: ISLANDS
            .iter()
            .map(|(_, species)| guide(species, "Species", side(GuideSide::Top)))
            .collect::<Result<_, _>>()?,
        sexes: guide(&SEXES, "Sex", side(GuideSide::Right))?,
    };

    let options = SolveOptions {
        padding: Edges::all(24.0),
        ..Default::default()
    };
    let solution = page(&guides).solve(&options)?;
    let size = solution.size();
    let scene = SceneGraph {
        width: size.width,
        height: size.height,
        origin: [0.0, 0.0],
        marks: draw(&solution, &guides),
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(&scene, &text_engine)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}

/// The page: a row of islands, each a column of rows of its species' panels. The islands' guide
/// stands above the row, each row of panels has a species guide above it, and each island has a
/// sex guide beside it. The row collapses each part of equal guides along each lane, and matches
/// the labels' thickness, so the titles beyond guides with and without a rule line up.
fn page(guides: &Guides) -> Layout<Id> {
    let islands = ISLANDS.iter().enumerate().map(|(island, (_, species))| {
        let rows = (0..SEXES.len()).map(|sex| {
            let panels = (0..species.len()).map(|species| {
                Leaf::new().min_size(PANEL).id(Id::Panel {
                    island,
                    sex,
                    species,
                })
            });
            let [labels, title] = guide_slabs(
                &guides.species[island],
                Guide::Species { island, sex },
                Side::Top,
            );
            Row::new(panels)
                .gap(Gap::new(PANEL_GAP))
                .slab(Side::Top, labels)
                .slab(Side::Top, title)
        });
        let [labels, title] = guide_slabs(&guides.sexes, Guide::Sexes { island }, Side::Right);
        Column::new(rows)
            .gap(Gap::new(PANEL_GAP))
            .slab(Side::Right, labels)
            .slab(Side::Right, title)
    });
    let [labels, title] = guide_slabs(&guides.islands, Guide::Islands, Side::Top);
    let mut page = Row::new(islands)
        .gap(Gap::new(ISLAND_GAP))
        .slab(Side::Top, labels)
        .slab(Side::Top, title);
    for name in ["species", "sexes"] {
        page = page
            .collapse_tag(tag(name, FacetGuidePart::Labels))
            .collapse_tag(tag(name, FacetGuidePart::Title))
            .match_tag(tag(name, FacetGuidePart::Labels));
    }
    page.into()
}

/// A guide's labels and title slabs on `side`, keyed by what each part draws.
fn guide_slabs(guide: &FacetGuide, place: Guide, side: Side) -> [Slab<Id>; 2] {
    let name = match place {
        Guide::Islands => "islands",
        Guide::Species { .. } => "species",
        Guide::Sexes { .. } => "sexes",
    };
    [FacetGuidePart::Labels, FacetGuidePart::Title].map(|part| {
        // Each part is as thick at every length.
        let thickness = guide.extent(part, 0.0).thickness;
        let slab = Slab::new()
            .id(Id::Guide(place, part))
            .tag(tag(name, part))
            .content_key(guide.content_key(part));
        match side {
            Side::Left | Side::Right => slab.min_width(thickness),
            Side::Top | Side::Bottom => slab.min_height(thickness),
        }
    })
}

fn tag(name: &str, part: FacetGuidePart) -> String {
    match part {
        FacetGuidePart::Labels => format!("{name}-labels"),
        FacetGuidePart::Title => format!("{name}-title"),
    }
}

/// The panels, and each guide part in the slab that stands for it.
fn draw(solution: &Solution<Id>, guides: &Guides) -> Vec<SceneMark> {
    let panel = |island, sex, species| {
        solution
            .region(&Id::Panel {
                island,
                sex,
                species,
            })
            .expect("every panel has a region")
            .content
    };
    let mut marks = Vec::new();
    for region in solution.regions() {
        let rect = region.content;
        match (region.id, &region.kind) {
            (
                Some(Id::Panel {
                    island,
                    sex,
                    species,
                }),
                _,
            ) => marks.push(panel_marks(rect, (island * 2 + sex) * 2 + species).into()),
            // A slab that another slab stands for, in a collapsed run, draws nothing.
            (
                Some(Id::Guide(place, part)),
                RegionKind::Slab {
                    represented_by: None,
                    ..
                },
            ) => {
                let guide = match place {
                    Guide::Islands => &guides.islands,
                    Guide::Species { island, .. } => &guides.species[island],
                    Guide::Sexes { .. } => &guides.sexes,
                };
                let group = match part {
                    FacetGuidePart::Labels => {
                        // Each panel's start and end along the slab, from the slab's start.
                        let bands: Vec<[f32; 2]> = match place {
                            Guide::Islands => (0..ISLANDS.len())
                                .map(|island| {
                                    let last = ISLANDS[island].1.len() - 1;
                                    let (first, last) =
                                        (panel(island, 0, 0), panel(island, 0, last));
                                    [first.x - rect.x, last.x + last.width - rect.x]
                                })
                                .collect(),
                            Guide::Species { island, sex } => (0..ISLANDS[island].1.len())
                                .map(|species| {
                                    let panel = panel(island, sex, species);
                                    [panel.x - rect.x, panel.x + panel.width - rect.x]
                                })
                                .collect(),
                            Guide::Sexes { island } => (0..SEXES.len())
                                .map(|sex| {
                                    let panel = panel(island, sex, 0);
                                    [panel.y - rect.y, panel.y + panel.height - rect.y]
                                })
                                .collect(),
                        };
                        guide.labels(&bands)
                    }
                    FacetGuidePart::Title => guide.title(match place {
                        Guide::Sexes { .. } => rect.height,
                        _ => rect.width,
                    }),
                };
                if let Some(mut group) = group {
                    group.origin = guide.origin([rect.x, rect.y, rect.width, rect.height]);
                    marks.push(group.into());
                }
            }
            _ => {}
        }
    }
    marks
}

/// One panel: a light frame with points that differ from panel to panel.
fn panel_marks(rect: Rect, seed: usize) -> SceneGroup {
    let frame = SceneRectMark {
        len: 1,
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(rect.width.into()),
        height: Some(rect.height.into()),
        fill: ColorOrGradient::Color([0.96, 0.96, 0.97, 1.0]).into(),
        stroke: ColorOrGradient::Color([0.75, 0.75, 0.78, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    };
    // Points along a noisy line whose slope and spread depend on the panel.
    let seed = seed as f32;
    let (x, y): (Vec<f32>, Vec<f32>) = (0..24)
        .map(|i| {
            let t = i as f32 / 23.0;
            let noise = ((i as f32 * 12.9898 + seed * 78.233).sin() * 43_758.547).fract() - 0.5;
            let slope = 0.3 + 0.05 * (seed % 4.0);
            let y = (0.8 - slope * t - (0.1 + 0.03 * (seed % 3.0)) * noise).clamp(0.1, 0.9);
            (12.0 + t * (rect.width - 24.0), y * rect.height)
        })
        .unzip();
    let points = SceneSymbolMark {
        len: x.len() as u32,
        x: x.into(),
        y: y.into(),
        size: 28.0.into(),
        fill: ColorOrGradient::Color([0.27, 0.47, 0.67, 0.85]).into(),
        ..Default::default()
    };
    SceneGroup {
        origin: [rect.x, rect.y],
        marks: vec![frame.into(), points.into()],
        ..Default::default()
    }
}
