use crate::selection::PLOTS;
use anyhow::{Context, Result};
use avenger_layout::{Layout, Size, SolveFor, SolveOptions};
use avenger_panels::*;

pub const SIZE: [f32; 2] = [648., 732.];

pub fn plots() -> Result<Vec<Rect>> {
    let tree = PanelTree::new(
        "figure".into(),
        PLOTS.iter().map(|p| PanelNode::panel(p.name)),
    )?;
    let arrangement =
        tree.arrange(&ArrangementSpec::new().group("figure", GroupArrangement::Column))?;
    // Each panel is 600×200, including space for the axes and their titles.
    let mut layout = Layout::<NodeId, String>::grid(3, 1)
        .id(NodeId::Group("figure".into()))
        .min_gap(12.)
        .sizing(SolveFor::Content);
    for (i, p) in PLOTS.iter().enumerate() {
        layout = layout.cell(
            i,
            0,
            Layout::grid(1, 1)
                .base_cell_size(Size::new(505., 150.))
                .guide(Side::Left, 75.)
                .guide(Side::Right, 20.)
                .guide(Side::Top, 20.)
                .guide(Side::Bottom, 30.)
                .sizing(SolveFor::Content)
                .id(NodeId::Panel(p.name.into())),
        );
    }
    let solution = layout.solve(&SolveOptions::default())?;
    let frames = PanelFrames::from_layout(&arrangement, &solution)?;
    PLOTS
        .iter()
        .map(|p| {
            let r = frames
                .rect(&NodeId::Panel(p.name.into()))
                .context("histogram frame")?;
            Ok(Rect::new(r.x + 24., r.y + 60., r.width, r.height))
        })
        .collect()
}
