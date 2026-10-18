//! Attributions under each panel of a wrap, which merge into one when every panel credits the
//! same sources.

use std::sync::Arc;

use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::attribution::{Attribution, AttributionConfig};
use avenger_layout::{
    Layout, Leaf, Measure, Measurement, Rect, RegionKind, Side, Size, Slab, Solution, SolveOptions,
    Wrap,
};
use avenger_typst_label::LabelEngine;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Panel(usize),
    Credits(usize),
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

fn attribution(credits: &[&str], engine: &LabelEngine) -> Attribution {
    let credits: Vec<String> = credits.iter().map(|credit| credit.to_string()).collect();
    Attribution::new(&credits, &AttributionConfig::default(), engine).unwrap()
}

/// Each panel's credits answer their thickness at their slab's length.
struct Credits<'a>(&'a [Attribution]);

impl Measure<Id> for Credits<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let Id::Credits(index) = id else {
            return None;
        };
        let extent = self.0[*index].extent(width);
        Some(Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang))
    }
}

/// Four panels in two columns, each with its credits below it, which the wrap merges.
fn solve(attributions: &[Attribution]) -> Solution<Id> {
    let panels = attributions.iter().enumerate().map(|(index, attribution)| {
        Leaf::new()
            .id(Id::Panel(index))
            .min_size(Size::new(180.0, 100.0))
            .slab(
                Side::Bottom,
                Slab::new()
                    .id(Id::Credits(index))
                    .tag("credits")
                    .content_key(attribution.content_key()),
            )
    });
    let layout: Layout<Id> = Wrap::new(panels)
        .min_column(180.0)
        .max_columns(2)
        .merge_tag("credits")
        .into();
    layout
        .measure(
            &SolveOptions::sized(400.0, 400.0),
            &mut Credits(attributions),
        )
        .unwrap()
        .into_solution()
}

/// The credits slabs that draw: those that no other slab represents.
fn drawn(solution: &Solution<Id>) -> Vec<(Id, Rect)> {
    solution
        .regions()
        .filter_map(|region| match (region.id, &region.kind) {
            (
                Some(id @ Id::Credits(_)),
                RegionKind::Slab {
                    represented_by: None,
                    ..
                },
            ) => Some((id, region.content)),
            _ => None,
        })
        .collect()
}

const SOURCES: [&str; 2] = [
    "Countries: Natural Earth",
    "Earthquakes: U.S. Geological Survey",
];

#[test]
fn panels_that_credit_the_same_sources_share_one_attribution() {
    let engine = engine();
    let attributions: Vec<Attribution> = (0..4).map(|_| attribution(&SOURCES, &engine)).collect();
    let solution = solve(&attributions);
    let [(id, rect)] = drawn(&solution)[..] else {
        panic!("one attribution: {:?}", drawn(&solution));
    };
    // The merged slab takes the first panel's id, spans both columns, and is as thick as the
    // credits at that length.
    assert_eq!(id, Id::Credits(0));
    let left = solution.region(&Id::Panel(0)).unwrap().content;
    let right = solution.region(&Id::Panel(1)).unwrap().content;
    assert!((rect.x - left.x).abs() < 1e-3, "{rect:?}");
    assert!((rect.right() - right.right()).abs() < 1e-3, "{rect:?}");
    assert_eq!(rect.height, attributions[0].extent(rect.width).thickness);
    // Every panel's credits resolve to it.
    for index in 1..4 {
        let representative = solution.representative(&Id::Credits(index)).unwrap();
        assert_eq!(representative.id, Some(Id::Credits(0)));
    }
}

#[test]
fn a_panel_with_other_credits_keeps_its_own() {
    let engine = engine();
    let mut attributions: Vec<Attribution> =
        (0..3).map(|_| attribution(&SOURCES, &engine)).collect();
    attributions.push(attribution(
        &[
            SOURCES[0],
            SOURCES[1],
            "Tiles: © OpenStreetMap contributors",
        ],
        &engine,
    ));
    let solution = solve(&attributions);
    // Without one key, nothing merges. The first column's equal credits collapse to the lower
    // panel's, and the second column keeps both of its panels' credits.
    let mut ids: Vec<Id> = drawn(&solution).into_iter().map(|(id, _)| id).collect();
    ids.sort_by_key(|id| match id {
        Id::Credits(index) | Id::Panel(index) => *index,
    });
    assert_eq!(ids, [Id::Credits(1), Id::Credits(2), Id::Credits(3)]);
    for (id, rect) in drawn(&solution) {
        let Id::Credits(index) = id else {
            unreachable!("credits");
        };
        let panel = solution.region(&Id::Panel(index)).unwrap().content;
        assert!(
            (rect.width - panel.width).abs() < 1e-3,
            "{id:?}: {rect:?} under {panel:?}"
        );
    }
}
