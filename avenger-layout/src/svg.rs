//! Debug data model and SVG export for solved layouts.
//!
//! This module makes solver results inspectable without any renderer: a
//! [`DebugScene`] captures the solved content rectangles and edge envelopes,
//! and [`DebugScene::to_svg`] writes a small self-contained SVG (white
//! background, labeled regions). It is deliberately independent of any scenegraph; output is
//! deterministic so SVG strings can be snapshot-tested.
//!
//! Legend of the rendered layers:
//!
//! - filled blue: content rectangles,
//! - black frame: the arrangement's content bounds (for a frame scene, the
//!   solved envelope extent),
//! - overflow strips beside each content rectangle: red for the inner
//!   (guide-like) layer, green for the remainder up to the total. The
//!   lighter shade is the coordinated target the solve produced; the darker
//!   shade overlaid on it is what the region itself requested — where they
//!   differ, coordination grew the region's allocation.
//!
//! Frame scenes ([`DebugScene::from_frame`]) additionally tile the chrome
//! layers as translucent strips, one per solved slab: gray margins, amber
//! bands, green outer (legend-like) layers, and red inner (guide-like)
//! layers. Bands, outer, and inner strips span the content on their cross
//! axis (as realized chart chrome does); margins span the full envelope.
//! Strip labels anchor inside the content span, strips too thin to hold a
//! label go unlabeled, and a color key row below the scene identifies the
//! layer kinds. All label text carries a white halo so overlapping labels
//! stay legible. Placement scenes ([`DebugScene::from_placements`]) draw
//! each child origin as a labeled cross marker.

use std::fmt::Display;
use std::fmt::Write as _;

use crate::band::{BandSolution, BoundaryDemand};
use crate::frame::{FrameAxisSolution, FrameSolution, SolvedSlab};
use crate::geometry::{Edges, Orientation, Rect, Side, Size};
use crate::grid::{GridItem, GridSolution, UniformTrackSolution};
use crate::region::{EdgeTargets, PlacementSolution};
use crate::tree::TreeSolution;

/// What a region represents, which selects its rendered style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DebugRegionKind {
    /// A content rectangle (filled blue).
    #[default]
    Content,
    /// A frame margin strip (gray).
    Margin,
    /// A frame band strip, e.g. a title row (amber).
    Band,
    /// A frame outer (legend-like) strip (green).
    Outer,
    /// A frame inner (guide-like) strip (red).
    Inner,
    /// An arrangement boundary (black frame, no fill) — used when several
    /// independent scenes are composed into one image and each needs its
    /// own bounds.
    Bounds,
}

/// One labeled region in a solved layout.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugRegion {
    /// Label text; an empty label draws nothing.
    pub label: String,
    pub kind: DebugRegionKind,
    pub content: Rect,
    /// The overflow this region asked for (its own measured demand), when
    /// known. Drawn as darker strips inside the coordinated bands.
    pub requested: Option<EdgeTargets>,
    /// The coordinated overflow the solve produced for this region, when
    /// known. Drawn as lighter strips: red for the inner layer, green for
    /// the remainder up to the total.
    pub target: Option<EdgeTargets>,
    /// Where to draw the label. Defaults to just inside the content
    /// rectangle's top-left corner; frame scenes anchor strip labels inside
    /// the content span so they stay clear of the perpendicular strips.
    pub label_anchor: Option<[f32; 2]>,
    /// Draw the label rotated 90 degrees (reads downward), for labels that
    /// run along a tall narrow strip.
    pub label_rotated: bool,
}

/// One labeled point marker (e.g. a placement origin).
#[derive(Clone, Debug, PartialEq)]
pub struct DebugMarker {
    pub label: String,
    pub position: [f32; 2],
}

/// A solved layout captured for inspection.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugScene {
    /// The arrangement's solved content size.
    pub content_size: Size,
    /// Draw the black bounds frame around `content_size`. Disable when
    /// composing several independent scenes into one image and giving each
    /// its own [`DebugRegionKind::Bounds`] region instead.
    pub draw_bounds: bool,
    pub regions: Vec<DebugRegion>,
    pub markers: Vec<DebugMarker>,
    /// Horizontal separator lines spanning the full image width at these y
    /// positions (for composed galleries of independent scenes).
    pub dividers: Vec<f32>,
}

impl DebugScene {
    /// Capture a solved grid: one region per item, with content rectangles
    /// from the solved tracks and envelopes from the slot edge targets.
    pub fn from_grid<Id: Display>(solution: &GridSolution, items: &[GridItem<Id>]) -> Self {
        let regions = items
            .iter()
            .map(|item| {
                let slot = item.slot;
                let origin = solution.content_origin_for_slot(slot);
                let size = solution.content_size_for_slot(slot);
                let content = Rect::new(origin[0], origin[1], size.width, size.height);
                let targets = solution.edge_targets_for_slot(slot);
                let span_label = if slot.row_span > 1 || slot.column_span > 1 {
                    format!(" span {}x{}", slot.row_span, slot.column_span)
                } else {
                    String::new()
                };
                DebugRegion {
                    label: format!("{} r{}c{}{}", item.id, slot.row, slot.column, span_label),
                    kind: DebugRegionKind::Content,
                    content,
                    requested: Some(item.requested_targets()),
                    target: Some(targets),
                    label_anchor: None,
                    label_rotated: false,
                }
            })
            .collect();

        Self {
            content_size: solution.content_size,
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Capture a solved band: one region per placed child, with requested
    /// and coordinated main-axis boundary overflow from
    /// [`BandSolution::boundaries`].
    pub fn from_band<Id: Display>(band: &BandSolution<Id>) -> Self {
        let cross_extent = band.cross_extent.unwrap_or(0.0);
        let content_size = match band.orientation {
            Orientation::Horizontal => Size::new(band.main_extent, cross_extent),
            Orientation::Vertical => Size::new(cross_extent, band.main_extent),
        };
        let regions = band
            .items
            .iter()
            .zip(
                band.boundaries
                    .iter()
                    .copied()
                    .chain(std::iter::repeat(crate::band::BandBoundary::default())),
            )
            .map(|(child, boundary)| {
                // Boundary demands are main-axis totals; map them onto the
                // band's main-axis sides (no inner/outer split).
                let main_axis_edges = |before: f32, after: f32| match band.orientation {
                    Orientation::Horizontal => Edges::new(0.0, after, 0.0, before),
                    Orientation::Vertical => Edges::new(before, 0.0, after, 0.0),
                };
                let demand = |boundary: BoundaryDemand| EdgeTargets {
                    inner: Edges::default(),
                    total: main_axis_edges(boundary.before.max(0.0), boundary.after.max(0.0)),
                };
                let content = match band.orientation {
                    Orientation::Horizontal => Rect::new(
                        child.main_start,
                        child.cross_start,
                        child.main_size,
                        child.cross_size,
                    ),
                    Orientation::Vertical => Rect::new(
                        child.cross_start,
                        child.main_start,
                        child.cross_size,
                        child.main_size,
                    ),
                };
                DebugRegion {
                    label: format!("{}", child.id),
                    kind: DebugRegionKind::Content,
                    content,
                    requested: Some(demand(boundary.requested)),
                    target: Some(demand(boundary.target)),
                    label_anchor: None,
                    label_rotated: false,
                }
            })
            .collect();

        Self {
            content_size,
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Capture a solved layout tree: one region per solved node or leaf,
    /// labeled with its ID and depth, in root coordinates.
    pub fn from_tree<Id: Display>(tree: &TreeSolution<Id>) -> Self {
        let regions = tree
            .regions
            .iter()
            .map(|region| {
                let content = region.content_rect;
                DebugRegion {
                    label: format!("{} d{}", region.id, region.depth),
                    kind: DebugRegionKind::Content,
                    content,
                    requested: Some(region.requested),
                    target: Some(region.edge_targets),
                    // Nested regions share corners with their ancestors;
                    // stepping the label down one line per depth keeps
                    // every label readable.
                    label_anchor: Some([
                        content.x + 3.0,
                        content.y + 12.0 + 12.0 * region.depth as f32,
                    ]),
                    label_rotated: false,
                }
            })
            .collect();
        Self {
            content_size: tree.content_size,
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Capture a solved frame: the content rectangle plus one strip per
    /// non-empty chrome slab.
    ///
    /// A frame solves each axis independently, so a slab has no cross
    /// extent of its own; this renders bands, outer, and inner strips
    /// spanning the content on the cross axis — matching how chart chrome
    /// (guide and legend rects) is realized — while margins span the full
    /// envelope, since they genuinely wrap everything.
    pub fn from_frame(solution: &FrameSolution) -> Self {
        /// Strips thinner than this get no in-strip label; the color key
        /// identifies the layer instead. Rotated labels need more strip
        /// width than horizontal labels need height.
        const MIN_LABELED_STRIP: f32 = 9.0;
        const MIN_ROTATED_LABELED_STRIP: f32 = 12.0;

        let extent = solution.extent();
        // Strip labels anchor inside the content span on the strip's long
        // axis: that segment is guaranteed clear of the perpendicular
        // strips, so labels never pile up in the double-covered corners.
        let h_content = solution.horizontal.content;
        let v_content = solution.vertical.content;
        let content_x = h_content.start;
        let content_y = v_content.start;
        let mut regions = Vec::new();

        let mut push_axis = |axis: &FrameAxisSolution, vertical: bool| {
            let (lead, trail) = if vertical { ("t", "b") } else { ("l", "r") };
            for (prefix, side) in [(lead, &axis.leading), (trail, &axis.trailing)] {
                let mut push = |label: String, kind: DebugRegionKind, slab: SolvedSlab| {
                    if slab.size <= 0.0 {
                        return;
                    }
                    let labeled = slab.size
                        >= if vertical {
                            MIN_LABELED_STRIP
                        } else {
                            MIN_ROTATED_LABELED_STRIP
                        };
                    // Vertically (resp. horizontally) center the label in
                    // the strip; 3.5 is half the cap height of the 10px
                    // monospace face.
                    let centered = slab.start + slab.size / 2.0 + 3.5;
                    // Margins wrap the whole envelope; every other layer
                    // spans the content on its cross axis, as realized
                    // chart chrome does.
                    let full_bleed = kind == DebugRegionKind::Margin;
                    let (content, label_anchor) = if vertical {
                        let (x, width) = if full_bleed {
                            (0.0, extent.width)
                        } else {
                            (h_content.start, h_content.size)
                        };
                        (
                            Rect::new(x, slab.start, width, slab.size),
                            [content_x + 3.0, centered],
                        )
                    } else {
                        let (y, height) = if full_bleed {
                            (0.0, extent.height)
                        } else {
                            (v_content.start, v_content.size)
                        };
                        (
                            Rect::new(slab.start, y, slab.size, height),
                            [centered, content_y + 3.0],
                        )
                    };
                    regions.push(DebugRegion {
                        label: if labeled { label } else { String::new() },
                        kind,
                        content,
                        requested: None,
                        target: None,
                        label_anchor: Some(label_anchor),
                        label_rotated: !vertical,
                    });
                };
                push(
                    format!("{prefix} margin"),
                    DebugRegionKind::Margin,
                    side.margin,
                );
                for (index, band) in side.bands.iter().enumerate() {
                    push(
                        format!("{prefix} band {index}"),
                        DebugRegionKind::Band,
                        *band,
                    );
                }
                push(
                    format!("{prefix} outer"),
                    DebugRegionKind::Outer,
                    side.outer,
                );
                push(
                    format!("{prefix} inner"),
                    DebugRegionKind::Inner,
                    side.inner,
                );
            }
        };
        push_axis(&solution.vertical, true);
        push_axis(&solution.horizontal, false);

        regions.push(DebugRegion {
            label: "content".to_string(),
            kind: DebugRegionKind::Content,
            content: solution.content_rect(),
            requested: None,
            target: None,
            label_anchor: None,
            label_rotated: false,
        });

        Self {
            content_size: extent,
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Capture solved uniform tracks, rendered horizontally: one region per
    /// track at the given cross extent.
    pub fn from_uniform_tracks(solution: &UniformTrackSolution, cross_extent: f32) -> Self {
        let regions = solution
            .starts
            .iter()
            .enumerate()
            .map(|(index, start)| DebugRegion {
                label: format!("track {index}"),
                kind: DebugRegionKind::Content,
                content: Rect::new(*start, 0.0, solution.track_size, cross_extent),
                requested: None,
                target: None,
                label_anchor: None,
                label_rotated: false,
            })
            .collect();
        Self {
            content_size: Size::new(solution.extent, cross_extent),
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Capture a placement handoff: each child origin as a labeled marker
    /// within the parent's content bounds.
    pub fn from_placements<Id: Display, M>(solution: &PlacementSolution<Id, M>) -> Self {
        let markers = solution
            .placements
            .iter()
            .map(|placement| DebugMarker {
                label: format!("{}", placement.id),
                position: placement.origin,
            })
            .collect();
        Self {
            content_size: solution.content_size,
            draw_bounds: true,
            regions: Vec::new(),
            markers,
            dividers: Vec::new(),
        }
    }

    /// Write the scene as a self-contained SVG document.
    pub fn to_svg(&self) -> String {
        const PADDING: f32 = 10.0;
        /// White halo under label text so overlapping labels stay legible.
        const TEXT_STYLE: &str = "fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\"";
        /// Swatch order and labels for the color key row.
        const KEY_KINDS: [(DebugRegionKind, &str); 5] = [
            (DebugRegionKind::Content, "content"),
            (DebugRegionKind::Margin, "margin"),
            (DebugRegionKind::Band, "band"),
            (DebugRegionKind::Outer, "outer"),
            (DebugRegionKind::Inner, "inner"),
        ];
        const KEY_HEIGHT: f32 = 24.0;
        const DEMAND_INNER_LIGHT: &str = "fill=\"#fecaca\" fill-opacity=\"0.6\"";
        const DEMAND_INNER_DARK: &str = "fill=\"#dc2626\" fill-opacity=\"0.75\"";
        const DEMAND_OUTER_LIGHT: &str = "fill=\"#bbf7d0\" fill-opacity=\"0.6\"";
        const DEMAND_OUTER_DARK: &str = "fill=\"#16a34a\" fill-opacity=\"0.75\"";
        const DEMAND_KEY_LABELS: [&str; 2] = ["inner", "outer"];

        let bounds_rect = Rect::new(0.0, 0.0, self.content_size.width, self.content_size.height);
        let mut bounds = bounds_rect;
        for region in &self.regions {
            bounds = union(bounds, region.content);
            for targets in [region.requested, region.target].into_iter().flatten() {
                bounds = union(
                    bounds,
                    expand(
                        region.content,
                        [
                            targets.total.top,
                            targets.total.right,
                            targets.total.bottom,
                            targets.total.left,
                        ],
                    ),
                );
            }
        }
        for marker in &self.markers {
            bounds = union(
                bounds,
                Rect::new(marker.position[0] - 4.0, marker.position[1] - 4.0, 8.0, 8.0),
            );
        }

        // A color key row is shown whenever chrome strips are present
        // (frame scenes); content-only scenes stay minimal.
        let has_chrome_kinds = self.regions.iter().any(|region| {
            region.kind != DebugRegionKind::Content && region.kind != DebugRegionKind::Bounds
        });
        let any_side = |edges: Edges<f32>| {
            edges.top > 0.0 || edges.right > 0.0 || edges.bottom > 0.0 || edges.left > 0.0
        };
        let demand_layers = self.regions.iter().fold((false, false), |layers, region| {
            let Some(target) = region.target else {
                return layers;
            };
            let inner = layers.0 || any_side(target.inner);
            // The outer strip is the remainder up to the total.
            let outer = layers.1
                || any_side(Edges::new(
                    target.total.top - target.inner.top,
                    target.total.right - target.inner.right,
                    target.total.bottom - target.inner.bottom,
                    target.total.left - target.inner.left,
                ));
            (inner, outer)
        });
        let has_demands = demand_layers.0 || demand_layers.1;
        let show_key = has_chrome_kinds || has_demands;
        let key_y = bounds.y + bounds.height + 10.0;
        if show_key {
            bounds.height += KEY_HEIGHT;
            // The key row must also fit horizontally in narrow scenes.
            let mut key_width: f32 = KEY_KINDS
                .iter()
                .filter(|(kind, _)| self.regions.iter().any(|region| region.kind == *kind))
                .map(|(_, label)| 13.0 + label.len() as f32 * 6.0 + 14.0)
                .sum();
            for (present, label) in [
                (demand_layers.0, DEMAND_KEY_LABELS[0]),
                (demand_layers.1, DEMAND_KEY_LABELS[1]),
            ] {
                if present {
                    key_width += 13.0 + label.len() as f32 * 6.0 + 14.0;
                }
            }
            key_width -= 14.0;
            bounds.width = bounds.width.max(bounds.x.max(0.0) - bounds.x + key_width);
        }

        let mut svg = String::new();
        let _ = write!(
            svg,
            concat!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" ",
                "viewBox=\"{} {} {} {}\" font-family=\"monospace\" font-size=\"10\">\n"
            ),
            bounds.x - PADDING,
            bounds.y - PADDING,
            bounds.width + 2.0 * PADDING,
            bounds.height + 2.0 * PADDING,
        );
        // White background covering the viewBox: the scenes are unreadable
        // over transparent-background viewers (e.g. dark-mode browsers).
        let _ = write!(
            svg,
            "  {}\n",
            rect_element(
                Rect::new(
                    bounds.x - PADDING,
                    bounds.y - PADDING,
                    bounds.width + 2.0 * PADDING,
                    bounds.height + 2.0 * PADDING,
                ),
                "fill=\"#ffffff\""
            )
        );
        if self.draw_bounds {
            let _ = write!(
                svg,
                "  {}\n",
                rect_element(
                    bounds_rect,
                    "fill=\"none\" stroke=\"#111111\" stroke-width=\"1\""
                )
            );
        }

        for region in &self.regions {
            // Coordinated (lighter) bands first, then the requested
            // (darker) demand inside them: red for the inner layer, green
            // for the remainder up to the total.
            if let Some(target) = region.target {
                let requested = region.requested.unwrap_or_default();
                for (side_target, side_requested, side) in [
                    (
                        (target.inner.top, target.total.top),
                        (requested.inner.top, requested.total.top),
                        Side::Top,
                    ),
                    (
                        (target.inner.right, target.total.right),
                        (requested.inner.right, requested.total.right),
                        Side::Right,
                    ),
                    (
                        (target.inner.bottom, target.total.bottom),
                        (requested.inner.bottom, requested.total.bottom),
                        Side::Bottom,
                    ),
                    (
                        (target.inner.left, target.total.left),
                        (requested.inner.left, requested.total.left),
                        Side::Left,
                    ),
                ] {
                    let (target_inner, target_total) = side_target;
                    let (requested_inner, requested_total) = side_requested;
                    let strip = |offset: f32, thickness: f32| {
                        edge_strip(region.content, side, offset, thickness)
                    };
                    for (rect, fill) in [
                        (strip(0.0, target_inner), DEMAND_INNER_LIGHT),
                        (
                            strip(target_inner, (target_total - target_inner).max(0.0)),
                            DEMAND_OUTER_LIGHT,
                        ),
                        (
                            strip(0.0, requested_inner.min(target_inner)),
                            DEMAND_INNER_DARK,
                        ),
                        (
                            strip(target_inner, (requested_total - requested_inner).max(0.0)),
                            DEMAND_OUTER_DARK,
                        ),
                    ] {
                        if rect.width > 0.0 && rect.height > 0.0 {
                            let _ = write!(svg, "  {}\n", rect_element(rect, fill));
                        }
                    }
                }
            }
            let _ = write!(
                svg,
                "  {}\n",
                rect_element(region.content, kind_style(region.kind))
            );
            if region.label.is_empty() {
                continue;
            }
            let [x, y] = region
                .label_anchor
                .unwrap_or([region.content.x + 3.0, region.content.y + 12.0]);
            if region.label_rotated {
                let _ = write!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" transform=\"rotate(90 {} {})\" {}>{}</text>\n",
                    x,
                    y,
                    x,
                    y,
                    TEXT_STYLE,
                    escape_text(&region.label),
                );
            } else {
                let _ = write!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" {}>{}</text>\n",
                    x,
                    y,
                    TEXT_STYLE,
                    escape_text(&region.label),
                );
            }
        }

        for marker in &self.markers {
            let [x, y] = marker.position;
            let _ = write!(
                svg,
                concat!(
                    "  <path d=\"M {} {} L {} {} M {} {} L {} {}\" ",
                    "stroke=\"#111827\" stroke-width=\"1.5\"/>\n"
                ),
                x - 4.0,
                y,
                x + 4.0,
                y,
                x,
                y - 4.0,
                x,
                y + 4.0,
            );
            let _ = write!(
                svg,
                "  <text x=\"{}\" y=\"{}\" {}>{}</text>\n",
                x + 6.0,
                y - 3.0,
                TEXT_STYLE,
                escape_text(&marker.label),
            );
        }

        if show_key {
            let mut cursor = bounds.x.max(0.0);
            let entry_text = |svg: &mut String, cursor: f32, label: &str| {
                let _ = write!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" {}>{}</text>\n",
                    cursor + 13.0,
                    key_y + 9.0,
                    TEXT_STYLE,
                    label,
                );
            };
            for (kind, label) in KEY_KINDS {
                if !self.regions.iter().any(|region| region.kind == kind) {
                    continue;
                }
                let _ = write!(
                    svg,
                    "  {}\n",
                    rect_element(Rect::new(cursor, key_y, 10.0, 10.0), kind_style(kind))
                );
                entry_text(&mut svg, cursor, label);
                cursor += 13.0 + label.len() as f32 * 6.0 + 14.0;
            }
            if has_demands {
                // Split swatches: darker half = requested, lighter half =
                // coordinated.
                for (present, label, dark, light) in [
                    (
                        demand_layers.0,
                        "inner",
                        DEMAND_INNER_DARK,
                        DEMAND_INNER_LIGHT,
                    ),
                    (
                        demand_layers.1,
                        "outer",
                        DEMAND_OUTER_DARK,
                        DEMAND_OUTER_LIGHT,
                    ),
                ] {
                    if !present {
                        continue;
                    }
                    let _ = write!(
                        svg,
                        "  {}\n",
                        rect_element(Rect::new(cursor, key_y, 5.0, 10.0), dark)
                    );
                    let _ = write!(
                        svg,
                        "  {}\n",
                        rect_element(Rect::new(cursor + 5.0, key_y, 5.0, 10.0), light)
                    );
                    entry_text(&mut svg, cursor, label);
                    cursor += 13.0 + label.len() as f32 * 6.0 + 14.0;
                }
            }
        }

        for y in &self.dividers {
            let _ = write!(
                svg,
                concat!(
                    "  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" ",
                    "stroke=\"#111111\" stroke-width=\"0.5\"/>\n"
                ),
                bounds.x - PADDING,
                y,
                bounds.x + bounds.width + PADDING,
                y,
            );
        }

        svg.push_str("</svg>\n");
        svg
    }
}

/// One overflow strip adjacent to a content rectangle: `offset` is the
/// distance from the content edge where the strip starts, `thickness` its
/// extent outward; the strip spans the content on the cross axis.
fn edge_strip(content: Rect, side: Side, offset: f32, thickness: f32) -> Rect {
    match side {
        Side::Top => Rect::new(
            content.x,
            content.y - offset - thickness,
            content.width,
            thickness,
        ),
        Side::Right => Rect::new(
            content.x + content.width + offset,
            content.y,
            thickness,
            content.height,
        ),
        Side::Bottom => Rect::new(
            content.x,
            content.y + content.height + offset,
            content.width,
            thickness,
        ),
        Side::Left => Rect::new(
            content.x - offset - thickness,
            content.y,
            thickness,
            content.height,
        ),
    }
}

/// Expand a rectangle by per-side amounts `[top, right, bottom, left]`.
fn expand(rect: Rect, sides: [f32; 4]) -> Rect {
    let [top, right, bottom, left] = sides;
    Rect::new(
        rect.x - left,
        rect.y - top,
        rect.width + left + right,
        rect.height + top + bottom,
    )
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect::new(x, y, right - x, bottom - y)
}

/// Rendered style per region kind.
fn kind_style(kind: DebugRegionKind) -> &'static str {
    match kind {
        DebugRegionKind::Content => "fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"",
        DebugRegionKind::Margin => "fill=\"#e5e7eb\" fill-opacity=\"0.6\" stroke=\"#6b7280\"",
        DebugRegionKind::Band => "fill=\"#fde68a\" fill-opacity=\"0.6\" stroke=\"#d97706\"",
        DebugRegionKind::Outer => "fill=\"#bbf7d0\" fill-opacity=\"0.6\" stroke=\"#16a34a\"",
        DebugRegionKind::Inner => "fill=\"#fecaca\" fill-opacity=\"0.6\" stroke=\"#dc2626\"",
        DebugRegionKind::Bounds => "fill=\"none\" stroke=\"#111111\" stroke-width=\"1\"",
    }
}

fn rect_element(rect: Rect, attributes: &str) -> String {
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {}/>",
        rect.x, rect.y, rect.width, rect.height, attributes
    )
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::band::{BandItem, BoundaryDemand, CrossAlign};
    use crate::geometry::Edges;
    use crate::grid::TrackSpacing;
    use crate::grid::{GridRequirements, GridShape, GridSlot};

    fn band_scene() -> DebugScene {
        let band = BandSolution::solve(
            Orientation::Horizontal,
            &[
                BandItem {
                    id: 0,
                    main_size: 30.0,
                    cross_size: 80.0,
                    boundary: BoundaryDemand::default(),
                },
                BandItem {
                    id: 1,
                    main_size: 40.0,
                    cross_size: 90.0,
                    boundary: BoundaryDemand::default(),
                },
            ],
            TrackSpacing {
                outer_start: 5.0,
                outer_end: 7.0,
                min_gap: 10.0,
            },
            CrossAlign::default(),
        );
        DebugScene::from_band(&band)
    }

    fn grid_scene() -> DebugScene {
        let shape = GridShape {
            rows: 1,
            columns: 2,
        };
        let items = vec![
            GridItem {
                id: 0,
                slot: GridSlot {
                    row: 0,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
                content_size: Size::new(100.0, 60.0),
                inner_edges: Edges::new(0.0, 6.0, 0.0, 0.0),
                outer_edges: Edges::new(0.0, 20.0, 0.0, 0.0),
                total_edges: Edges::new(0.0, 12.0, 0.0, 0.0),
            },
            GridItem {
                id: 1,
                slot: GridSlot {
                    row: 0,
                    column: 1,
                    row_span: 1,
                    column_span: 1,
                },
                content_size: Size::new(100.0, 60.0),
                inner_edges: Edges::default(),
                outer_edges: Edges::default(),
                total_edges: Edges::new(0.0, 0.0, 0.0, 3.0),
            },
        ];
        let requirements = GridRequirements::from_items(shape, Size::new(100.0, 60.0), &items)
            .expect("test grid items fit the shape");
        let solution = requirements.solve(&items);
        DebugScene::from_grid(&solution, &items)
    }

    #[test]
    fn band_scene_captures_placed_children() {
        let scene = band_scene();

        assert_eq!(scene.content_size, Size::new(92.0, 90.0));
        assert_eq!(scene.regions.len(), 2);
        assert_eq!(scene.regions[0].content, Rect::new(5.0, 0.0, 30.0, 80.0));
        assert_eq!(scene.regions[1].content, Rect::new(45.0, 0.0, 40.0, 90.0));
        // Boundary demands default to zero in this scene.
        let target = scene.regions[0].target.expect("band regions carry targets");
        assert_eq!(target.total, Edges::default());
    }

    #[test]
    fn grid_scene_captures_content_and_demands() {
        let scene = grid_scene();

        // Gap between tracks: item 0 right total (max(12, 6+20) = 26) plus
        // item 1 left total (3) = 29.
        assert_eq!(scene.regions[0].content, Rect::new(0.0, 0.0, 100.0, 60.0));
        assert_eq!(scene.regions[1].content, Rect::new(129.0, 0.0, 100.0, 60.0));
        let requested = scene.regions[0]
            .requested
            .expect("grid regions carry requested demand");
        let target = scene.regions[0].target.expect("grid regions carry targets");
        assert_eq!(requested.inner.right, 6.0);
        assert_eq!(requested.total.right, 26.0); // lifted: max(12, 6 + 20)
        assert_eq!(target.inner.right, 6.0);
        assert_eq!(target.total.right, 26.0);
        let target_1 = scene.regions[1].target.expect("grid regions carry targets");
        assert_eq!(target_1.total.left, 3.0);
        assert_eq!(scene.content_size, Size::new(229.0, 60.0));
    }

    #[test]
    fn solved_tree_svg_snapshot() {
        use crate::grid::{GridShape, GridSlot, TrackSpacing};
        use crate::tree::{LayoutItem, LayoutNode, LayoutSlotContent};

        fn leaf(id: usize, column: usize, width: f32) -> LayoutItem<usize> {
            LayoutItem {
                id,
                slot: GridSlot {
                    row: 0,
                    column,
                    row_span: 1,
                    column_span: 1,
                },
                content: LayoutSlotContent::Leaf {
                    content_size: Size::new(width, 60.0),
                    inner_edges: Edges::default(),
                    outer_edges: Edges::default(),
                    total_edges: Edges::default(),
                },
            }
        }
        fn band(items: Vec<LayoutItem<usize>>, min_gap: f32) -> LayoutNode<usize> {
            LayoutNode {
                shape: GridShape {
                    rows: 1,
                    columns: items.len(),
                },
                column_spacing: TrackSpacing {
                    min_gap,
                    ..Default::default()
                },
                row_spacing: TrackSpacing::default(),
                base_cell_size: Size::default(),
                stacked_inner_edges: Edges::default(),
                stacked_outer_edges: Edges::default(),
                items,
            }
        }

        let inner = band(vec![leaf(10, 0, 40.0), leaf(11, 1, 40.0)], 6.0);
        let root = band(
            vec![
                leaf(0, 0, 50.0),
                LayoutItem {
                    id: 1,
                    slot: GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Node(inner),
                },
            ],
            10.0,
        );
        let solved = root.solve(None).expect("tree should solve");

        let svg = DebugScene::from_tree(&solved).to_svg();
        let expected = "\
<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-10 -10 166 80\" font-family=\"monospace\" font-size=\"10\">
  <rect x=\"-10\" y=\"-10\" width=\"166\" height=\"80\" fill=\"#ffffff\"/>
  <rect x=\"0\" y=\"0\" width=\"146\" height=\"60\" fill=\"none\" stroke=\"#111111\" stroke-width=\"1\"/>
  <rect x=\"0\" y=\"0\" width=\"50\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"3\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">0 d0</text>
  <rect x=\"60\" y=\"0\" width=\"86\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"63\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">1 d0</text>
  <rect x=\"60\" y=\"0\" width=\"40\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"63\" y=\"24\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">10 d1</text>
  <rect x=\"106\" y=\"0\" width=\"40\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"109\" y=\"24\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">11 d1</text>
</svg>
";
        assert_eq!(svg, expected);
    }

    #[test]
    fn band_svg_snapshot() {
        let svg = band_scene().to_svg();

        let expected = "\
<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-10 -10 112 110\" font-family=\"monospace\" font-size=\"10\">
  <rect x=\"-10\" y=\"-10\" width=\"112\" height=\"110\" fill=\"#ffffff\"/>
  <rect x=\"0\" y=\"0\" width=\"92\" height=\"90\" fill=\"none\" stroke=\"#111111\" stroke-width=\"1\"/>
  <rect x=\"5\" y=\"0\" width=\"30\" height=\"80\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"8\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">0</text>
  <rect x=\"45\" y=\"0\" width=\"40\" height=\"90\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"48\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">1</text>
</svg>
";
        assert_eq!(svg, expected);
    }

    #[test]
    fn grid_svg_snapshot() {
        let svg = grid_scene().to_svg();

        let expected = "\
<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-10 -10 249 104\" font-family=\"monospace\" font-size=\"10\">
  <rect x=\"-10\" y=\"-10\" width=\"249\" height=\"104\" fill=\"#ffffff\"/>
  <rect x=\"0\" y=\"0\" width=\"229\" height=\"60\" fill=\"none\" stroke=\"#111111\" stroke-width=\"1\"/>
  <rect x=\"100\" y=\"0\" width=\"6\" height=\"60\" fill=\"#fecaca\" fill-opacity=\"0.6\"/>
  <rect x=\"106\" y=\"0\" width=\"20\" height=\"60\" fill=\"#bbf7d0\" fill-opacity=\"0.6\"/>
  <rect x=\"100\" y=\"0\" width=\"6\" height=\"60\" fill=\"#dc2626\" fill-opacity=\"0.75\"/>
  <rect x=\"106\" y=\"0\" width=\"20\" height=\"60\" fill=\"#16a34a\" fill-opacity=\"0.75\"/>
  <rect x=\"0\" y=\"0\" width=\"100\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"3\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">0 r0c0</text>
  <rect x=\"126\" y=\"0\" width=\"3\" height=\"60\" fill=\"#bbf7d0\" fill-opacity=\"0.6\"/>
  <rect x=\"126\" y=\"0\" width=\"3\" height=\"60\" fill=\"#16a34a\" fill-opacity=\"0.75\"/>
  <rect x=\"129\" y=\"0\" width=\"100\" height=\"60\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"132\" y=\"12\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">1 r0c1</text>
  <rect x=\"0\" y=\"70\" width=\"10\" height=\"10\" fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#1d4ed8\"/>
  <text x=\"13\" y=\"79\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">content</text>
  <rect x=\"69\" y=\"70\" width=\"5\" height=\"10\" fill=\"#dc2626\" fill-opacity=\"0.75\"/>
  <rect x=\"74\" y=\"70\" width=\"5\" height=\"10\" fill=\"#fecaca\" fill-opacity=\"0.6\"/>
  <text x=\"82\" y=\"79\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">inner</text>
  <rect x=\"126\" y=\"70\" width=\"5\" height=\"10\" fill=\"#16a34a\" fill-opacity=\"0.75\"/>
  <rect x=\"131\" y=\"70\" width=\"5\" height=\"10\" fill=\"#bbf7d0\" fill-opacity=\"0.6\"/>
  <text x=\"139\" y=\"79\" fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">outer</text>
</svg>
";
        assert_eq!(svg, expected);
    }
}
