//! Colorbars: a title above a gradient with an axis beside it, prepared once and drawn at any
//! length.

use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::types::{FontWeight, TextSyntaxMode};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark};
use avenger_typst_label::LabelEngine;

use super::{make_background, make_title, title_group, Title, TitleFont};
use crate::{
    axis::{Axis, AxisConfig, AxisPart, AxisStyle},
    error::AvengerGuidesError,
    guide::{excess, fits},
    BackgroundStyle, GuideSide, PartExtent,
};

/// Ticks along horizontal colorbars, unless the axis style sets a count.
const HORIZONTAL_TICK_COUNT: f64 = 10.0;
/// How far end labels may reach into the background's padding.
const LABEL_EDGE_MARGIN: f32 = 2.0;
/// Gradient segments between the color scale's stops.
const GRADIENT_SEGMENTS: usize = 10;
/// The most times the gradient shortens to make room for its end labels, whose tick set can
/// change with the gradient's length.
const FIT_STEPS: usize = 4;

/// A colorbar's settings: its axis, the gradient's thickness, the space between them, and the
/// background.
#[derive(Debug, Clone)]
pub struct ColorbarConfig {
    /// The colorbar's side, where its axis lies beside the gradient, its ticks and labels, and
    /// the title's visibility, markup and font, which the axis style's title settings give.
    pub axis: AxisConfig,
    /// The gradient's thickness, across the colorbar.
    pub thickness: f32,
    /// The space between the gradient and its axis.
    pub axis_gap: f32,
    /// The box behind the colorbar.
    pub background: BackgroundStyle,
}

impl Default for ColorbarConfig {
    /// A colorbar on the right of the plot, with the legends' fonts.
    fn default() -> Self {
        Self {
            axis: AxisConfig {
                side: GuideSide::Right,
                tick_format: String::new(),
                tick_label: None,
                style: AxisStyle {
                    label_color: [0.235, 0.235, 0.235, 1.0],
                    label_font_size: 11.0,
                    label_font_weight: FontWeight(300),
                    title_color: [0.173, 0.173, 0.173, 1.0],
                    title_syntax: TextSyntaxMode::Plain,
                    ..Default::default()
                },
            },
            thickness: 15.0,
            axis_gap: 5.0,
            background: BackgroundStyle::default(),
        }
    }
}

/// A drawn colorbar, with the gradient that interactions can target.
#[derive(Debug, Clone)]
pub struct ColorbarOutput {
    pub group: SceneGroup,
    /// The scene path of the gradient rect, which takes the colorbar's hits, relative to `group`.
    pub gradient_path: Vec<usize>,
    /// The gradient's [x, y, width, height] relative to `group`.
    pub gradient_bounds: [f32; 4],
}

/// A colorbar prepared from a color scale, a title, a configuration and a text engine, without
/// a length.
///
/// A colorbar is one part: its title at the top, and below it the gradient with a continuous
/// [`Axis`] beside it, inside its background. Every call takes the colorbar's whole length along
/// its side, background included, and the gradient takes what the padding, the title and its
/// end labels leave, so that the end labels stay inside the background. The constructor checks
/// the formats, the template and the title, so the methods do not fail.
#[derive(Debug)]
pub struct Colorbar {
    axis: Axis,
    side: GuideSide,
    thickness: f32,
    axis_gap: f32,
    background: BackgroundStyle,
    title: Option<Title>,
    stops: Vec<GradientStop>,
}

/// Where everything lies at one length, in the colorbar's frame with its top left corner at 0.
struct Placed {
    /// The gradient's length.
    gradient: f32,
    /// Where the body's inner edge meets the gradient's start: the origin of the gradient, the
    /// ticks and the labels.
    body_origin: [f32; 2],
    /// The background's size.
    size: [f32; 2],
    extent: PartExtent,
}

impl Colorbar {
    /// A colorbar for a color scale with a numeric or temporal domain. It fails where its
    /// axis's formats or template fail, where the title does not typeset, or where the scale
    /// has no color range.
    pub fn new(
        scale: &ConfiguredScale,
        title: &str,
        config: &ColorbarConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let side = config.axis.side;
        let mut axis_config = config.axis.clone();
        if !side.is_vertical() && axis_config.style.tick_count.is_none() {
            axis_config.style.tick_count = Some(HORIZONTAL_TICK_COUNT);
        }
        // The gradient runs left to right, or bottom to top. Each call sets the range's length.
        let range = if side.is_vertical() {
            (1.0, 0.0)
        } else {
            (0.0, 1.0)
        };
        // The colorbar draws the title itself, above the gradient.
        let axis = Axis::continuous(
            scale.clone().with_range_interval(range),
            "",
            &axis_config,
            engine,
        )?;
        let style = &config.axis.style;
        let title = if style.title_visible && !title.is_empty() {
            let font = TitleFont {
                family: &style.title_font_family,
                size: style.title_font_size,
                weight: style.title_font_weight,
                color: style.title_color,
                syntax: style.title_syntax,
            };
            Some(make_title(title, &font, engine)?)
        } else {
            None
        };
        Ok(Self {
            axis,
            side,
            thickness: config.thickness,
            axis_gap: config.axis_gap,
            background: config.background.clone(),
            title,
            stops: scale.color_range_as_gradient_stops(GRADIENT_SEGMENTS)?,
        })
    }

    /// The colorbar's thickness and overhang at `length`, its whole length along its side, and
    /// the lengths where they hold. A colorbar that fits has no overhang, and a length too short
    /// for its padding, title and end labels reports the excess as its end overhang. The answer
    /// holds where the labels hold, at `length` only, unless the labels are hidden.
    pub fn extent(&self, length: f32) -> PartExtent {
        self.placed(length).extent
    }

    /// The origin for the colorbar in the slab with rectangle `slot`, `[x, y, width, height]`:
    /// the colorbar's inner edge, nearest the plot, lies on the slab's inner edge, at its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        let length = if self.side.is_vertical() {
            slot[3]
        } else {
            slot[2]
        };
        self.side.box_origin(slot, self.extent(length).thickness)
    }

    /// The color scale's domain mapped onto the gradient of a colorbar of `length`, in the
    /// colorbar's frame, for interactions such as a brush on the gradient.
    pub fn scale(&self, length: f32) -> ConfiguredScale {
        let placed = self.placed(length);
        let gradient = placed.gradient;
        let range = if self.side.is_vertical() {
            let start = placed.body_origin[1];
            (start + gradient, start)
        } else {
            let start = placed.body_origin[0];
            (start, start + gradient)
        };
        self.axis
            .scale(gradient)
            .with_range_interval((f64::from(range.0), f64::from(range.1)))
    }

    /// The colorbar at `length`, in the frame that [`Colorbar::origin`] places: its background,
    /// its title, and its gradient with the axis's domain line, ticks and labels. The gradient
    /// takes the colorbar's hits, and the rest takes none.
    pub fn marks(&self, length: f32) -> ColorbarOutput {
        let placed = self.placed(length);
        let padding = self.background.padding;
        let mut marks = vec![make_background(
            &self.background,
            placed.size[0],
            placed.size[1],
        )];
        let mut body = self.body(placed.gradient);
        body.origin = placed.body_origin;
        marks.push(body.into());
        if let Some(title) = &self.title {
            marks.push(title_group(title, padding));
        }
        let [x, y, width, height] = self.gradient_rect(placed.gradient);
        ColorbarOutput {
            group: SceneGroup {
                marks,
                ..Default::default()
            },
            gradient_path: vec![1, 0],
            gradient_bounds: [
                placed.body_origin[0] + x,
                placed.body_origin[1] + y,
                width,
                height,
            ],
        }
    }

    fn placed(&self, length: f32) -> Placed {
        let padding = self.background.padding;
        let (title_width, title_block) = self
            .title
            .as_ref()
            .map_or((0.0, 0.0), |title| (title.width, title.block));
        let vertical = self.side.is_vertical();
        // Beside the plot, the title stands above the gradient, along it. Above or below the
        // plot, it stands across it.
        let before = if vertical { title_block } else { 0.0 };
        let available = (length - 2.0 * padding - before).max(0.0);
        let (gradient, insets, body) = self.fit_gradient(available);
        let along = before + insets[0] + gradient + insets[1];
        let (content_along, content_across) = if vertical {
            (along, body.thickness.max(title_width))
        } else {
            (along.max(title_width), title_block + body.thickness)
        };
        let background_along = content_along + 2.0 * padding;
        let thickness = content_across + 2.0 * padding;
        let overhang = excess(background_along, length);
        let labels_vary = body.holds.start() > &0.0 || body.holds.end().is_finite();
        let holds = if overhang > 0.0 || labels_vary {
            length..=length
        } else {
            // Without labels, only the padding and the title need room.
            let shortest = 2.0 * padding + if vertical { title_block } else { title_width };
            shortest.min(length)..=f32::INFINITY
        };
        let start = padding + before + insets[0];
        let inner = match self.side {
            GuideSide::Right => padding,
            GuideSide::Left => padding + content_across,
            GuideSide::Bottom => padding + title_block,
            GuideSide::Top => padding + title_block + body.thickness,
        };
        let (body_origin, size) = if vertical {
            ([inner, start], [thickness, background_along])
        } else {
            ([start, inner], [background_along, thickness])
        };
        Placed {
            gradient,
            body_origin,
            size,
            extent: PartExtent {
                thickness,
                overhang: [0.0, overhang],
                holds,
            },
        }
    }

    /// The longest gradient that leaves room within `available` for its end labels, with the
    /// room before and after it and the body's extent. Each tick set has its own end labels,
    /// so the gradient shortens until its labels fit, a few times at most.
    fn fit_gradient(&self, available: f32) -> (f32, [f32; 2], PartExtent) {
        let room = (self.background.padding - LABEL_EDGE_MARGIN).max(0.0);
        let insets = |body: &PartExtent| body.overhang.map(|overhang| (overhang - room).max(0.0));
        let mut gradient = available;
        let mut body = self.body_extent(gradient);
        for _ in 0..FIT_STEPS {
            let [start, end] = insets(&body);
            if fits(gradient + start + end, available) {
                break;
            }
            gradient = (available - start - end).max(0.0);
            body = self.body_extent(gradient);
        }
        (gradient, insets(&body), body)
    }

    /// The thickness and overhang of the gradient with its axis, for a gradient of `length`.
    fn body_extent(&self, length: f32) -> PartExtent {
        let ticks = self.axis.extent(AxisPart::Ticks, length);
        let labels = self.axis.extent(AxisPart::Labels, length);
        let holds = ticks.holds.start().max(*labels.holds.start())
            ..=ticks.holds.end().min(*labels.holds.end());
        PartExtent {
            thickness: self.axis_offset() + ticks.thickness + labels.thickness,
            overhang: [
                ticks.overhang[0].max(labels.overhang[0]),
                ticks.overhang[1].max(labels.overhang[1]),
            ],
            holds,
        }
    }

    /// The gradient with the axis's ticks and labels beside it, from the body's inner edge at
    /// the gradient's start. The gradient comes first and takes the hits, and the axis takes
    /// none.
    fn body(&self, length: f32) -> SceneGroup {
        let mut marks = vec![self.gradient(length)];
        let mut across = self.axis_offset();
        for part in [AxisPart::Ticks, AxisPart::Labels] {
            if let Some(mut group) = self.axis.part(part, length) {
                group.origin = self.side.point(0.0, across);
                group.interactive = Some(false);
                marks.push(group.into());
            }
            across += self.axis.extent(part, length).thickness;
        }
        SceneGroup {
            marks,
            ..Default::default()
        }
    }

    /// The axis's domain line, across the gradient and the gap from the body's inner edge.
    fn axis_offset(&self) -> f32 {
        self.thickness + self.axis_gap
    }

    /// The gradient's rect in the body's frame, `[x, y, width, height]`.
    fn gradient_rect(&self, length: f32) -> [f32; 4] {
        let [x, y] = self.side.point(0.0, 0.0);
        let [x2, y2] = self.side.point(length, self.thickness);
        [x.min(x2), y.min(y2), (x2 - x).abs(), (y2 - y).abs()]
    }

    /// The gradient along `length`, colored left to right, or bottom to top.
    fn gradient(&self, length: f32) -> SceneMark {
        let [x, y, width, height] = self.gradient_rect(length);
        // Gradient controls are in the unit box over the rect.
        let [x0, y0, x1, y1] = if self.side.is_vertical() {
            [0.0, 1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0, 0.0]
        };
        SceneMark::Rect(SceneRectMark {
            len: 1,
            gradients: vec![Gradient::LinearGradient(LinearGradient {
                x0,
                y0,
                x1,
                y1,
                stops: self.stops.clone(),
            })],
            x: x.into(),
            x2: Some((x + width).into()),
            y: y.into(),
            y2: Some((y + height).into()),
            fill: ColorOrGradient::GradientIndex(0).into(),
            clip: false,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Date32Array, Float64Array};
    use avenger_geometry::marks::{MarkGeometryUtils, TextGeometryUtils};
    use avenger_geometry::rtree::{EnvelopeUtils, SceneGraphRTree};
    use avenger_scales::scales::{linear::LinearScale, time::TimeScale};
    use avenger_scenegraph::{marks::text::SceneTextMark, scene_graph::SceneGraph};

    use super::*;
    use crate::{axis::AxisTicks, test_engine};

    const SIDES: [GuideSide; 4] = [
        GuideSide::Top,
        GuideSide::Bottom,
        GuideSide::Left,
        GuideSide::Right,
    ];

    fn color_scale() -> ConfiguredScale {
        LinearScale::configured_color((0.0, 100.0), ["#440154", "#fde725"])
    }

    fn config(side: GuideSide, tick_format: &str) -> ColorbarConfig {
        let defaults = ColorbarConfig::default();
        ColorbarConfig {
            axis: AxisConfig {
                side,
                tick_format: tick_format.into(),
                ..defaults.axis
            },
            thickness: 18.0,
            axis_gap: 4.0,
            ..defaults
        }
    }

    fn titled(
        scale: &ConfiguredScale,
        side: GuideSide,
        tick_format: &str,
        title: &str,
    ) -> Colorbar {
        Colorbar::new(scale, title, &config(side, tick_format), &test_engine()).expect("a colorbar")
    }

    fn colorbar(scale: &ConfiguredScale, side: GuideSide, tick_format: &str) -> Colorbar {
        titled(scale, side, tick_format, "temperature")
    }

    fn mark_at_path<'a>(marks: &'a [SceneMark], path: &[usize]) -> &'a SceneMark {
        let (first, rest) = path.split_first().expect("non-empty path");
        let mark = marks.get(*first).expect("mark at path segment");
        if rest.is_empty() {
            return mark;
        }
        let SceneMark::Group(group) = mark else {
            panic!("interior path segment should be a group");
        };
        mark_at_path(&group.marks, rest)
    }

    /// The tick labels, from the first text mark with several values.
    fn labels(group: &SceneGroup) -> &SceneTextMark {
        fn text(group: &SceneGroup) -> Option<&SceneTextMark> {
            group.marks.iter().find_map(|mark| match mark {
                SceneMark::Text(text) if text.len > 1 => Some(text.as_ref()),
                SceneMark::Group(group) => text(group),
                _ => None,
            })
        }
        text(group).expect("tick labels")
    }

    fn label_text(group: &SceneGroup) -> Vec<String> {
        let text = labels(group);
        text.text.as_vec(text.len as usize, None)
    }

    /// The background's size, `[width, height]`.
    fn background(output: &ColorbarOutput) -> [f32; 2] {
        let SceneMark::Rect(rect) = &output.group.marks[0] else {
            panic!("the background comes first");
        };
        let value = |values: &Option<avenger_common::value::ScalarOrArray<f32>>| {
            values.as_ref().unwrap().as_vec(1, None)[0]
        };
        [value(&rect.width), value(&rect.height)]
    }

    #[test]
    fn end_labels_are_the_outer_tick_labels() {
        let scale = LinearScale::configured_color((900_000.0, 1_100_000.0), ["#440154", "#fde725"]);
        let output = colorbar(&scale, GuideSide::Top, "s").marks(180.0);
        let text = label_text(&output.group);
        assert_eq!(
            (
                text.first().unwrap().as_str(),
                text.last().unwrap().as_str()
            ),
            ("0.90M", "1.10M")
        );

        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
        let scale = TimeScale::configured_color((start, end), vec!["#440154", "#fde725"]);
        let output = colorbar(&scale, GuideSide::Bottom, "").marks(200.0);
        let text = label_text(&output.group);
        assert_eq!(
            (
                text.first().unwrap().as_str(),
                text.last().unwrap().as_str()
            ),
            ("2024", "December")
        );
    }

    /// A colorbar's axis draws explicit ticks, and only the ones inside the color scale's domain.
    #[test]
    fn explicit_ticks_label_the_colorbar() {
        let mut config = config(GuideSide::Right, ",f");
        config.axis.style.ticks = Some(AxisTicks::Values(Arc::new(Float64Array::from(vec![
            10.0, 50.0, 90.0, 150.0,
        ]))));
        let colorbar =
            Colorbar::new(&color_scale(), "", &config, &test_engine()).expect("a colorbar");
        for length in [120.0, 300.0] {
            assert_eq!(
                label_text(&colorbar.marks(length).group),
                ["10", "50", "90"],
                "{length}"
            );
        }
    }

    /// The answer is the size of the drawn background: its thickness across the side, and how
    /// far it passes the length along the side. The background rounds to whole pixels.
    #[test]
    fn extents_equal_the_drawn_background() {
        for side in SIDES {
            for title in ["temperature", "", "a much longer temperature title"] {
                let colorbar = titled(&color_scale(), side, ",f", title);
                for length in (10..400).step_by(23).map(|length| length as f32) {
                    let extent = colorbar.extent(length);
                    let [width, height] = background(&colorbar.marks(length));
                    let (along, across) = if side.is_vertical() {
                        (height, width)
                    } else {
                        (width, height)
                    };
                    let case = format!("{side:?} {title:?} {length}");
                    assert!((extent.thickness - across).abs() <= 0.5, "{case}");
                    assert!(
                        (extent.overhang[1] - (along - length).max(0.0)).abs() <= 0.5,
                        "{case}: {:?} drawn {along}",
                        extent.overhang
                    );
                    assert_eq!(extent.overhang[0], 0.0, "{case}");
                    assert!(extent.holds.contains(&length), "{case}");
                }
            }
        }
    }

    /// Without labels, the answer holds from the shortest length that fits, and every length in
    /// that range gives the same answer.
    #[test]
    fn a_colorbar_without_labels_holds_at_every_length_that_fits() {
        for side in SIDES {
            let mut config = config(side, ",f");
            config.axis.style.labels_visible = false;
            let colorbar =
                Colorbar::new(&color_scale(), "temperature", &config, &test_engine()).unwrap();
            let extent = colorbar.extent(150.0);
            let (low, high) = (*extent.holds.start(), *extent.holds.end());
            assert_eq!(high, f32::INFINITY, "{side:?}");
            for other in [low, (low + 150.0) / 2.0, 1_000.0] {
                assert_eq!(
                    colorbar.extent(other),
                    PartExtent {
                        holds: colorbar.extent(other).holds,
                        ..extent.clone()
                    },
                    "{side:?} at {other}"
                );
            }
            assert!(colorbar.extent(low - 1.0).overhang[1] > 0.0, "{side:?}");
        }
    }

    /// The title sits at the top on every side, above the gradient, and a long title widens a
    /// colorbar beside the plot.
    #[test]
    fn the_title_is_at_the_top_on_every_side() {
        for side in SIDES {
            let colorbar = colorbar(&color_scale(), side, ",f");
            let padding = colorbar.background.padding;
            let block = colorbar.title.as_ref().unwrap().block;
            let output = colorbar.marks(220.0);
            let SceneMark::Group(title) = &output.group.marks[2] else {
                panic!("the title group");
            };
            assert_eq!(title.origin, [padding, padding], "{side:?}");
            assert!(output.gradient_bounds[1] >= padding + block, "{side:?}");
            let labels = labels(&output.group);
            let SceneMark::Group(body) = &output.group.marks[1] else {
                panic!("the body");
            };
            let ys = labels.y.as_vec(labels.len as usize, None);
            assert!(
                ys.iter()
                    .all(|y| body.origin[1] + y >= padding + block - 1e-3)
                    || side == GuideSide::Top,
                "{side:?}"
            );
        }
        for side in [GuideSide::Left, GuideSide::Right] {
            let short = titled(&color_scale(), side, ",f", "t").extent(200.0);
            let long = titled(
                &color_scale(),
                side,
                ",f",
                "a much longer temperature title",
            )
            .extent(200.0);
            assert!(long.thickness > short.thickness, "{side:?}");
        }
    }

    /// The scale maps each labeled tick to its drawn tick, in the colorbar's frame.
    #[test]
    fn the_scale_places_each_tick_at_its_drawn_tick() {
        for side in SIDES {
            let colorbar = colorbar(&color_scale(), side, ",f");
            let length = 220.0;
            let output = colorbar.marks(length);
            let SceneMark::Group(body) = &output.group.marks[1] else {
                panic!("the body");
            };
            let SceneMark::Group(ticks_part) = &body.marks[1] else {
                panic!("the ticks part");
            };
            let SceneMark::Rule(ticks) = &ticks_part.marks[1] else {
                panic!("the ticks");
            };
            let k = usize::from(side.is_vertical());
            let positions = [&ticks.x, &ticks.y][k].as_vec(ticks.len as usize, None);
            let values = label_text(&output.group);
            assert_eq!(values.len(), positions.len(), "{side:?}");
            let scale = colorbar.scale(length);
            for (label, position) in values.iter().zip(positions) {
                let value: f64 = label.parse().unwrap();
                let expected = scale.scale_scalar(&value).unwrap().as_f64().unwrap() as f32;
                let drawn = body.origin[k] + ticks_part.origin[k] + position;
                assert!(
                    (expected - drawn).abs() < 1e-3,
                    "{side:?} {label}: {expected} drawn {drawn}"
                );
            }
        }
    }

    /// End labels stay inside the background on every side.
    #[test]
    fn end_labels_stay_inside_the_background() {
        let scale = LinearScale::configured_color((0.0, 1_000_000.0), ["#440154", "#fde725"]);
        let engine = test_engine();
        for side in SIDES {
            let mut config = config(side, ",f");
            config.axis.style.label_font_size = 16.0;
            let output = Colorbar::new(&scale, "title", &config, &engine)
                .unwrap()
                .marks(240.0);
            let [width, height] = background(&output);
            let body = output.group.marks[1].bounding_box(&engine);
            let k = usize::from(side.is_vertical());
            let along = [width, height][k];
            assert!(
                body.lower()[k] >= 0.0 && body.upper()[k] <= along,
                "{side:?}: labels span {:?} in a {along} px background",
                [body.lower()[k], body.upper()[k]]
            );
        }
    }

    #[test]
    fn the_gradient_path_resolves_to_the_gradient_rect_at_its_bounds() {
        for side in SIDES {
            let output = colorbar(&color_scale(), side, ",f").marks(200.0);
            let gradient_mark = mark_at_path(&output.group.marks, &output.gradient_path);
            assert_eq!(gradient_mark.interactive(), None);
            let SceneMark::Rect(rect) = gradient_mark else {
                panic!("colorbar hit path should resolve to the gradient rect");
            };
            let SceneMark::Group(body) = &output.group.marks[1] else {
                panic!("the body");
            };
            let rect_bounds = rect.bounding_box();
            assert_eq!(
                rect_bounds.lower()[0] + body.origin[0],
                output.gradient_bounds[0]
            );
            assert_eq!(
                rect_bounds.lower()[1] + body.origin[1],
                output.gradient_bounds[1]
            );
            assert_eq!(rect_bounds.width(), output.gradient_bounds[2]);
            assert_eq!(rect_bounds.height(), output.gradient_bounds[3]);
        }
    }

    #[test]
    fn hits_land_on_the_gradient_but_not_on_the_chrome() {
        let output = colorbar(&color_scale(), GuideSide::Right, ",f").marks(200.0);
        let bounds = output.gradient_bounds;
        let scene = SceneGraph {
            marks: output.group.marks.clone(),
            width: output.group.bounding_box(&test_engine()).width(),
            height: output.group.bounding_box(&test_engine()).height(),
            origin: output.group.origin,
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &test_engine());

        let gradient_point = [bounds[0] + bounds[2] * 0.5, bounds[1] + bounds[3] * 0.5];
        let pick = |point| {
            let topmost = avenger_geometry::query::Query::new(
                avenger_geometry::query::QueryShape::Point(point),
                avenger_geometry::query::Relation::Intersects,
            );
            (rtree.query(&topmost).first()).map(|instance| instance.mark_instance.clone())
        };
        let hit = pick(gradient_point).expect("gradient should be interactive");
        assert_eq!(hit.mark_path, output.gradient_path);

        let background_point = [1.0, 1.0];
        assert!(
            pick(background_point).is_none(),
            "transparent background/chrome should remain noninteractive"
        );
    }
}
