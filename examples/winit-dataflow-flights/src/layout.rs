use crate::dataflow::Metadata;
use anyhow::{Context, Result};
use avenger_layout::{Layout, Size, SolveFor, SolveOptions};
use avenger_panels::*;
use std::{collections::BTreeMap, num::NonZeroUsize};

#[derive(Clone)]
pub struct DashboardLayout {
    pub size: [f32; 2],
    pub scatter: Rect,
    pub airline: Rect,
    pub panels: BTreeMap<String, Rect>,
    pub frames: PanelFrames,
    pub guides: GuidePlan,
}
impl DashboardLayout {
    pub fn solve(metadata: &Metadata, size: [f32; 2]) -> Result<Self> {
        let columns = if size[0] >= 1100. {
            3
        } else if size[0] >= 650. {
            2
        } else {
            1
        };
        let tree = PanelTree::new(
            "figure".into(),
            [
                PanelNode::group(
                    "main",
                    [PanelNode::panel("scatter"), PanelNode::panel("airline")],
                ),
                PanelNode::group(
                    "destinations",
                    metadata
                        .destinations
                        .iter()
                        .map(|d| PanelNode::panel(d.clone())),
                ),
            ],
        )?;
        let stacked = size[0] < 950. && size[1] >= 1300.;
        let rows = metadata.destinations.len().div_ceil(columns);
        let chrome = if stacked { 176. } else { 77. }
            + rows as f32 * 77.
            + rows.saturating_sub(1) as f32 * 22.
            + 30.;
        let content_budget = (size[1] - 170. - chrome).max(100.);
        let main_height = content_budget * 0.64;
        let panel_height = content_budget * 0.36 / rows.max(1) as f32;
        let arrangement = tree.arrange(
            &ArrangementSpec::new()
                .group("figure", GroupArrangement::Column)
                .group(
                    "main",
                    if stacked {
                        GroupArrangement::Column
                    } else {
                        GroupArrangement::Row
                    },
                )
                .group(
                    "destinations",
                    GroupArrangement::Wrap(NonZeroUsize::new(columns).unwrap()),
                ),
        )?;
        fn node(
            id: &NodeId,
            a: &PanelArrangement,
            size: [f32; 2],
            stacked: bool,
            main_height: f32,
            panel_height: f32,
        ) -> Layout<NodeId, String> {
            match id {
                NodeId::Panel(p) => {
                    let (width, height) = match p.as_str() {
                        "scatter" => (
                            if stacked {
                                size[0] - 145.
                            } else {
                                (size[0] - 190.) * 0.62
                            },
                            if stacked {
                                main_height * 0.45
                            } else {
                                main_height
                            },
                        ),
                        "airline" => (
                            if stacked {
                                size[0] - 145.
                            } else {
                                (size[0] - 190.) * 0.38
                            },
                            if stacked {
                                main_height * 0.55
                            } else {
                                main_height
                            },
                        ),
                        _ => (120., panel_height),
                    };
                    Layout::grid(1, 1)
                        .base_cell_size(Size::new(width, height))
                        .guide(Side::Left, if p.as_str() == "airline" { 34. } else { 52. })
                        .guide(Side::Bottom, 35.)
                        .guide(Side::Right, 12.)
                        .strip(Side::Top, if p.as_str() == "scatter" { 36. } else { 42. })
                        .sizing(SolveFor::Content)
                        .id(id.clone())
                }
                NodeId::Group(g) => {
                    let grid = a.grid(g).unwrap();
                    let shape = grid.shape();
                    let mut n = Layout::grid(shape.rows, shape.columns)
                        .min_gap(if g.as_str() == "figure" { 30. } else { 22. })
                        .sizing(SolveFor::Content)
                        .id(id.clone());
                    if g.as_str() == "destinations" {
                        n = n.uniform_columns().uniform_rows();
                    }
                    for (child, slot) in grid.slots() {
                        n = n.cell_span(
                            slot.row,
                            slot.column,
                            slot.row_span,
                            slot.column_span,
                            node(child, a, size, stacked, main_height, panel_height),
                        );
                    }
                    n
                }
            }
        }
        let root = NodeId::Group("figure".into());
        let solution = node(
            &root,
            &arrangement,
            size,
            stacked,
            main_height,
            panel_height,
        )
        .solve(&SolveOptions {
            width: Some(size[0] - 48.),
            height: Some(size[1] - 170.),
        })?;
        let local = PanelFrames::from_layout(&arrangement, &solution)?;
        let frames = PanelFrames::new(
            &tree,
            tree.nodes().map(|id| {
                let r = local.rect(id).unwrap();
                (
                    id.clone(),
                    Rect::new(r.x + 24., r.y + 115., r.width, r.height),
                )
            }),
            [],
        )?;
        let guides = tree.plan_guides(
            &frames,
            [AxisLabels::new(
                "departure-labels".into(),
                Side::Bottom,
                Scope::ancestor(1)?,
                metadata.destinations.iter().map(|d| {
                    GuideContribution::new(d.clone().into())
                        .equivalent("scheduled-hour-0-24".into())
                }),
            )
            .visibility(LabelVisibility::Outer)
            .into()],
            GuideOptions::default(),
        )?;
        let scatter = frames
            .rect(&NodeId::Panel("scatter".into()))
            .context("scatter frame")?;
        let airline = frames
            .rect(&NodeId::Panel("airline".into()))
            .context("airline frame")?;
        let panels = metadata
            .destinations
            .iter()
            .map(|d| {
                (
                    d.clone(),
                    frames.rect(&NodeId::Panel(d.clone().into())).unwrap(),
                )
            })
            .collect();
        Ok(Self {
            size,
            scatter,
            airline,
            panels,
            frames,
            guides,
        })
    }
    pub fn panel_size(&self) -> [f32; 2] {
        self.panels
            .values()
            .next()
            .map(|r| [r.width, r.height])
            .unwrap_or([100., 100.])
    }
}
pub fn contains(r: Rect, p: [f32; 2]) -> bool {
    p[0] >= r.x && p[0] <= r.x + r.width && p[1] >= r.y && p[1] <= r.y + r.height
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plot_frames_stay_inside_supported_viewports() -> Result<()> {
        let metadata = Metadata {
            source_rows: 1,
            eligible_rows: 1,
            carriers: vec!["AA".into()],
            destinations: ["ATL", "BOS", "CLT", "LAX", "MCO", "ORD"]
                .map(str::to_owned)
                .to_vec(),
            domains: [[-10., 100.], [-10., 100.]],
        };
        for size in [[1280., 940.], [1100., 1000.], [900., 1150.], [720., 780.]] {
            let layout = DashboardLayout::solve(&metadata, size)?;
            for r in [&layout.scatter, &layout.airline]
                .into_iter()
                .chain(layout.panels.values())
            {
                assert!(r.width > 20. && r.height > 20., "{size:?}: {r:?}");
                assert!(
                    r.x >= 24.
                        && r.y >= 115.
                        && r.x + r.width <= size[0] - 12.
                        && r.y + r.height + 40. <= size[1] - 25.,
                    "{size:?}: {r:?}"
                );
            }
        }
        Ok(())
    }
}
