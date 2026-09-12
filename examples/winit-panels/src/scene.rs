use crate::state::State;
use avenger_color::ColorOrGradient;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::{
    axis::{
        numeric::make_numeric_axis_marks_with_text_engine,
        opts::{AxisConfig, AxisOrientation},
    },
    legend::symbol::{SymbolLegendConfig, make_symbol_legend_with_text_engine},
};
use avenger_layout::{ChromeLayer, Edges, Layout, LayoutSolution, Size, SolveFor, SolveOptions};
use avenger_panels::*;
use avenger_scales::scales::{ConfiguredScale, linear::LinearScale};
use avenger_scenegraph::{
    marks::{
        group::SceneGroup, line::SceneLineMark, mark::SceneMark, rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_text::{TextEngine, types::FontWeight};
use std::{collections::BTreeMap, num::NonZeroUsize};

const INK: [f32; 4] = [0.10, 0.17, 0.23, 1.0];
const MUTED: [f32; 4] = [0.36, 0.43, 0.49, 1.0];
const BLUE: [f32; 4] = [0.08, 0.45, 0.66, 1.0];
const ORANGE: [f32; 4] = [0.89, 0.40, 0.18, 1.0];
const IDS: [&str; 6] = [
    "north-a", "north-b", "north-c", "south-a", "south-b", "south-c",
];
const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];
const CHART_ORIGIN: [f32; 2] = [24.0, 126.0];
const GAP: f32 = 10.0;

/// Rendered scene and the public plans used to construct it.
pub struct Output {
    pub scene: SceneGraph,
    pub plan: GuidePlan,
    pub frames: PanelFrames,
    pub domains: BTreeMap<PanelId, (f32, f32)>,
    pub iterations: usize,
    pub fallback: bool,
    pub columns: usize,
}

fn text(value: impl Into<String>, x: f32, y: f32, size: f32, color: [f32; 4]) -> SceneTextMark {
    SceneTextMark {
        text: value.into().into(),
        x: x.into(),
        y: y.into(),
        font_size: size.into(),
        color: ColorOrGradient::Color(color).into(),
        interactive: false,
        ..Default::default()
    }
}
fn rect(name: &str, r: Rect, fill: [f32; 4], stroke: Option<[f32; 4]>) -> SceneRectMark {
    SceneRectMark {
        name: name.into(),
        x: r.x.into(),
        y: r.y.into(),
        width: Some(r.width.into()),
        height: Some(r.height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(stroke.unwrap_or([0.0; 4])).into(),
        stroke_width: if stroke.is_some() { 1.0 } else { 0.0 }.into(),
        interactive: !name.is_empty(),
        // Numeric guide grids use layer -1; opaque backgrounds must stay below them.
        zindex: (name.is_empty() && stroke.is_none()).then_some(-2),
        ..Default::default()
    }
}
fn hierarchy() -> PanelTree {
    PanelTree::new(
        "figure".into(),
        [
            PanelNode::group("north", IDS[..3].iter().map(|id| PanelNode::panel(*id))),
            PanelNode::group("south", IDS[3..].iter().map(|id| PanelNode::panel(*id))),
        ],
    )
    .expect("fixed hierarchy")
}
fn value(panel: usize, month: usize, channel: usize) -> f32 {
    let level = [30.0, 65.0, 100.0, 95.0, 160.0, 230.0][panel];
    level
        * (0.48
            + month as f32 * 0.032
            + ((month + panel * 2 + channel * 3) as f32 * 0.6).sin() * 0.13)
        * (if channel == 0 { 1.0 } else { 0.68 })
}
fn domains(tree: &PanelTree, state: &State) -> Result<BTreeMap<PanelId, (f32, f32)>, String> {
    let groups = tree
        .group(tree.panels().cloned(), State::scope(state.y_scope))
        .map_err(|e| e.to_string())?;
    let mut result = BTreeMap::new();
    for group in groups.iter() {
        let max = group
            .members()
            .iter()
            .map(|p| {
                IDS.iter()
                    .position(|id| *id == p.as_str())
                    .expect("demo panel")
            })
            .flat_map(|i| (0..12).flat_map(move |m| (0..2).map(move |c| value(i, m, c))))
            .fold(0.0, f32::max);
        let max = if state.preset == 2 {
            1.0
        } else {
            (max / 20.0).ceil() * 20.0
        };
        for panel in group.members() {
            result.insert(panel.clone(), (0.0, max));
        }
    }
    Ok(result)
}
fn y_format(state: &State, panel: &PanelId) -> &'static str {
    if state.preset == 2 {
        if panel.as_str().ends_with('b') {
            ".0%"
        } else {
            "$.2f"
        }
    } else {
        ".0f"
    }
}
fn scales(domain: (f32, f32), size: Size) -> (ConfiguredScale, ConfiguredScale) {
    (
        LinearScale::configured((1.0, 12.0), (0.0, size.width)),
        LinearScale::configured(domain, (size.height, 0.0)),
    )
}
fn label_visible(plan: Option<&GuidePlan>, key: &str, panel: &PanelId) -> bool {
    plan.is_none_or(|p| {
        p.decision(&key.into(), panel)
            .and_then(|d| d.instance())
            .is_some_and(|id| id.anchor() == &NodeId::Panel(panel.clone()))
    })
}
fn axes(
    state: &State,
    panel: &PanelId,
    domain: (f32, f32),
    size: Size,
    plan: Option<&GuidePlan>,
    grid: bool,
) -> Result<Vec<SceneGroup>, String> {
    let (x, y) = scales(domain, size);
    [
        (x, AxisOrientation::Bottom, "x-labels", 6.0, ".0f"),
        (
            y,
            AxisOrientation::Left,
            "y-labels",
            4.0,
            y_format(state, panel),
        ),
    ]
    .into_iter()
    .map(|(scale, orientation, key, count, format)| {
        make_numeric_axis_marks_with_text_engine(
            &scale,
            "",
            [0.0, 0.0],
            &AxisConfig {
                orientation,
                dimensions: [size.width, size.height],
                grid,
                tick_count: Some(count),
                format_number: Some(format.into()),
                title_visible: Some(false),
                labels_visible: Some(label_visible(plan, key, panel)),
                label_font_size: Some(11.0),
                grid_color: Some([0.89, 0.92, 0.94, 1.0]),
                grid_width: Some(1.0),
                domain_color: Some([0.65, 0.71, 0.75, 1.0]),
                tick_color: Some([0.65, 0.71, 0.75, 1.0]),
                label_color: Some(MUTED),
                ..Default::default()
            },
            &state.engine,
        )
        .map_err(|e| e.to_string())
    })
    .collect()
}
fn requests(
    tree: &PanelTree,
    state: &State,
    domains: &BTreeMap<PanelId, (f32, f32)>,
    all: bool,
) -> Vec<GuideRequest> {
    let content = |key: &dyn Fn(&PanelId) -> String| {
        tree.panels()
            .map(|p| GuideContribution::new(p.clone()).equivalent(key(p).into()))
            .collect::<Vec<_>>()
    };
    let visibility = if state.outer && !all {
        LabelVisibility::Outer
    } else {
        LabelVisibility::All
    };
    // Fixed tick counts, linear transforms, and explicit formats make this
    // certificate independent of pixel size. Frames still certify alignment.
    let y = content(&|p| format!("linear/reversed/4/{:?}/{}", domains[p], y_format(state, p)));
    vec![
        AxisLabels::new(
            "x-labels".into(),
            Side::Bottom,
            Scope::Root,
            content(&|_| "linear/1..12/6/.0f".into()),
        )
        .visibility(visibility)
        .into(),
        AxisLabels::new(
            "y-labels".into(),
            Side::Left,
            State::scope(state.y_scope),
            y,
        )
        .visibility(visibility)
        .into(),
        SharedGuide::new(
            "x-title".into(),
            SharedGuideKind::AxisTitle,
            Side::Bottom,
            Scope::Root,
            content(&|_| "Month".into()),
        )
        .into(),
        SharedGuide::new(
            "y-title".into(),
            SharedGuideKind::AxisTitle,
            Side::Left,
            State::scope(state.title_scope),
            content(&|_| {
                if state.preset == 2 {
                    "Reported value"
                } else {
                    "Sales ($ thousands)"
                }
                .into()
            }),
        )
        .into(),
        SharedGuide::new(
            "legend".into(),
            SharedGuideKind::Legend,
            state.legend_side(),
            State::scope(state.legend_scope),
            content(&|_| "Online/blue;Retail/orange".into()),
        )
        .into(),
        SharedGuide::new(
            "region-header".into(),
            SharedGuideKind::Header,
            Side::Top,
            Scope::ancestor(1).expect("parent"),
            content(&|p| p.as_str().split('-').next().unwrap().into()),
        )
        .align(CellAlign::Start)
        .into(),
        SharedGuide::new(
            "product-header".into(),
            SharedGuideKind::Header,
            Side::Top,
            Scope::Panel,
            content(&|p| p.as_str().into()),
        )
        .align(CellAlign::Start)
        .into(),
    ]
}

#[derive(Clone)]
struct Measured {
    group: SceneGroup,
    size: Size,
}
fn measured(mut group: SceneGroup, engine: &TextEngine) -> Measured {
    let bbox = group.bounding_box_with_text_engine(engine);
    let lo = bbox.lower();
    let hi = bbox.upper();
    group.origin[0] -= lo[0];
    group.origin[1] -= lo[1];
    Measured {
        group,
        size: Size::new((hi[0] - lo[0]).ceil(), (hi[1] - lo[1]).ceil()),
    }
}
fn shared_content(instance: &GuideInstance, state: &State) -> Result<Measured, String> {
    let mut group = SceneGroup::default();
    if instance.kind() == GuideKind::Legend {
        group = make_symbol_legend_with_text_engine(
            &SymbolLegendConfig {
                title: Some("Sales channel".into()),
                text: vec!["Online".to_string(), "Retail".to_string()].into(),
                fill: vec![ColorOrGradient::Color(BLUE), ColorOrGradient::Color(ORANGE)].into(),
                size: 64.0.into(),
                inner_width: 0.0,
                inner_height: 0.0,
                outer_margin: 0.0,
                text_padding: 7.0,
                title_font_size: Some(12.0),
                label_font_size: Some(12.0),
                title_color: Some(INK),
                label_color: Some(MUTED),
                ..Default::default()
            },
            &state.engine,
        )
        .map_err(|e| e.to_string())?;
    } else {
        let label = match instance.key().as_str() {
            "x-title" => "Month".to_string(),
            "y-title" => if state.preset == 2 {
                "Reported value"
            } else {
                "Sales ($ thousands)"
            }
            .into(),
            "region-header" => if instance.source_panel().as_str().starts_with("north") {
                "NORTH REGION"
            } else {
                "SOUTH REGION"
            }
            .into(),
            _ => format!(
                "Product {}",
                instance
                    .source_panel()
                    .as_str()
                    .chars()
                    .last()
                    .unwrap()
                    .to_ascii_uppercase()
            ),
        };
        let mut mark = text(
            label,
            0.0,
            0.0,
            if instance.key().as_str() == "region-header" {
                14.0
            } else {
                12.0
            },
            INK,
        );
        if instance.kind() == GuideKind::Header {
            mark.font_weight = FontWeight::Number(600.0).into();
        }
        if matches!(instance.side(), Side::Left | Side::Right) {
            mark.angle = (-90.0).into();
        }
        group.marks.push(mark.into());
    }
    Ok(measured(group, &state.engine))
}
fn normal(size: Size, side: Side) -> f32 {
    if matches!(side, Side::Left | Side::Right) {
        size.width
    } else {
        size.height
    }
}
fn layer(kind: GuideKind) -> ChromeLayer {
    match kind {
        GuideKind::Legend => ChromeLayer::Legend,
        GuideKind::Header => ChromeLayer::Strip,
        _ => ChromeLayer::Guide,
    }
}

struct Measurement {
    boundary: BTreeMap<GroupId, Edges<f32>>,
    axes: BTreeMap<PanelId, Edges<f32>>,
    shared: BTreeMap<GuideInstanceId, Measured>,
}
fn measure(
    state: &State,
    tree: &PanelTree,
    domains: &BTreeMap<PanelId, (f32, f32)>,
    frames: Option<&PanelFrames>,
    plan: Option<&GuidePlan>,
    arrangement: &PanelArrangement,
) -> Result<Measurement, String> {
    let mut result = Measurement {
        boundary: BTreeMap::new(),
        axes: BTreeMap::new(),
        shared: BTreeMap::new(),
    };
    for panel in tree.panels() {
        let mut edges = Edges::default();
        if !(state.missing == 2 && panel.as_str() == IDS[5]) {
            let size = frames
                .and_then(|f| f.rect(&NodeId::Panel(panel.clone())))
                .map(|r| Size::new(r.width, r.height))
                .unwrap_or(Size::new(180.0, 140.0));
            for axis in axes(state, panel, domains[panel], size, plan, false)? {
                let bbox = axis.bounding_box_with_text_engine(&state.engine);
                let lo = bbox.lower();
                let hi = bbox.upper();
                edges = edges.max(Edges::new(
                    (-lo[1]).max(0.0).ceil(),
                    (hi[0] - size.width).max(0.0).ceil(),
                    (hi[1] - size.height).max(0.0).ceil(),
                    (-lo[0]).max(0.0).ceil(),
                ));
            }
        }
        result.axes.insert(panel.clone(), edges);
    }
    if let Some(plan) = plan {
        for instance in plan
            .instances()
            .filter(|i| i.kind() != GuideKind::AxisLabels)
        {
            result
                .shared
                .insert(instance.id().clone(), shared_content(instance, state)?);
        }
    }
    // Equal top clearance aligns product headers even when only one y axis has labels.
    let top = result.axes.values().map(|e| e.top).fold(0.0, f32::max);
    let left = result.axes.values().map(|e| e.left).fold(0.0, f32::max);
    for (panel, edges) in &mut result.axes {
        edges.top = top;
        if label_visible(plan, "y-labels", panel) {
            edges.left = left;
        }
    }
    for node in tree.nodes() {
        let NodeId::Group(group) = node else { continue };
        let grid = arrangement.grid(group).expect("arranged group");
        let mut boundary = Edges::<f32>::default();
        for (child, slot) in grid.slots() {
            let NodeId::Panel(panel) = child else {
                continue;
            };
            for side in SIDES {
                let touches = match side {
                    Side::Top => slot.row == 0,
                    Side::Bottom => slot.row_end() == grid.shape().rows,
                    Side::Left => slot.column == 0,
                    Side::Right => slot.column_end() == grid.shape().columns,
                };
                if !touches {
                    continue;
                }
                let mut total = *result.axes[panel].side(side);
                if let Some(plan) = plan {
                    for i in plan.instances().filter(|i| {
                        i.anchor() == child && i.side() == side && i.kind() != GuideKind::AxisLabels
                    }) {
                        total += normal(result.shared[i.id()].size, side) + GAP;
                    }
                }
                boundary.set_side(side, boundary.side(side).max(total));
            }
        }
        result.boundary.insert(group.clone(), boundary);
    }
    // Contained region boxes absorb their children's exterior guides. Matching
    // these clearances keeps the two region grids aligned at their plot edges.
    let shared_boundary =
        result.boundary[&GroupId::from("north")].max(result.boundary[&GroupId::from("south")]);
    for group in ["north", "south"] {
        result.boundary.insert(group.into(), shared_boundary);
    }
    Ok(result)
}
fn decorate(
    mut node: Layout<NodeId, String>,
    id: &NodeId,
    m: &Measurement,
    plan: Option<&GuidePlan>,
) -> Layout<NodeId, String> {
    for side in SIDES {
        let mut guide = if let NodeId::Panel(p) = id {
            *m.axes[p].side(side)
        } else if let NodeId::Group(g) = id {
            *m.boundary[g].side(side)
        } else {
            0.0
        };
        let mut legend = 0.0;
        let mut strip = 0.0;
        if let Some(plan) = plan {
            for i in plan.instances().filter(|i| {
                i.anchor() == id && i.side() == side && i.kind() != GuideKind::AxisLabels
            }) {
                let demand = normal(m.shared[i.id()].size, side) + GAP;
                match i.kind() {
                    GuideKind::Legend => legend += demand,
                    GuideKind::Header => strip += demand,
                    _ => guide += demand,
                }
            }
        }
        node = node.guide(side, guide).legend(side, legend);
        if strip > 0.0 {
            node = node.strip(side, strip);
        }
    }
    node.id(id.clone())
}
fn layout_node(
    id: &NodeId,
    a: &PanelArrangement,
    m: &Measurement,
    plan: Option<&GuidePlan>,
) -> Layout<NodeId, String> {
    let node = match id {
        NodeId::Panel(_) => {
            // A local title or legend also needs room along its anchor edge.
            let mut minimum = Size::new(70.0, 55.0);
            if let Some(plan) = plan {
                for i in plan
                    .instances()
                    .filter(|i| i.anchor() == id && i.kind() != GuideKind::AxisLabels)
                {
                    let size = m.shared[i.id()].size;
                    if matches!(i.side(), Side::Left | Side::Right) {
                        minimum.height = minimum.height.max(size.height);
                    } else {
                        minimum.width = minimum.width.max(size.width);
                    }
                }
            }
            Layout::grid(1, 1).base_cell_size(minimum)
        }
        NodeId::Group(g) => {
            let grid = a.grid(g).expect("arranged group");
            let shape = grid.shape();
            let mut node = Layout::grid(shape.rows, shape.columns)
                .min_gap(if g.as_str() == "figure" { 26.0 } else { 20.0 })
                .sizing(SolveFor::Content);
            for (child, slot) in grid.slots() {
                node = node.cell_span(
                    slot.row,
                    slot.column,
                    slot.row_span,
                    slot.column_span,
                    layout_node(child, a, m, plan),
                );
            }
            if g.as_str() != "figure" {
                node = node
                    .uniform_columns()
                    .uniform_rows()
                    .share("region-tracks".to_string());
            }
            node
        }
    };
    decorate(node, id, m, plan)
}
fn solve(
    state: &State,
    a: &PanelArrangement,
    m: &Measurement,
    p: Option<&GuidePlan>,
) -> Result<LayoutSolution<NodeId>, String> {
    layout_node(&NodeId::Group(a.tree().root().clone()), a, m, p)
        .margin(12.0)
        .solve(&SolveOptions {
            width: Some((state.size[0] - 320.0).max(360.0)),
            height: Some((state.size[1] - 176.0).max(590.0)),
        })
        .map_err(|e| e.to_string())
}
struct Settled {
    solution: LayoutSolution<NodeId>,
    frames: PanelFrames,
    plan: GuidePlan,
    measurement: Measurement,
    iterations: usize,
    fallback: bool,
}
fn settle(
    state: &State,
    tree: &PanelTree,
    a: &PanelArrangement,
    domains: &BTreeMap<PanelId, (f32, f32)>,
) -> Result<Settled, String> {
    for fallback in [false, true] {
        let mut measurement = measure(state, tree, domains, None, None, a)?;
        let mut solution = solve(state, a, &measurement, None)?;
        let mut previous: Option<GuidePlan> = None;
        let mut history = Vec::new();
        for iteration in 1..=12 {
            let frames = PanelFrames::from_layout(a, &solution).map_err(|e| e.to_string())?;
            let plan = tree
                .plan_guides(
                    &frames,
                    requests(tree, state, domains, fallback),
                    GuideOptions {
                        alignment_tolerance: 0.01,
                    },
                )
                .map_err(|e| e.to_string())?;
            measurement = measure(state, tree, domains, Some(&frames), Some(&plan), a)?;
            let next = solve(state, a, &measurement, Some(&plan))?;
            if previous.as_ref() == Some(&plan) && next.content_delta(&solution) < 0.01 {
                return Ok(Settled {
                    solution,
                    frames,
                    plan,
                    measurement,
                    iterations: iteration,
                    fallback,
                });
            }
            let signature = format!(
                "{plan:?}/{:?}",
                tree.panels()
                    .map(|p| next.region(&NodeId::Panel(p.clone())).unwrap().content)
                    .collect::<Vec<_>>()
            );
            if history.contains(&signature) {
                break;
            }
            history.push(signature);
            previous = Some(plan);
            solution = next;
        }
    }
    Err("Guide measurements did not settle after the bounded layout passes".into())
}

/// Construct the scene and return the actual plans used by the renderer.
pub fn build(state: &State) -> Result<Output, String> {
    let tree = hierarchy();
    let domains = domains(&tree, state)?;
    let wrap =
        GroupArrangement::Wrap(NonZeroUsize::new(state.columns()).expect("positive columns"));
    let mut spec = ArrangementSpec::new()
        .group("figure", GroupArrangement::Column)
        .group("north", wrap.clone())
        .group("south", wrap);
    if state.missing == 2 {
        spec = spec.display(IDS[5], PanelDisplay::Hole);
    }
    let arrangement = tree.arrange(&spec).map_err(|e| e.to_string())?;
    let settled = settle(state, &tree, &arrangement, &domains)?;
    let mut chart = Vec::<SceneMark>::new();
    for (index, panel) in tree.panels().enumerate() {
        let r = settled.frames.rect(&NodeId::Panel(panel.clone())).unwrap();
        if settled.frames.display(panel) == Some(PanelDisplay::Hole) {
            if state.overlay {
                chart.push(rect("", r, [0.98, 0.98, 0.98, 1.0], Some([0.7, 0.7, 0.7, 1.0])).into());
                chart.push(text("hole", r.x + 10.0, r.y + 20.0, 12.0, MUTED).into());
            }
            continue;
        }
        chart.push(rect("", r, [0.98, 0.987, 0.99, 1.0], None).into());
        for mut axis in axes(
            state,
            panel,
            domains[panel],
            Size::new(r.width, r.height),
            Some(&settled.plan),
            true,
        )? {
            axis.origin = [r.x, r.y];
            chart.push(axis.into());
        }
        if state.missing == 0 || index != 5 {
            let (x, y) = scales(domains[panel], Size::new(r.width, r.height));
            for (channel, color) in [BLUE, ORANGE].into_iter().enumerate() {
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                for month in 0..12 {
                    let v = if state.preset == 2 {
                        value(index, month, channel)
                            / [40.0, 80.0, 120.0, 120.0, 200.0, 280.0][index]
                    } else {
                        value(index, month, channel)
                    };
                    xs.push(
                        r.x + x
                            .scale_scalar(&(month as f32 + 1.0))
                            .map_err(|e| e.to_string())?
                            .as_f32()
                            .unwrap(),
                    );
                    ys.push(
                        r.y + y
                            .scale_scalar(&v)
                            .map_err(|e| e.to_string())?
                            .as_f32()
                            .unwrap(),
                    );
                }
                chart.push(
                    SceneLineMark {
                        len: 12,
                        x: xs.into(),
                        y: ys.into(),
                        stroke: ColorOrGradient::Color(color),
                        stroke_width: 2.5,
                        interactive: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }
        } else {
            chart.push(text("No observations", r.x + 14.0, r.y + 28.0, 12.0, MUTED).into());
        }
    }
    let mut offsets: BTreeMap<(NodeId, u8, u8), f32> = BTreeMap::new();
    let mut shared: Vec<_> = settled
        .plan
        .instances()
        .filter(|i| i.kind() != GuideKind::AxisLabels)
        .collect();
    shared.sort_by(|a, b| a.order().cmp(&b.order()).then_with(|| a.key().cmp(b.key())));
    for i in shared {
        let region = settled.solution.region(i.anchor()).unwrap();
        let slab = region
            .slabs
            .iter()
            .find(|s| s.side == i.side() && s.layer == layer(i.kind()))
            .ok_or_else(|| format!("Missing slab for {}", i.key()))?;
        let content = &settled.measurement.shared[i.id()];
        let size = content.size;
        let offset = offsets
            .entry((i.anchor().clone(), i.side() as u8, layer(i.kind()) as u8))
            .or_insert_with(|| {
                if layer(i.kind()) == ChromeLayer::Guide {
                    match i.anchor() {
                        NodeId::Panel(p) => *settled.measurement.axes[p].side(i.side()),
                        NodeId::Group(g) => *settled.measurement.boundary[g].side(i.side()),
                    }
                } else {
                    0.0
                }
            });
        let anchor = region.content;
        let (x, y) = match i.side() {
            Side::Left => (
                slab.rect.x + slab.rect.width - *offset - size.width - GAP,
                anchor.y + i.alignment().offset(anchor.height, size.height),
            ),
            Side::Right => (
                slab.rect.x + *offset + GAP,
                anchor.y + i.alignment().offset(anchor.height, size.height),
            ),
            Side::Top => (
                anchor.x + i.alignment().offset(anchor.width, size.width),
                slab.rect.y + slab.rect.height - *offset - size.height - GAP,
            ),
            Side::Bottom => (
                anchor.x + i.alignment().offset(anchor.width, size.width),
                slab.rect.y + *offset + GAP,
            ),
        };
        *offset += normal(size, i.side()) + GAP;
        let mut group = content.group.clone();
        group.name = format!("guide/{}/{:?}", i.key(), i.anchor());
        group.origin[0] += x;
        group.origin[1] += y;
        chart.push(group.into());
    }
    if state.overlay {
        for (group, color) in [("north", BLUE), ("south", ORANGE)] {
            let r = settled.frames.rect(&GroupId::from(group).into()).unwrap();
            chart.push(rect("", r, [0.0; 4], Some(color)).into());
        }
        for i in settled
            .plan
            .instances()
            .filter(|i| i.kind() == GuideKind::AxisLabels)
        {
            let r = settled.frames.rect(i.anchor()).unwrap();
            let (x, y) = if i.side() == Side::Left {
                (r.x + 5.0, r.y + 17.0)
            } else {
                (r.x + 5.0, r.y + r.height - 8.0)
            };
            chart.push(
                text(
                    format!(
                        "{}: {}",
                        if i.side() == Side::Left { "y" } else { "x" },
                        i.members().len()
                    ),
                    x,
                    y,
                    10.0,
                    BLUE,
                )
                .into(),
            );
        }
    }
    let mut marks: Vec<SceneMark> = vec![
        rect(
            "",
            Rect::new(0.0, 0.0, state.size[0], state.size[1]),
            [1.0; 4],
            None,
        )
        .into(),
    ];
    let mut title = text("One figure. Independent decisions.", 28.0, 47.0, 28.0, INK);
    title.font_weight = FontWeight::Number(600.0).into();
    marks.push(title.into());
    marks.push(
        text(
            "Share domains, simplify axes, and place guides at the group where they belong.",
            28.0,
            77.0,
            14.0,
            MUTED,
        )
        .into(),
    );
    marks.push(
        text(
            format!(
                "PANEL EXPLORER   /   2 regions · 3 products · {} columns per region",
                state.columns()
            ),
            28.0,
            105.0,
            11.0,
            BLUE,
        )
        .into(),
    );
    let mut chart = SceneGroup {
        name: "panels".into(),
        origin: CHART_ORIGIN,
        marks: chart,
        ..Default::default()
    };
    let bounds = chart.bounding_box_with_text_engine(&state.engine);
    if bounds.upper()[0] > state.size[0] - 280.0 || bounds.upper()[1] > state.size[1] - 20.0 {
        chart.name = "needs-room".into();
        chart.marks = vec![
            text("This arrangement needs more room.", 12.0, 40.0, 16.0, INK).into(),
            text(
                "Enlarge the window or share more guides.",
                12.0,
                68.0,
                12.0,
                MUTED,
            )
            .into(),
        ];
    }
    marks.push(chart.into());
    let sx = state.size[0] - 262.0;
    marks.push(
        rect(
            "",
            Rect::new(sx - 14.0, 124.0, 252.0, state.size[1] - 150.0),
            [0.95, 0.965, 0.973, 1.0],
            None,
        )
        .into(),
    );
    for (index, (label, value)) in state.controls().into_iter().enumerate() {
        let y = 145.0 + index as f32 * 65.0;
        marks.push(text(label, sx, y, 11.0, MUTED).into());
        marks.push(
            rect(
                &format!("control-{index}"),
                Rect::new(sx, y + 8.0, 222.0, 32.0),
                [1.0; 4],
                Some([0.81, 0.86, 0.89, 1.0]),
            )
            .into(),
        );
        marks.push(
            text(
                value,
                sx + 9.0,
                y + 29.0,
                if index == 7 { 11.0 } else { 12.0 },
                INK,
            )
            .into(),
        );
    }
    let y = 145.0 + 8.0 * 65.0;
    marks.push(text("Click a control or press 1–8.", sx, y, 11.0, MUTED).into());
    marks.push(text("Resize to rewrap each region.", sx, y + 19.0, 11.0, MUTED).into());
    let label_count = settled
        .plan
        .instances()
        .filter(|i| i.kind() == GuideKind::AxisLabels)
        .count();
    let legend_count = settled
        .plan
        .instances()
        .filter(|i| i.kind() == GuideKind::Legend)
        .count();
    marks.push(
        text(
            format!("{label_count} labeled axes · {legend_count} legends"),
            sx,
            y + 51.0,
            12.0,
            BLUE,
        )
        .into(),
    );
    if state.overlay {
        marks.push(
            text(
                format!(
                    "{} layout passes{}",
                    settled.iterations,
                    if settled.fallback {
                        " · all-label fallback"
                    } else {
                        ""
                    }
                ),
                sx,
                y + 74.0,
                11.0,
                MUTED,
            )
            .into(),
        );
        let group_count = tree
            .group(tree.panels().cloned(), State::scope(state.y_scope))
            .unwrap()
            .iter()
            .count();
        marks.push(
            text(
                format!("{group_count} y-domain groups"),
                sx,
                y + 94.0,
                11.0,
                MUTED,
            )
            .into(),
        );
    }
    let scene = SceneGraph {
        width: state.size[0],
        height: state.size[1],
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                marks,
                ..Default::default()
            }
            .into(),
        ],
    };
    Ok(Output {
        scene,
        plan: settled.plan,
        frames: settled.frames,
        domains,
        iterations: settled.iterations,
        fallback: settled.fallback,
        columns: state.columns(),
    })
}
