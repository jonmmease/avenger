//! Axes prepared once, without a length, that size and draw their parts at any length.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use arrow::array::{Array, ArrayRef};
use avenger_scales::scales::{band::BandScale, fitting::FittingScale, ConfiguredScale};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_typst_label::LabelEngine;

use super::{
    band::{self, category_text},
    continuous::{
        self, axis_ticks, continuous_tick_text, min_tick_spacing, tick_count, values_in_range,
    },
    marks::{
        domain_mark, grid_group, label_mark, label_reach, separators_mark, thin_row, ticks_mark,
        title_mark, title_reach, LabelRow, Labels, Separators, Title, PIXEL_OFFSET, TEXT_MARGIN,
    },
    nested_band::{NestedBandAxisLevelConfig, NestedLevelRow, NestedLevels},
    opts::{AxisConfig, AxisTicks},
    tick_label_text,
};
use crate::{
    error::AvengerGuidesError,
    guide::{PartExtent, Reach},
    GuideSide,
};

/// The most tick sets that a continuous axis keeps, with their formatted and measured labels.
const KEPT_TICK_SETS: usize = 8;

/// One part of an axis, which a layout measures and a chart draws in its own slab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisPart {
    /// The domain line and the ticks.
    Ticks,
    /// The tick labels, with a nested band axis's label rows and separators.
    Labels,
    /// The axis title, past the labels, centered on the axis and turned beside a vertical one.
    Title,
}

/// An axis prepared from a scale, a title, a configuration and a text engine, without a length.
///
/// Every method takes the axis's length, or the plot's size, and sets the scale's range to it,
/// so the sizes, the drawing and the data's positions agree at every length. A [`FittingScale`]
/// with a summary also fits its domain, or the clip padding of a band or point scale, at each
/// length, and a `ConfiguredScale` keeps its domain. The constructors check the formats, the
/// templates and the title, so the methods do not fail.
#[derive(Debug)]
pub struct Axis {
    /// The scale as the caller gave it, which [`Axis::scale`] fits for the chart's data.
    scale: FittingScale,
    /// Whether the scale's range runs from high to low, as a y scale's does.
    reversed: bool,
    kind: Kind,
    config: AxisConfig,
    /// The measured title, or `None` when it is hidden or empty.
    title: Option<Title>,
    title_margin: f32,
}

#[derive(Debug)]
enum Kind {
    Continuous(Box<ContinuousTicks>),
    /// A band axis, and a point axis as a band axis: a tick and a label at the center of each
    /// band that it draws. The categories don't change with the length, so the labels are
    /// measured once.
    Band {
        /// Whether the scale is a point scale, which places ticks through its band form.
        point: bool,
        /// The categories that the axis draws: the domain, or the explicit values in it.
        categories: ArrayRef,
        labels: Arc<Labels>,
    },
    Nested(NestedLevels),
}

#[derive(Debug)]
struct ContinuousTicks {
    /// Whether the scale has a band option, so that ticks center in the band.
    band: bool,
    min_spacing: f32,
    engine: LabelEngine,
    /// The most recent tick sets, newest first.
    kept: Mutex<VecDeque<Arc<TickSet>>>,
}

/// The ticks that one tick count labels, with their labels.
#[derive(Debug)]
struct TickSet {
    labeled: ArrayRef,
    labels: Arc<Labels>,
}

/// What an axis shows at one length, in its parts' frames.
struct Drawn {
    length: f32,
    ticks: Vec<f32>,
    ticks_visible: bool,
    rows: Vec<NestedLevelRow>,
}

impl Axis {
    /// An axis for a scale with a numeric or temporal domain, such as a linear, log or time
    /// scale, with tick labels that the text engine formats with `config.tick_format`.
    pub fn continuous(
        scale: impl Into<FittingScale>,
        title: &str,
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let scale = scale.into();
        // For scales with a band option, ticks center in the band. Other scales reject unknown
        // options.
        let band = scale
            .scale()
            .scale_impl
            .option_definitions()
            .iter()
            .any(|option| option.name == "band");
        let ticks = ContinuousTicks {
            band,
            min_spacing: min_tick_spacing(config.side, &config.style, engine),
            engine: engine.clone(),
            kept: Mutex::new(VecDeque::new()),
        };
        // Label the ticks at the scale's own length once, so that a format, a timezone or a
        // template that fails is the constructor's error.
        let (length, fitted) = configured_fit(&scale)?;
        ticks.labels(config, &ticks.tick_scale(&fitted), length)?;
        Self::new(
            scale,
            Kind::Continuous(Box::new(ticks)),
            title,
            continuous::TITLE_MARGIN,
            config,
            engine,
        )
    }

    /// An axis for a band scale, with a tick and a label at the center of each band. The text
    /// engine formats numeric and temporal categories with `config.tick_format`, each on its
    /// own, and other categories show as text.
    pub fn band(
        scale: impl Into<FittingScale>,
        title: &str,
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        Self::banded(scale.into(), false, title, config, engine)
    }

    /// An axis for a point scale, with a tick and a label at each point.
    pub fn point(
        scale: impl Into<FittingScale>,
        title: &str,
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        Self::banded(scale.into(), true, title, config, engine)
    }

    /// An axis for a nested band scale. `levels` overrides the axis's settings for each level,
    /// outermost first, and levels past its end keep the defaults.
    pub fn nested_band(
        scale: impl Into<FittingScale>,
        title: &str,
        levels: &[NestedBandAxisLevelConfig],
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let scale = scale.into();
        let (_, fitted) = configured_fit(&scale)?;
        let levels = NestedLevels::new(&fitted, title, config, levels, engine)?;
        let title = levels.title.clone();
        Self::new(
            scale,
            Kind::Nested(levels),
            &title,
            band::TITLE_MARGIN,
            config,
            engine,
        )
    }

    fn banded(
        scale: FittingScale,
        point: bool,
        title: &str,
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let (_, fitted) = configured_fit(&scale)?;
        let tick_scale = band_ticks(&fitted, point);
        let categories = match &config.style.ticks {
            Some(AxisTicks::Values(values)) => values_in_range(&tick_scale, values)?,
            _ => tick_scale.domain().clone(),
        };
        let labels = if config.style.labels_visible {
            let (text, syntax) = category_text(
                &categories,
                &config.tick_format,
                config.tick_label.as_deref(),
                engine,
            )?;
            Labels::measure(text, syntax, &config.style, engine)
        } else {
            Labels::empty()
        };
        let kind = Kind::Band {
            point,
            categories,
            labels: Arc::new(labels),
        };
        Self::new(scale, kind, title, band::TITLE_MARGIN, config, engine)
    }

    fn new(
        scale: FittingScale,
        kind: Kind,
        title: &str,
        title_margin: f32,
        config: &AxisConfig,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let (start, end) = scale.scale().numeric_interval_range()?;
        let style = &config.style;
        let title = if style.title_visible && !title.is_empty() {
            let mark = title_mark(title, 0.0, 0.0, config.side, style);
            let bounds = mark
                .labels()
                .map(|label| engine.bounds(&label.label))
                .next();
            bounds.transpose()?.map(|bounds| Title {
                text: title.to_string(),
                bounds,
            })
        } else {
            None
        };
        Ok(Self {
            scale,
            reversed: end < start,
            kind,
            config: config.clone(),
            title,
            title_margin,
        })
    }

    /// The part's thickness and overhang at `length`, and the lengths where they hold.
    ///
    /// The ticks part holds at every length. The labels part holds only at `length`, since its
    /// ticks and its end labels move with the length. The title holds from its own length up,
    /// where it reaches past neither end, and otherwise only at `length`.
    pub fn extent(&self, part: AxisPart, length: f32) -> PartExtent {
        match part {
            AxisPart::Ticks => PartExtent {
                thickness: self.ticks_thickness(),
                overhang: [PIXEL_OFFSET; 2],
                holds: 0.0..=f32::INFINITY,
            },
            // Hidden labels draw nothing at any length, unless a nested level shows its row.
            AxisPart::Labels
                if !self.config.style.labels_visible && !matches!(self.kind, Kind::Nested(_)) =>
            {
                PartExtent::empty()
            }
            AxisPart::Labels => {
                let reach = self.labels_reach(&self.drawn(length));
                PartExtent {
                    thickness: reach.across.max(0.0),
                    overhang: reach.overhang(),
                    holds: length..=length,
                }
            }
            AxisPart::Title => {
                let Some(title) = &self.title else {
                    return PartExtent::empty();
                };
                let reach = title_reach(title, length, self.title_margin, self.config.side);
                let fits = reach.start <= 0.0 && reach.end <= 0.0;
                let holds = if fits {
                    // The title is as long as `length` less its room at each end.
                    (length + reach.start + reach.end)..=f32::INFINITY
                } else {
                    length..=length
                };
                PartExtent {
                    thickness: reach.across.max(0.0),
                    overhang: [reach.start.max(0.0), reach.end.max(0.0)],
                    holds,
                }
            }
        }
    }

    /// The part at `length`, in its slab's frame, or `None` for hidden labels or a hidden or
    /// empty title.
    ///
    /// Along the axis, 0 is the start of the part's slab. Across it, 0 is the part's inner edge,
    /// nearest the plot, and the part extends outward: to positive coordinates beside a bottom
    /// or right axis, and to negative ones beside a top or left axis.
    pub fn part(&self, part: AxisPart, length: f32) -> Option<SceneGroup> {
        let side = self.config.side;
        let style = &self.config.style;
        let marks = match part {
            AxisPart::Ticks => {
                let drawn = self.drawn(length);
                let mut marks = vec![domain_mark(length, side, style).into()];
                if drawn.ticks_visible {
                    marks.push(ticks_mark(&drawn.ticks, side, style).into());
                }
                marks
            }
            AxisPart::Labels => {
                let drawn = self.drawn(length);
                if drawn.rows.is_empty() {
                    return None;
                }
                let mut marks = Vec::new();
                for level in &drawn.rows {
                    marks.push(label_mark(&level.row, side, style).into());
                    if let Some(separators) = &level.separators {
                        marks.push(separators_mark(separators, side, style).into());
                    }
                }
                marks
            }
            AxisPart::Title => {
                let title = self.title.as_ref()?;
                vec![title_mark(&title.text, length, self.title_margin, side, style).into()]
            }
        };
        Some(SceneGroup {
            marks,
            ..Default::default()
        })
    }

    /// The origin for a part in the slab with rectangle `slot`, `[x, y, width, height]`: where
    /// the slab's inner edge, nearest the plot, meets its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        self.config.side.slot_origin(slot)
    }

    /// The scale with its range set for an axis of `length`, in the direction that the scale's
    /// range had, and its domain fitted there, for the chart's data. A fit that fails at this
    /// length, as at a length of 0, keeps the configured domain.
    pub fn scale(&self, length: f32) -> ConfiguredScale {
        match self.scale.at(length) {
            Ok(at) => at.scale,
            Err(_) => self
                .scale
                .scale()
                .clone()
                .with_range_interval(self.range(length)),
        }
    }

    /// Grid lines at the ticks across a plot of `size`, in the plot's frame, behind its data.
    pub fn grid(&self, size: [f32; 2]) -> SceneGroup {
        let side = self.config.side;
        let length = self.length_of(size);
        grid_group(&self.drawn(length).ticks, side, size, &self.config.style)
    }

    /// The three parts stacked in the frame of a plot of `size`, with its top-left corner at 0,
    /// for a chart without a layout.
    pub fn marks(&self, size: [f32; 2]) -> SceneGroup {
        let side = self.config.side;
        let length = self.length_of(size);
        // The domain line lies on the plot's edge on the axis's side.
        let edge = match side {
            GuideSide::Left | GuideSide::Top => [0.0, 0.0],
            GuideSide::Right => [size[0], 0.0],
            GuideSide::Bottom => [0.0, size[1]],
        };
        let mut across = 0.0;
        let mut marks = Vec::new();
        for part in [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title] {
            if let Some(mut group) = self.part(part, length) {
                let offset = side.point(0.0, across);
                group.origin = [edge[0] + offset[0], edge[1] + offset[1]];
                marks.push(group.into());
            }
            across += self.extent(part, length).thickness;
        }
        SceneGroup {
            zindex: Some(1),
            marks,
            ..Default::default()
        }
    }

    /// The axis's length in a plot of `size`.
    fn length_of(&self, size: [f32; 2]) -> f32 {
        if self.config.side.is_vertical() {
            size[1]
        } else {
            size[0]
        }
    }

    fn range(&self, length: f32) -> (f64, f64) {
        let length = f64::from(length);
        if self.reversed {
            (length, 0.0)
        } else {
            (0.0, length)
        }
    }

    fn ticks_thickness(&self) -> f32 {
        let visible = match &self.kind {
            Kind::Nested(levels) => levels.ticks_visible(),
            _ => true,
        };
        if visible {
            self.config.style.tick_length
        } else {
            0.0
        }
    }

    /// The ticks and label rows at `length`, as the axis draws them: a continuous, band, or point
    /// axis drops the labels that overlap, as its style's label thinning chooses.
    fn drawn(&self, length: f32) -> Drawn {
        let mut drawn = self.every_label(length);
        if !matches!(self.kind, Kind::Nested(_)) {
            for level in &mut drawn.rows {
                thin_row(&mut level.row, self.config.side, &self.config.style);
            }
        }
        drawn
    }

    /// The ticks and label rows at `length`, with every label.
    fn every_label(&self, length: f32) -> Drawn {
        let scale = self.scale(length);
        let positions = |scale: &ConfiguredScale, values: &ArrayRef| -> Vec<f32> {
            // The constructor placed the same scale's ticks, so this succeeds.
            scale
                .scale_to_numeric(values)
                .map(|positions| positions.as_vec(values.len(), None))
                .unwrap_or_default()
        };
        let row = |labels: Arc<Labels>, positions: Vec<f32>| NestedLevelRow {
            row: LabelRow {
                labels,
                positions,
                distance: TEXT_MARGIN,
                angle: self.config.style.label_angle,
            },
            separators: None::<Separators>,
        };
        let labels_visible = self.config.style.labels_visible;
        match &self.kind {
            Kind::Continuous(continuous) => {
                let tick_scale = continuous.tick_scale(&scale);
                let (ticks, set) = continuous.at(&self.config, &tick_scale, length);
                let rows = match set.filter(|_| labels_visible) {
                    Some(set) => {
                        let labeled = positions(&tick_scale, &set.labeled);
                        vec![row(set.labels.clone(), labeled)]
                    }
                    None => Vec::new(),
                };
                Drawn {
                    length,
                    ticks: positions(&tick_scale, &ticks),
                    ticks_visible: true,
                    rows,
                }
            }
            Kind::Band {
                point,
                categories,
                labels,
            } => {
                let tick_scale = band_ticks(&scale, *point);
                let centers = positions(&tick_scale, categories);
                let rows = if labels_visible {
                    vec![row(labels.clone(), centers.clone())]
                } else {
                    Vec::new()
                };
                Drawn {
                    length,
                    ticks: centers,
                    ticks_visible: true,
                    rows,
                }
            }
            Kind::Nested(levels) => {
                let range = self.range(length);
                let range = (range.0 as f32, range.1 as f32);
                let (ticks, rows) = levels.at(&scale, range, self.ticks_thickness());
                Drawn {
                    length,
                    ticks,
                    ticks_visible: levels.ticks_visible(),
                    rows,
                }
            }
        }
    }

    /// How far the label rows and separators reach in the labels part's frame.
    fn labels_reach(&self, drawn: &Drawn) -> Reach {
        let side = self.config.side;
        let mut reach = Reach::default();
        for level in &drawn.rows {
            label_reach(
                &mut reach,
                &level.row,
                side,
                drawn.length,
                &self.config.style,
            );
            if let Some(separators) = &level.separators {
                reach.across = reach.across.max(separators.outer);
            }
        }
        reach
    }
}

/// The scale fitted at the length of its configured range, with that length, which the
/// constructors use for the checks and the measurements that don't depend on the length.
fn configured_fit(scale: &FittingScale) -> Result<(f32, ConfiguredScale), AvengerGuidesError> {
    let (start, end) = scale.scale().numeric_interval_range()?;
    let length = (end - start).abs() as f32;
    Ok((length, scale.at(length)?.scale))
}

/// The scale that places a band or point axis's ticks and labels at band centers.
fn band_ticks(scale: &ConfiguredScale, point: bool) -> ConfiguredScale {
    let bands = if point {
        BandScale::from_point_scale(scale)
    } else {
        scale.clone()
    };
    bands.with_option("band", 0.5)
}

impl ContinuousTicks {
    /// The scale that places the ticks: `scale`, centered in the band when it has a band option.
    fn tick_scale(&self, scale: &ConfiguredScale) -> ConfiguredScale {
        if self.band {
            scale.clone().with_option("band", 0.5)
        } else {
            scale.clone()
        }
    }

    /// The drawn ticks of `tick_scale` at `length`, and the tick set that labels them. The tick
    /// set is `None` when the ticks cannot be computed, which the constructor rules out.
    fn at(
        &self,
        config: &AxisConfig,
        tick_scale: &ConfiguredScale,
        length: f32,
    ) -> (ArrayRef, Option<Arc<TickSet>>) {
        let count = tick_count(&config.style, length, self.min_spacing);
        let Ok((ticks, labeled)) = axis_ticks(tick_scale, &config.style, count) else {
            return (
                arrow::array::new_empty_array(&arrow::datatypes::DataType::Float64),
                None,
            );
        };
        let set = self.tick_set(config, tick_scale, labeled).ok();
        (ticks, set)
    }

    /// The labels of the tick set of `tick_scale` at `length`, which fails when the ticks do not
    /// label.
    fn labels(
        &self,
        config: &AxisConfig,
        tick_scale: &ConfiguredScale,
        length: f32,
    ) -> Result<(), AvengerGuidesError> {
        let count = tick_count(&config.style, length, self.min_spacing);
        let (_, labeled) = axis_ticks(tick_scale, &config.style, count)?;
        self.tick_set(config, tick_scale, labeled).map(|_| ())
    }

    /// The kept tick set for `labeled`, or a new one with its labels formatted and measured.
    /// Tick sets key on their labeled values, so one serves every domain that labels them.
    fn tick_set(
        &self,
        config: &AxisConfig,
        tick_scale: &ConfiguredScale,
        labeled: ArrayRef,
    ) -> Result<Arc<TickSet>, AvengerGuidesError> {
        {
            let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(set) = kept
                .iter()
                .find(|set| set.labeled.as_ref() == labeled.as_ref())
            {
                return Ok(set.clone());
            }
        }
        // Format and measure without the lock, which the text engine never needs.
        let labels = if config.style.labels_visible {
            let (text, syntax) = tick_label_text(
                continuous_tick_text(&labeled, tick_scale, &config.tick_format, &self.engine)?,
                config.tick_label.as_deref(),
            )?;
            Labels::measure(text, syntax, &config.style, &self.engine)
        } else {
            Labels::empty()
        };
        let set = Arc::new(TickSet {
            labeled,
            labels: Arc::new(labels),
        });
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.push_front(set.clone());
        kept.truncate(KEPT_TICK_SETS);
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, StringArray, StructArray};
    use avenger_geometry::marks::TextGeometryUtils;
    use avenger_scales::scales::{
        linear::LinearScale, log::LogScale, nested_band::NestedBandScale, point::PointScale,
    };
    use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
    use geo::BoundingRect;

    use super::*;
    use crate::axis::opts::AxisStyle;

    const SIDES: [GuideSide; 4] = [
        GuideSide::Top,
        GuideSide::Bottom,
        GuideSide::Left,
        GuideSide::Right,
    ];
    const PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];
    /// Longer than the shortest test axis, so that it reaches past both of its ends there.
    const TITLE: &str = "An axis title longer than a short axis";

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Linear,
        Log,
        Band,
        Point,
        Nested,
        /// A linear axis with explicit ticks, one of them outside its domain.
        LinearValues,
        /// A band axis with explicit categories, in another order than its domain's and with one
        /// that the domain does not hold.
        BandValues,
    }

    fn fruit() -> ArrayRef {
        Arc::new(StringArray::from(vec!["Apples", "Bananas", "Cherries"]))
    }

    fn axis(kind: Kind, side: GuideSide, angle: f32, title: &str) -> Axis {
        // A y scale's range runs up the screen.
        let range = if side.is_vertical() {
            (100.0, 0.0)
        } else {
            (0.0, 100.0)
        };
        let ticks = match kind {
            Kind::LinearValues => Some(AxisTicks::Values(Arc::new(
                arrow::array::Float64Array::from(vec![0.0, 30.0, 45.0, 90.0, 120.0]),
            ))),
            Kind::BandValues => Some(AxisTicks::Values(Arc::new(StringArray::from(vec![
                "Cherries", "Kiwis", "Apples",
            ])))),
            _ => None,
        };
        let config = AxisConfig {
            side,
            tick_format: ",f".into(),
            tick_label: None,
            style: AxisStyle {
                label_angle: angle,
                ticks,
                ..Default::default()
            },
        };
        let engine = crate::test_engine();
        match kind {
            Kind::Linear | Kind::LinearValues => Axis::continuous(
                LinearScale::configured((0.0, 90.0), range),
                title,
                &config,
                &engine,
            ),
            Kind::Log => Axis::continuous(
                LogScale::configured((1.0, 1000.0), range),
                title,
                &config,
                &engine,
            ),
            Kind::Band | Kind::BandValues => Axis::band(
                avenger_scales::scales::band::BandScale::configured(fruit(), range),
                title,
                &config,
                &engine,
            ),
            Kind::Point => Axis::point(
                PointScale::configured(fruit(), range),
                title,
                &config,
                &engine,
            ),
            Kind::Nested => {
                let columns: Vec<(&str, ArrayRef)> = vec![
                    (
                        "region",
                        Arc::new(StringArray::from(vec!["East", "East", "West"])),
                    ),
                    (
                        "channel",
                        Arc::new(StringArray::from(vec!["Online", "Retail", "Online"])),
                    ),
                ];
                let domain: ArrayRef = Arc::new(StructArray::try_from(columns).unwrap());
                Axis::nested_band(
                    NestedBandScale::configured(domain, range),
                    title,
                    &[],
                    &config,
                    &engine,
                )
            }
        }
        .unwrap_or_else(|error| panic!("{kind:?} {side:?}: {error}"))
    }

    /// How far the drawn part reaches, from its geometry without the margin that hit testing
    /// adds around it.
    fn drawn_reach(group: &SceneGroup, side: GuideSide, length: f32) -> Reach {
        let mut reach = Reach::default();
        for instance in group.geometry_iter(Vec::new(), [0.0, 0.0], &crate::test_engine()) {
            let rect = instance.geometry.bounding_rect().expect("a drawn shape");
            let (min, max) = (rect.min(), rect.max());
            for corner in [
                [min.x, min.y],
                [max.x, min.y],
                [max.x, max.y],
                [min.x, max.y],
            ] {
                reach.add(side, length, corner);
            }
        }
        reach
    }

    fn assert_extent_is_drawn(axis: &Axis, part: AxisPart, length: f32, case: &str) {
        let extent = axis.extent(part, length);
        let Some(group) = axis.part(part, length) else {
            assert_eq!(extent.thickness, 0.0, "{case}");
            assert_eq!(extent.overhang, [0.0; 2], "{case}");
            return;
        };
        let reach = drawn_reach(&group, axis.config.side, length);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(
            close(extent.thickness, reach.across.max(0.0)),
            "{case}: thickness {} drawn {}",
            extent.thickness,
            reach.across
        );
        assert!(
            close(extent.overhang[0], reach.start.max(0.0))
                && close(extent.overhang[1], reach.end.max(0.0)),
            "{case}: overhang {:?} drawn {:?}",
            extent.overhang,
            reach.overhang()
        );
    }

    #[test]
    fn extents_equal_the_drawn_parts() {
        for kind in [
            Kind::Linear,
            Kind::Log,
            Kind::Band,
            Kind::Point,
            Kind::Nested,
            Kind::LinearValues,
            Kind::BandValues,
        ] {
            let angles: Vec<f32> = match kind {
                Kind::Linear | Kind::Band => (0..360).step_by(15).map(|a| a as f32).collect(),
                _ => vec![0.0, 45.0, -90.0],
            };
            for side in SIDES {
                for &angle in &angles {
                    let axis = axis(kind, side, angle, TITLE);
                    for length in [60.0, 150.0, 400.0] {
                        for part in PARTS {
                            let case = format!("{kind:?} {side:?} {angle} {length} {part:?}");
                            assert_extent_is_drawn(&axis, part, length, &case);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_length_in_holds_has_the_same_answer() {
        for kind in [
            Kind::Linear,
            Kind::Band,
            Kind::Nested,
            Kind::LinearValues,
            Kind::BandValues,
        ] {
            for side in SIDES {
                let axis = axis(kind, side, 0.0, "Title");
                for part in PARTS {
                    for length in [80.0, 300.0] {
                        let extent = axis.extent(part, length);
                        let (low, high) = (*extent.holds.start(), *extent.holds.end());
                        assert!(
                            low <= length && length <= high,
                            "{kind:?} {side:?} {part:?}"
                        );
                        for other in [low, (low + length) / 2.0, length * 2.0, high.min(5_000.0)] {
                            if !extent.holds.contains(&other) {
                                continue;
                            }
                            let at_other = axis.extent(part, other);
                            assert_eq!(
                                at_other.thickness, extent.thickness,
                                "{kind:?} {side:?} {part:?} {other}"
                            );
                            assert_eq!(
                                at_other.overhang, extent.overhang,
                                "{kind:?} {side:?} {part:?} {other}"
                            );
                        }
                    }
                }
            }
        }
        // A title longer than the axis holds only where it was asked, and reaches past both ends.
        let axis = axis(Kind::Linear, GuideSide::Bottom, 0.0, TITLE);
        let short = axis.extent(AxisPart::Title, 60.0);
        assert_eq!(short.holds, 60.0..=60.0);
        assert!(short.overhang[0] > 0.0 && short.overhang[0] == short.overhang[1]);
    }

    /// Parts drawn at their origin in slabs that their extents size fill those slabs, from the
    /// slab's inner edge to its outer edge, on every side.
    #[test]
    fn parts_fill_slabs_that_their_extents_size() {
        let length = 200.0;
        for side in SIDES {
            let axis = axis(Kind::Band, side, 0.0, "Title");
            // The plot's edge on the axis's side, and the slabs stacked outward from it.
            let plot = [50.0, 50.0, 200.0, 200.0];
            let mut inner = match side {
                GuideSide::Left => plot[0],
                GuideSide::Right => plot[0] + plot[2],
                GuideSide::Top => plot[1],
                GuideSide::Bottom => plot[1] + plot[3],
            };
            for part in PARTS {
                let thickness = axis.extent(part, length).thickness;
                let slot = match side {
                    GuideSide::Left => [inner - thickness, plot[1], thickness, length],
                    GuideSide::Right => [inner, plot[1], thickness, length],
                    GuideSide::Top => [plot[0], inner - thickness, length, thickness],
                    GuideSide::Bottom => [plot[0], inner, length, thickness],
                };
                let mut group = axis.part(part, length).expect("a drawn part");
                group.origin = axis.origin(slot);
                let bounds = group
                    .geometry_iter(Vec::new(), [0.0, 0.0], &crate::test_engine())
                    .map(|instance| instance.geometry.bounding_rect().unwrap())
                    .reduce(|a, b| {
                        geo::Rect::new(
                            geo::coord! { x: a.min().x.min(b.min().x), y: a.min().y.min(b.min().y) },
                            geo::coord! { x: a.max().x.max(b.max().x), y: a.max().y.max(b.max().y) },
                        )
                    })
                    .unwrap();
                let outer = match side {
                    GuideSide::Left => bounds.min().x - slot[0],
                    GuideSide::Right => slot[0] + slot[2] - bounds.max().x,
                    GuideSide::Top => bounds.min().y - slot[1],
                    GuideSide::Bottom => slot[1] + slot[3] - bounds.max().y,
                };
                assert!(
                    outer.abs() < 1e-3,
                    "{side:?} {part:?}: outer edge off by {outer}"
                );
                inner += side.outward() * thickness;
            }
        }
    }

    fn labels(group: &SceneGroup) -> &SceneTextMark {
        group
            .marks
            .iter()
            .find_map(|mark| match mark {
                SceneMark::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .expect("a text mark")
    }

    #[test]
    fn the_scale_places_data_where_the_parts_draw_the_ticks() {
        for side in [GuideSide::Bottom, GuideSide::Left] {
            let axis = axis(Kind::Linear, side, 30.0, "");
            for length in [120.0, 333.0] {
                let scale = axis.scale(length);
                let group = axis.part(AxisPart::Labels, length).unwrap();
                let mark = labels(&group);
                let along = if side.is_vertical() { &mark.y } else { &mark.x };
                let along = along.as_vec(mark.len as usize, None);
                for (text, position) in mark.text.as_vec(mark.len as usize, None).iter().zip(along)
                {
                    let value: f64 = text.replace(',', "").parse().unwrap();
                    let expected = scale.scale_scalar(&value).unwrap().as_f64().unwrap() as f32;
                    assert!(
                        (expected - position).abs() < 1e-3,
                        "{side:?} {length} {text}"
                    );
                }
            }
        }
        // A y scale keeps pointing up the screen at every length.
        let axis = axis(Kind::Linear, GuideSide::Left, 0.0, "");
        assert_eq!(
            axis.scale(250.0).numeric_interval_range().unwrap(),
            (250.0, 0.0)
        );
    }

    /// Explicit ticks draw where the scale places them at every length, and only the ones that
    /// the domain holds, with a band axis's categories in their own order.
    #[test]
    fn explicit_ticks_draw_where_the_scale_places_them() {
        for (kind, expected) in [
            (Kind::LinearValues, vec!["0", "30", "45", "90"]),
            (Kind::BandValues, vec!["Cherries", "Apples"]),
        ] {
            let axis = axis(kind, GuideSide::Bottom, 0.0, "");
            for length in [120.0, 333.0] {
                let group = axis.part(AxisPart::Labels, length).unwrap();
                let mark = labels(&group);
                let texts = mark.text.as_vec(mark.len as usize, None);
                assert_eq!(texts, expected, "{kind:?} {length}");
                let scale = axis.scale(length);
                let tick_scale = match kind {
                    Kind::BandValues => band_ticks(&scale, false),
                    _ => scale,
                };
                let placed: Vec<f32> = match kind {
                    Kind::BandValues => {
                        let categories = Arc::new(StringArray::from(expected.clone())) as ArrayRef;
                        tick_scale
                            .scale_to_numeric(&categories)
                            .unwrap()
                            .as_vec(categories.len(), None)
                    }
                    _ => expected
                        .iter()
                        .map(|text| {
                            let value: f64 = text.parse().unwrap();
                            tick_scale.scale_scalar(&value).unwrap().as_f64().unwrap() as f32
                        })
                        .collect(),
                };
                let drawn = mark.x.as_vec(mark.len as usize, None);
                for (drawn, placed) in drawn.iter().zip(&placed) {
                    assert!((drawn - placed).abs() < 1e-3, "{kind:?} {length}");
                }
            }
        }
    }

    #[test]
    fn axes_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Axis>();
    }

    #[test]
    fn a_continuous_axis_keeps_a_bounded_number_of_tick_sets() {
        let axis = axis(Kind::Linear, GuideSide::Bottom, 0.0, "");
        for length in (20..2_000).step_by(7) {
            axis.extent(AxisPart::Labels, length as f32);
        }
        let super::Kind::Continuous(ticks) = &axis.kind else {
            panic!("a continuous axis");
        };
        let kept = ticks.kept.lock().unwrap().len();
        assert!(0 < kept && kept <= KEPT_TICK_SETS, "{kept}");
    }
}
