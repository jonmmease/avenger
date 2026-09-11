//! Solution rendering: SVG export of solved layouts.
//!
//! [`LayoutSolution::to_svg`] writes a small self-contained SVG (white
//! background, labeled regions) and [`svg_panels`] stacks several solutions
//! with captions. Output is deterministic so SVG strings can be
//! snapshot-tested.
//!
//! Visual language: blue content rectangles; dashed gray slot (allotment)
//! outlines where granted space exceeds honest content; chrome slabs behind
//! (gray margins, amber strips, dark green legend, dark red guide); demand
//! strips beside each content rectangle where **hue** is the layer (red =
//! guide strata, green = legend strata
//! remainder up to the total — total-only demands and lift-law slack),
//! **shade** is nesting depth, and **solid vs hatched** is requested vs
//! granted. A color key row identifies every kind present; the black frame
//! is the canvas.

use std::fmt::Display;
use std::fmt::Write as _;

use crate::geometry::{Edges, Rect, Side, Size};
use crate::region::EdgeGrant;
use crate::solution::{ChromeLayer, LayoutSolution};

/// Rendering options for [`LayoutSolution::to_svg_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvgOptions {
    /// Draw region labels (ids / structural paths). The color key always
    /// renders.
    pub labels: bool,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self { labels: true }
    }
}

/// What a region represents, which selects its rendered style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DebugRegionKind {
    /// A content rectangle (filled blue).
    #[default]
    Content,
    /// A frame margin strip (gray).
    Margin,
    /// A frame chrome strip, e.g. a title row (amber).
    Strip,
    /// A frame legend strip (green).
    Legend,
    /// A frame guide strip (red).
    Guide,
    /// A slot (allotment) outline where it differs from the honest content
    /// rectangle (dashed gray) — granted space the content does not fill.
    Slot,
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
    /// known. Drawn as solid strips inside the granted regions.
    pub requested: Option<Edges<EdgeGrant>>,
    /// The coordinated overflow the solve produced for this region, when
    /// known. Drawn as hatched strips: red for the guide stratum, green
    /// for the legend stratum.
    pub target: Option<Edges<EdgeGrant>>,
    /// Where to draw the label. Defaults to just inside the content
    /// rectangle's top-left corner; frame scenes anchor strip labels inside
    /// the content span so they stay clear of the perpendicular strips.
    pub label_anchor: Option<[f32; 2]>,
    /// Draw the label rotated 90 degrees (reads downward), for labels that
    /// run along a tall narrow strip.
    pub label_rotated: bool,
    /// Nesting depth of the region (0 = top level). Selects the shade of
    /// its demand strips so layers of nested arrangements are tellable
    /// apart.
    pub depth: usize,
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
    /// Capture a unified-API solution: chrome slabs (behind), content
    /// rectangles with requested-vs-granted demand strips, and dashed slot
    /// outlines where the allotment exceeds the honest content.
    pub fn from_solution<Id: Display>(solution: &LayoutSolution<Id>, options: &SvgOptions) -> Self {
        let mut regions = Vec::new();
        for region in solution.regions() {
            // Declared chrome slabs first: they sit behind the demand
            // strips of the same node ("frame in the back").
            for slab in &region.slabs {
                regions.push(DebugRegion {
                    label: String::new(),
                    kind: match slab.layer {
                        ChromeLayer::Margin => DebugRegionKind::Margin,
                        ChromeLayer::Strip => DebugRegionKind::Strip,
                        ChromeLayer::Legend => DebugRegionKind::Legend,
                        ChromeLayer::Guide => DebugRegionKind::Guide,
                    },
                    content: slab.rect,
                    requested: None,
                    target: None,
                    label_anchor: None,
                    label_rotated: false,
                    depth: region.depth,
                });
            }
            // Slot outline when the allotment differs from the content —
            // skipped at the root, whose slot is the canvas itself.
            let slack = !region.path.is_empty()
                && ((region.slot.width - region.content.width).abs() > 0.5
                    || (region.slot.height - region.content.height).abs() > 0.5
                    || (region.slot.x - region.content.x).abs() > 0.5
                    || (region.slot.y - region.content.y).abs() > 0.5);
            if slack {
                regions.push(DebugRegion {
                    label: String::new(),
                    kind: DebugRegionKind::Slot,
                    content: region.slot,
                    requested: None,
                    target: None,
                    label_anchor: None,
                    label_rotated: false,
                    depth: region.depth,
                });
            }
            let label = if !options.labels {
                String::new()
            } else if let Some(id) = &region.id {
                id.to_string()
            } else if region.path.is_empty() {
                String::new() // the unlabeled canvas
            } else {
                format!(
                    "c{}",
                    region.path.iter().map(usize::to_string).collect::<String>()
                )
            };
            // Demand strips for nested regions only: the root's chrome is
            // already visible as slabs, and its demands wrap the canvas.
            let (requested, target) = if region.path.is_empty() {
                (None, None)
            } else {
                (Some(region.requested), Some(region.granted))
            };
            regions.push(DebugRegion {
                label,
                kind: DebugRegionKind::Content,
                content: region.content,
                requested,
                target,
                label_anchor: Some([
                    region.content.x + 3.0,
                    region.content.y + 12.0 + 12.0 * region.depth as f32,
                ]),
                label_rotated: false,
                depth: region.depth,
            });
        }
        Self {
            content_size: solution.size,
            draw_bounds: true,
            regions,
            markers: Vec::new(),
            dividers: Vec::new(),
        }
    }

    /// Embed another scene at an origin: regions, label anchors, and
    /// markers translate; the embedded scene's own bounds/dividers are
    /// dropped (the host scene owns the canvas). This is how composed
    /// solves render as one image — e.g. a tree solved inside a frame's
    /// content rectangle embeds at that rectangle's origin.
    pub fn embed(&mut self, scene: DebugScene, origin: [f32; 2]) {
        self.regions
            .extend(scene.regions.into_iter().map(|mut region| {
                region.content.x += origin[0];
                region.content.y += origin[1];
                if let Some(anchor) = &mut region.label_anchor {
                    anchor[0] += origin[0];
                    anchor[1] += origin[1];
                }
                region
            }));
        self.markers
            .extend(scene.markers.into_iter().map(|mut marker| {
                marker.position[0] += origin[0];
                marker.position[1] += origin[1];
                marker
            }));
    }

    /// Write the scene as a self-contained SVG document.
    pub fn to_svg(&self) -> String {
        const PADDING: f32 = 10.0;
        /// White halo under label text so overlapping labels stay legible.
        const TEXT_STYLE: &str = "fill=\"#111111\" stroke=\"#ffffff\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\"";
        /// Swatch order and labels for the color key row.
        const KEY_KINDS: [(DebugRegionKind, &str); 6] = [
            (DebugRegionKind::Content, "content"),
            (DebugRegionKind::Margin, "margin"),
            (DebugRegionKind::Strip, "strip"),
            (DebugRegionKind::Legend, "legend"),
            (DebugRegionKind::Guide, "guide"),
            (DebugRegionKind::Slot, "slot"),
        ];
        const KEY_HEIGHT: f32 = 24.0;
        // Demand strips encode three things: hue = stratum (red guide,
        // green legend), shade = nesting depth (dark at the base layer,
        // lighter as nesting deepens; a frame's own chrome strips are the
        // darkest step of the same ramps), and solid vs hatched =
        // requested vs coordinated. Fills are opaque: nested regions draw
        // overlapping strips, and translucency would invent in-between
        // shades where they stack.
        const STRIP_BORDER: &str = "stroke=\"#9ca3af\" stroke-width=\"0.5\" stroke-opacity=\"0.7\"";
        let guide_shade = |depth: usize| GUIDE_SHADES[(depth + 1).min(GUIDE_SHADES.len() - 1)];
        let legend_shade = |depth: usize| LEGEND_SHADES[(depth + 1).min(LEGEND_SHADES.len() - 1)];
        let layer_shade = |layer: char, depth: usize| match layer {
            'g' => guide_shade(depth),
            _ => legend_shade(depth),
        };
        let solid_fill = |shade: &str| format!("fill=\"{shade}\" {STRIP_BORDER}");
        let hatch_fill = |layer: char, depth: usize| {
            let shade = layer_shade(layer, depth);
            format!("fill=\"url(#hatch-{layer}{depth})\" stroke=\"{shade}\" stroke-width=\"1\"")
        };

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
                            targets.top.total,
                            targets.right.total,
                            targets.bottom.total,
                            targets.left.total,
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
        let any_side = |sides: Edges<EdgeGrant>, component: fn(EdgeGrant) -> f32| {
            component(sides.top) > 0.0
                || component(sides.right) > 0.0
                || component(sides.bottom) > 0.0
                || component(sides.left) > 0.0
        };
        // Which (layer, depth) demand combinations the scene contains; each
        // gets a shade, a hatch pattern, and a key entry.
        let mut inner_depths = std::collections::BTreeSet::new();
        let mut outer_depths = std::collections::BTreeSet::new();
        for region in &self.regions {
            for targets in [region.target, region.requested].into_iter().flatten() {
                if any_side(targets, |grant| grant.guide) {
                    inner_depths.insert(region.depth);
                }
                if any_side(targets, |grant| grant.legend) {
                    outer_depths.insert(region.depth);
                }
            }
        }
        let has_demands = !inner_depths.is_empty() || !outer_depths.is_empty();
        let multi_depth = inner_depths
            .iter()
            .chain(outer_depths.iter())
            .any(|&depth| depth > 0);
        let key_entries: Vec<(char, usize, String)> = inner_depths
            .iter()
            .map(|&depth| ('g', depth, String::from("guide")))
            .chain(
                outer_depths
                    .iter()
                    .map(|&depth| ('l', depth, String::from("legend"))),
            )
            .map(|(layer, depth, label)| {
                let label = if multi_depth {
                    format!("{label} d{depth}")
                } else {
                    label
                };
                (layer, depth, label)
            })
            .collect();
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
            for (_, _, label) in &key_entries {
                key_width += 13.0 + label.len() as f32 * 6.0 + 14.0;
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
        if has_demands {
            // Hatch patterns for coordinated strips, one per (layer, depth)
            // shade. Transparent between the lines so whatever sits behind
            // (a frame's reservation strips in composed scenes) shows
            // through; requested strips stay opaque solids, which keeps
            // nested overlaps from compositing.
            svg.push_str("  <defs>\n");
            let mut emit_pattern = |layer: char, depth: usize, shade: &str| {
                let _ = write!(
                    svg,
                    concat!(
                        "    <pattern id=\"hatch-{}{}\" patternUnits=\"userSpaceOnUse\" ",
                        "width=\"4\" height=\"4\" patternTransform=\"rotate(45)\">",
                        "<line x1=\"0\" y1=\"0\" x2=\"0\" y2=\"4\" stroke=\"{}\" ",
                        "stroke-width=\"1.6\"/></pattern>\n"
                    ),
                    layer, depth, shade,
                );
            };
            for &depth in &inner_depths {
                emit_pattern('g', depth, guide_shade(depth));
            }
            for &depth in &outer_depths {
                emit_pattern('l', depth, legend_shade(depth));
            }
            svg.push_str("  </defs>\n");
        }
        // White background covering the viewBox: the scenes are unreadable
        // over transparent-background viewers (e.g. dark-mode browsers).
        let _ = writeln!(
            svg,
            "  {}",
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
        for region in &self.regions {
            // Coordinated (hatched) regions first, then the requested (solid)
            // demand inside them. Per side, layers stack outward from the
            // content edge: red guide, then green legend. Solid strips draw
            // at the granted layer offsets so requested space nests inside
            // its coordinated region.
            if let Some(target) = region.target {
                let requested = region.requested.unwrap_or_default();
                for (granted, asked, side) in [
                    (target.top, requested.top, Side::Top),
                    (target.right, requested.right, Side::Right),
                    (target.bottom, requested.bottom, Side::Bottom),
                    (target.left, requested.left, Side::Left),
                ] {
                    let strip = |offset: f32, thickness: f32| {
                        edge_strip(region.content, side, offset, thickness)
                    };
                    for (rect, fill) in [
                        (strip(0.0, granted.guide), hatch_fill('g', region.depth)),
                        (
                            strip(granted.guide, granted.legend),
                            hatch_fill('l', region.depth),
                        ),
                        (
                            strip(0.0, asked.guide.min(granted.guide)),
                            solid_fill(guide_shade(region.depth)),
                        ),
                        (
                            strip(granted.guide, asked.legend),
                            solid_fill(legend_shade(region.depth)),
                        ),
                    ] {
                        if rect.width > 0.0 && rect.height > 0.0 {
                            let _ = writeln!(svg, "  {}", rect_element(rect, &fill));
                        }
                    }
                }
            }
            let _ = writeln!(
                svg,
                "  {}",
                rect_element(region.content, kind_style(region.kind))
            );
            if region.label.is_empty() {
                continue;
            }
            let [x, y] = region
                .label_anchor
                .unwrap_or([region.content.x + 3.0, region.content.y + 12.0]);
            if region.label_rotated {
                let _ = writeln!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" transform=\"rotate(90 {} {})\" {}>{}</text>",
                    x,
                    y,
                    x,
                    y,
                    TEXT_STYLE,
                    escape_text(&region.label),
                );
            } else {
                let _ = writeln!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" {}>{}</text>",
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
            let _ = writeln!(
                svg,
                "  <text x=\"{}\" y=\"{}\" {}>{}</text>",
                x + 6.0,
                y - 3.0,
                TEXT_STYLE,
                escape_text(&marker.label),
            );
        }

        if show_key {
            let mut cursor = bounds.x.max(0.0);
            let entry_text = |svg: &mut String, cursor: f32, label: &str| {
                let _ = writeln!(
                    svg,
                    "  <text x=\"{}\" y=\"{}\" {}>{}</text>",
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
                let _ = writeln!(
                    svg,
                    "  {}",
                    rect_element(Rect::new(cursor, key_y, 10.0, 10.0), kind_style(kind))
                );
                entry_text(&mut svg, cursor, label);
                cursor += 13.0 + label.len() as f32 * 6.0 + 14.0;
            }
            // Split swatches: solid half = requested, hatched half =
            // coordinated; shade encodes nesting depth.
            for (layer, depth, label) in &key_entries {
                let shade = layer_shade(*layer, *depth);
                // Same border as the hatched half so the split swatch
                // reads as one aligned chip.
                let _ = writeln!(
                    svg,
                    "  {}",
                    rect_element(
                        Rect::new(cursor, key_y, 5.0, 10.0),
                        &format!("fill=\"{shade}\" stroke=\"{shade}\" stroke-width=\"1\"")
                    )
                );
                let _ = writeln!(
                    svg,
                    "  {}",
                    rect_element(
                        Rect::new(cursor + 5.0, key_y, 5.0, 10.0),
                        &hatch_fill(*layer, *depth)
                    )
                );
                entry_text(&mut svg, cursor, label);
                cursor += 13.0 + label.len() as f32 * 6.0 + 14.0;
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

        // The canvas outline draws last so chrome strips never paint over
        // it.
        if self.draw_bounds {
            let _ = writeln!(
                svg,
                "  {}",
                rect_element(
                    bounds_rect,
                    "fill=\"none\" stroke=\"#111111\" stroke-width=\"1\""
                )
            );
        }

        svg.push_str("</svg>\n");
        svg
    }
}

impl<Id: Display> LayoutSolution<Id> {
    /// Render this solution as a self-contained SVG document (the debug
    /// gallery visual language: blue content, chrome slabs behind,
    /// requested-vs-granted demand strips, dashed slot outlines).
    pub fn to_svg(&self) -> String {
        self.to_svg_with(&SvgOptions::default())
    }

    /// [`LayoutSolution::to_svg`] with options.
    pub fn to_svg_with(&self, options: &SvgOptions) -> String {
        DebugScene::from_solution(self, options).to_svg()
    }
}

/// Render several solutions stacked vertically with captions and divider
/// lines — the before/after gallery format.
pub fn svg_panels<Id: Display>(panels: &[(&str, &LayoutSolution<Id>)]) -> String {
    const CAPTION: f32 = 16.0;
    const GAP: f32 = 20.0;

    let mut combined = DebugScene {
        content_size: Size::default(),
        draw_bounds: false,
        regions: Vec::new(),
        markers: Vec::new(),
        dividers: Vec::new(),
    };
    let mut y = 0.0f32;
    let mut max_width = 0.0f32;
    for (index, (caption, solution)) in panels.iter().enumerate() {
        if index > 0 {
            combined.dividers.push(y - GAP / 2.0);
        }
        combined.regions.push(DebugRegion {
            label: (*caption).to_string(),
            kind: DebugRegionKind::Content,
            content: Rect::new(0.0, y, 0.0, 0.0),
            requested: None,
            target: None,
            label_anchor: Some([0.0, y + 11.0]),
            label_rotated: false,
            depth: 0,
        });
        y += CAPTION;

        let mut scene = DebugScene::from_solution(solution, &SvgOptions::default());
        // Each panel carries its own bounds, drawn after its regions.
        scene.regions.push(DebugRegion {
            label: String::new(),
            kind: DebugRegionKind::Bounds,
            content: Rect::new(
                0.0,
                0.0,
                scene.content_size.width,
                scene.content_size.height,
            ),
            requested: None,
            target: None,
            label_anchor: None,
            label_rotated: false,
            depth: 0,
        });
        let panel_size = scene.content_size;
        combined.embed(scene, [0.0, y]);
        y += panel_size.height + GAP;
        max_width = max_width.max(panel_size.width);
    }
    combined.content_size = Size::new(max_width, (y - GAP).max(0.0));
    combined.to_svg()
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

/// Shade ramps shared by frame chrome and demand strips: index 0 is the
/// base (frame) layer, tree depth `d` uses index `d + 1`; dark at the base,
/// lighter as nesting deepens (clamped).
const GUIDE_SHADES: [&str; 4] = ["#9f2222", "#cf4444", "#e98080", "#f7bcbc"];
const LEGEND_SHADES: [&str; 4] = ["#14602f", "#2f9c5c", "#6cc795", "#b2e6c9"];

/// Rendered style per region kind.
fn kind_style(kind: DebugRegionKind) -> &'static str {
    match kind {
        DebugRegionKind::Content => {
            "fill=\"#dbeafe\" fill-opacity=\"0.6\" stroke=\"#9ca3af\" stroke-width=\"0.5\" stroke-opacity=\"0.7\""
        }
        DebugRegionKind::Margin => {
            "fill=\"#e5e7eb\" fill-opacity=\"0.6\" stroke=\"#e5e7eb\" stroke-width=\"0.5\" stroke-opacity=\"0.6\""
        }
        DebugRegionKind::Strip => {
            "fill=\"#fde68a\" fill-opacity=\"0.6\" stroke=\"#fde68a\" stroke-width=\"0.5\" stroke-opacity=\"0.6\""
        }
        // LEGEND_SHADES[0]: the base step of the legend ramp.
        DebugRegionKind::Legend => "fill=\"#14602f\" stroke=\"#14602f\" stroke-width=\"0.5\"",
        // GUIDE_SHADES[0]: the base step of the guide ramp.
        DebugRegionKind::Guide => "fill=\"#9f2222\" stroke=\"#9f2222\" stroke-width=\"0.5\"",
        DebugRegionKind::Slot => {
            "fill=\"none\" stroke=\"#6b7280\" stroke-width=\"0.7\" stroke-dasharray=\"3 2\""
        }
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

    #[test]
    fn solution_scene_renders_slabs_demands_and_slot_outlines() {
        use crate::build::{Layout, SolveFor, SolveOptions};
        use crate::region::EdgeDemand as Demand;

        // A chromed canvas containing a row of two leaves with different
        // top demands: slabs, hatched grants, and a slot outline (second
        // leaf narrower than its track after a sibling stretches it) all
        // appear.
        let root: Layout<&str> = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 60.0))
                .demand(
                    Side::Top,
                    Demand {
                        guide: 20.0,
                        legend: 0.0,
                    },
                )
                .id("a"),
            Layout::leaf(Size::new(60.0, 60.0))
                .demand(
                    Side::Top,
                    Demand {
                        guide: 8.0,
                        legend: 0.0,
                    },
                )
                .id("b"),
        ])
        .margin(8.0)
        .guide(Side::Left, 30.0)
        .sizing(SolveFor::Content)
        .id("fig");
        let solved = root
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(200.0),
            })
            .expect("solve");

        let svg = solved.to_svg();
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("url(#hatch-g1)"), "granted strips hatch");
        assert!(svg.contains("stroke-dasharray=\"3 2\""), "slot outline");
        assert!(svg.contains(">a</text>"), "id labels render");
        assert!(svg.contains(">slot</text>"), "key explains the slot");
        assert!(svg.contains(">margin</text>"), "key explains slabs");

        let unlabeled = solved.to_svg_with(&SvgOptions { labels: false });
        assert!(!unlabeled.contains(">a</text>"));

        let panels = svg_panels(&[("before", &solved), ("after", &solved)]);
        assert!(panels.contains(">before</text>"));
        assert!(panels.contains(">after</text>"));
        assert!(panels.contains("<line "), "divider between panels");
    }
}
