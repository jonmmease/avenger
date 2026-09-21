use crate::{dataflow::*, definition::*, *};
use crate::{error, Result};
use avenger_common::value::ScalarOrArray;
use avenger_layout::{Layout, SolveOptions};
use avenger_panels::{
    ArrangementSpec, GroupArrangement, GroupId, NodeId, PanelArrangement, PanelId, PanelNode,
    PanelTree,
};
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use dataflow::datafusion::common::ScalarValue;
use prost::Message;

#[derive(Clone)]
pub(crate) struct Context<'a> {
    pub root: &'a DataflowResult,
    pub chain: Vec<(ScopeHandle, PartitionKey, ScopeResult<'a>)>,
    pub interface: DataflowInterface,
}
impl<'a> Context<'a> {
    pub fn table(&self, h: &TableOutput) -> Result<&'a TableSnapshot> {
        let depth = self.interface.table_metadata(h)?.reference.scope.len();
        Ok(if depth == 0 {
            self.root.table(h)?
        } else {
            self.chain[depth - 1].2.table(h)?
        })
    }
    pub fn scalar(&self, h: &ScalarOutput) -> Result<&'a ScalarValue> {
        let depth = self.interface.scalar_metadata(h)?.reference.scope.len();
        Ok(if depth == 0 {
            self.root.scalar(h)?
        } else {
            self.chain[depth - 1].2.scalar(h)?
        })
    }
    fn scope(&self, h: &ScopeHandle) -> Result<ScopeResults<'a>> {
        Ok(match self.chain.last() {
            None => self.root.scope(h)?,
            Some((_, _, r)) => r.scope(h)?,
        })
    }
    fn text(&self, t: &Text) -> Result<String> {
        Ok(match t {
            Text::Literal(s) => s.clone(),
            Text::Scalar(h) => self.scalar(h)?.to_string(),
            Text::Key(name) => {
                let (s, k, _) = self
                    .chain
                    .last()
                    .ok_or_else(|| error("key outside facet"))?;
                k.values()[s.key_schema().index_of(name).map_err(error)?].to_string()
            }
        })
    }
    fn identity(&self, path: &[String]) -> Result<String> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
        for part in path {
            bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
            bytes.extend_from_slice(part.as_bytes());
        }
        bytes.extend_from_slice(&(self.chain.len() as u64).to_le_bytes());
        for (s, k, _) in &self.chain {
            bytes.extend_from_slice(&(s.name().len() as u64).to_le_bytes());
            bytes.extend_from_slice(s.name().as_bytes());
            bytes.extend_from_slice(&(k.values().len() as u64).to_le_bytes());
            for value in k.values() {
                let v: dataflow::protobuf::common::ScalarValue = value.try_into().map_err(error)?;
                v.encode_length_delimited(&mut bytes).map_err(error)?;
            }
        }
        Ok(format!(
            "node:{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }
}
pub(crate) struct PlotInstance<'a> {
    pub plot: &'a Plot,
    pub ctx: Context<'a>,
    pub id: PanelId,
    pub path: Vec<String>,
    pub scales: BTreeMap<String, avenger_scales::scales::ConfiguredScale>,
}
struct GroupInstance {
    name: String,
    id: GroupId,
    arrangement: Arrangement,
    title: Option<String>,
    children: Vec<Expanded>,
}
enum Expanded {
    Plot(usize),
    Group(Box<GroupInstance>),
}
fn expand<'a>(
    g: &'a Group,
    ctx: Context<'a>,
    path: Vec<String>,
    plots: &mut Vec<PlotInstance<'a>>,
) -> Result<GroupInstance> {
    let mut children = vec![];
    for n in &g.children {
        let mut child_path = path.clone();
        child_path.push(n.name().to_owned());
        children.push(match n {
            Node::Plot(p) => {
                let index = plots.len();
                plots.push(PlotInstance {
                    plot: p,
                    id: ctx.identity(&child_path)?.into(),
                    path: child_path,
                    ctx: ctx.clone(),
                    scales: BTreeMap::new(),
                });
                Expanded::Plot(index)
            }
            Node::Group(g) => Expanded::Group(Box::new(expand(g, ctx.clone(), child_path, plots)?)),
            Node::Facet {
                scope,
                arrangement,
                template,
                ..
            } => {
                let mut instances: Vec<_> = ctx.scope(scope)?.iter().collect();
                instances.sort_by(|(a, _), (b, _)| {
                    a.values()
                        .partial_cmp(b.values())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                match &template.key_order {
                    KeyOrder::Descending => instances.reverse(),
                    KeyOrder::Explicit(keys) => instances.sort_by_key(|(k, _)| {
                        keys.iter().position(|v| v == *k).unwrap_or(usize::MAX)
                    }),
                    _ => {}
                }
                let mut facet_children = vec![];
                for (k, r) in instances {
                    let mut local = ctx.clone();
                    local.chain.push((scope.clone(), k.clone(), r));
                    facet_children.push(Expanded::Group(Box::new(expand(
                        template,
                        local,
                        child_path.clone(),
                        plots,
                    )?)));
                }
                Expanded::Group(Box::new(GroupInstance {
                    name: template.name.clone(),
                    id: format!("facet:{}", ctx.identity(&child_path)?).into(),
                    arrangement: arrangement.clone(),
                    title: None,
                    children: facet_children,
                }))
            }
        });
    }
    Ok(GroupInstance {
        name: g.name.clone(),
        id: ctx.identity(&path)?.into(),
        arrangement: g.arrangement.clone(),
        title: g.title.as_ref().map(|t| ctx.text(t)).transpose()?,
        children,
    })
}
impl Expanded {
    fn id(&self, p: &[PlotInstance]) -> NodeId {
        match self {
            Self::Plot(i) => NodeId::Panel(p[*i].id.clone()),
            Self::Group(g) => NodeId::Group(g.id.clone()),
        }
    }
    fn node(&self, p: &[PlotInstance]) -> PanelNode {
        match self {
            Self::Plot(i) => PanelNode::panel(p[*i].id.clone()),
            Self::Group(g) => PanelNode::group(g.id.clone(), g.children.iter().map(|c| c.node(p))),
        }
    }
}
fn arrangement(
    g: &GroupInstance,
    p: &[PlotInstance],
    mut spec: ArrangementSpec,
) -> ArrangementSpec {
    let a = match &g.arrangement.kind {
        ArrangementKind::Row => GroupArrangement::Row,
        ArrangementKind::Column => GroupArrangement::Column,
        ArrangementKind::Wrap(n) => {
            GroupArrangement::Wrap(std::num::NonZeroUsize::new(*n).unwrap())
        }
        ArrangementKind::Grid {
            rows,
            columns,
            slots,
        } => GroupArrangement::Grid {
            shape: avenger_layout::GridShape {
                rows: *rows,
                columns: *columns,
            },
            slots: slots
                .iter()
                .map(|(name, slot)| {
                    let child = g
                        .children
                        .iter()
                        .find(|c| match c {
                            Expanded::Plot(i) => p[*i].plot.name == *name,
                            Expanded::Group(c) => c.name == *name,
                        })
                        .expect("validated grid child");
                    (child.id(p), *slot)
                })
                .collect(),
        },
    };
    spec = spec.group(g.id.clone(), a);
    for c in &g.children {
        if let Expanded::Group(c) = c {
            spec = arrangement(c, p, spec)
        }
    }
    spec
}
fn layout(
    g: &GroupInstance,
    p: &[PlotInstance],
    a: &PanelArrangement,
    edges: &[Edges<f32>],
    guides: Option<&avenger_panels::GuidePlan>,
) -> Layout<NodeId, String> {
    let grid = a.grid(&g.id).expect("arranged group");
    let mut l = if g.children.is_empty() {
        Layout::leaf(Size::new(0.0, 0.0))
    } else {
        Layout::grid(grid.shape().rows, grid.shape().columns)
    };
    for (c, (_, slot)) in g.children.iter().zip(grid.slots()) {
        let child = match c {
            Expanded::Group(g) => layout(g, p, a, edges, guides),
            Expanded::Plot(i) => {
                let mut l = Layout::leaf(p[*i].plot.size).id(NodeId::Panel(p[*i].id.clone()));
                for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
                    l = l.guide(side, *edges[*i].side(side));
                }
                crate::scales::reserve_titles(l, &NodeId::Panel(p[*i].id.clone()), guides)
            }
        };
        l = l.cell_span(
            slot.row,
            slot.column,
            slot.row_span,
            slot.column_span,
            child,
        );
    }
    l = l
        .id(NodeId::Group(g.id.clone()))
        .margin(g.arrangement.margin);
    if !g.children.is_empty() {
        l = l.min_gap(g.arrangement.gap);
    }
    if g.title.is_some() {
        l = l.strip(Side::Top, 26.0);
    }
    l = crate::scales::reserve_titles(l, &NodeId::Group(g.id.clone()), guides);
    if !g.children.is_empty() && g.arrangement.uniform_columns {
        l = l.uniform_columns()
    }
    if !g.children.is_empty() && g.arrangement.uniform_rows {
        l = l.uniform_rows()
    }
    if let Some(key) = g
        .arrangement
        .share
        .as_ref()
        .filter(|_| !g.children.is_empty())
    {
        l = l.share(key.clone());
    }
    if !g.children.is_empty() && !g.arrangement.columns.is_empty() {
        l = l.columns(g.arrangement.columns.clone())
    }
    if !g.children.is_empty() && !g.arrangement.rows.is_empty() {
        l = l.rows(g.arrangement.rows.clone())
    }
    l
}
fn titles(
    g: &GroupInstance,
    solution: &avenger_layout::LayoutSolution<NodeId>,
    out: &mut Vec<SceneMark>,
) {
    if let Some(title) = &g.title {
        let r = solution.region(&NodeId::Group(g.id.clone())).unwrap();
        out.push(SceneMark::Text(
            SceneTextMark {
                name: format!("{}:header", g.id.as_str()),
                text: ScalarOrArray::new_scalar(title.clone()),
                x: ScalarOrArray::new_scalar(r.content.x),
                y: ScalarOrArray::new_scalar(
                    r.slabs
                        .iter()
                        .find(|s| {
                            s.layer == avenger_layout::ChromeLayer::Strip && s.side == Side::Top
                        })
                        .map_or(r.content.y - 10.0, |s| s.rect.y + 18.0),
                ),
                font_size: ScalarOrArray::new_scalar(15.0),
                interactive: false,
                ..Default::default()
            }
            .into(),
        ));
    }
    for c in &g.children {
        if let Expanded::Group(g) = c {
            titles(g, solution, out)
        }
    }
}
pub(crate) fn render(
    chart: &Chart,
    result: &DataflowResult,
    inputs: Inputs,
) -> Result<RenderedChart> {
    let mut plots = vec![];
    let ctx = Context {
        root: result,
        chain: vec![],
        interface: chart.0.interface.clone(),
    };
    let root = expand(chart.definition().root(), ctx, vec![], &mut plots)?;
    let tree = PanelTree::new(
        root.id.clone(),
        root.children.iter().map(|c| c.node(&plots)),
    )
    .map_err(error)?;
    let arranged = tree
        .arrange(&arrangement(&root, &plots, ArrangementSpec::new()))
        .map_err(error)?;
    crate::scales::configure(&mut plots, &tree, &chart.0.text)?;
    let mut edges = crate::scales::measure(&plots, &chart.0.text, None)?;
    let mut solution = layout(&root, &plots, &arranged, &edges, None)
        .solve(&SolveOptions::default())
        .map_err(error)?;
    let mut guides = crate::scales::guides(&plots, &tree, &arranged, &solution)?;
    for iteration in 0..4 {
        let next = crate::scales::measure(&plots, &chart.0.text, Some(&guides))?;
        if next == edges && iteration > 0 {
            break;
        }
        edges = next;
        solution = layout(&root, &plots, &arranged, &edges, Some(&guides))
            .solve(&SolveOptions::default())
            .map_err(error)?;
        guides = crate::scales::guides(&plots, &tree, &arranged, &solution)?;
    }
    let mut scene_marks = vec![];
    titles(&root, &solution, &mut scene_marks);
    let mut frames = vec![];
    let mut geometry = GeometryReport::default();
    let mut alive = std::collections::HashSet::new();
    for p in &plots {
        let rect = solution
            .region(&NodeId::Panel(p.id.clone()))
            .unwrap()
            .content;
        let mut marks = crate::marks::build(chart, p, &mut geometry, &mut alive)?;
        let group = SceneGroup {
            name: p.id.as_str().to_owned(),
            origin: [rect.x, rect.y],
            clip: if p.plot.clip {
                avenger_scenegraph::marks::group::Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: rect.width,
                    height: rect.height,
                }
            } else {
                Default::default()
            },
            marks: std::mem::take(&mut marks),
            ..Default::default()
        };
        scene_marks.push(SceneMark::Group(group));
        scene_marks.extend(
            crate::scales::axes(p, &chart.0.text, Some(&guides))?
                .into_iter()
                .map(|mut g| {
                    g.origin[0] += rect.x;
                    g.origin[1] += rect.y;
                    SceneMark::Group(g)
                }),
        );
        frames.push(RenderedPlot {
            path: p.path.clone(),
            instance: p.ctx.chain.last().map(|(_, _, r)| r.instance().clone()),
            panel: p.id.clone(),
            rect,
            scales: p.scales.clone(),
        });
    }
    chart
        .0
        .positions
        .lock()
        .map_err(error)?
        .retain(|k, _| alive.contains(k));
    scene_marks.extend(crate::scales::shared_titles(&plots, &guides, &solution));
    let size = solution.size;
    Ok(RenderedChart {
        scene: Arc::new(SceneGraph {
            marks: vec![SceneMark::Group(SceneGroup {
                marks: scene_marks,
                ..Default::default()
            })],
            width: size.width,
            height: size.height,
            origin: [0.0, 0.0],
        }),
        text: chart.0.text.clone(),
        inputs,
        plots: frames,
        report: result.report().clone(),
        geometry,
        elapsed: std::time::Duration::ZERO,
    })
}
