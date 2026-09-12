use crate::{
    Edges, Rect, Size, WidgetError, WidgetId, WidgetRuntime, WidgetSpec, WidgetTarget, WidgetTheme,
    WidgetUpdate,
};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_text::{TextEngine, measurement::TextBounds};
use std::collections::{BTreeMap, BTreeSet};

/// Sizes, first text baseline, and focus-paint overflow in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetMetrics {
    pub minimum: Size,
    pub preferred: Size,
    pub baseline: Option<f32>,
    pub paint_overflow: Edges,
}
#[derive(Clone, Debug)]
pub(crate) struct Measured {
    pub metrics: WidgetMetrics,
    pub label: TextBounds,
    pub rows: Vec<(crate::ChoiceItemId, TextBounds, Size)>,
    pub header_height: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    pub target: WidgetTarget,
    pub rect: Rect,
    pub clip: Rect,
    pub name: String,
}

/// An isolated candidate frame. Dropping it leaves the installed runtime unchanged.
pub struct PreparedWidgets {
    pub(crate) source_revision: u64,
    pub(crate) candidate: WidgetRuntime,
    pub(crate) measurements: BTreeMap<WidgetId, Measured>,
    pub(crate) update: WidgetUpdate,
    pub(crate) placements: BTreeMap<WidgetId, (Rect, Option<Rect>)>,
}
impl PreparedWidgets {
    /// Read intrinsic sizes before allocating a control rectangle.
    pub fn metrics(&self, id: impl Into<WidgetId>) -> Option<WidgetMetrics> {
        self.measurements.get(&id.into()).map(|v| v.metrics)
    }
    /// Allocate a control in root-canvas coordinates, with an optional outer clip.
    /// Every declared control must be placed once. Small allocations clip content.
    pub fn place(
        &mut self,
        id: impl Into<WidgetId>,
        rect: Rect,
        clip: Option<Rect>,
    ) -> Result<(), WidgetError> {
        let id = id.into();
        validate_rect(rect)?;
        if let Some(clip) = clip {
            validate_rect(clip)?;
        }
        if !self.measurements.contains_key(&id) {
            return Err(WidgetError::Invalid(format!(
                "unknown widget {}",
                id.as_str()
            )));
        }
        if self.placements.contains_key(&id) {
            return Err(WidgetError::Invalid(format!(
                "widget {} was already placed",
                id.as_str()
            )));
        }
        self.placements.insert(id, (rect, clip));
        Ok(())
    }
    /// Render the candidate and reconcile lifecycle changes without publishing effects.
    pub fn finish(mut self) -> Result<WidgetFrame, WidgetError> {
        if self.placements.len() != self.measurements.len() {
            return Err(WidgetError::Invalid(
                "every declared widget must be placed".into(),
            ));
        }
        let mut regions = Vec::new();
        for id in &self.candidate.order {
            let control = &self.candidate.controls[id];
            let (rect, outer) = self.placements[id];
            let measured = &self.measurements[id];
            if let Some(items) = control.spec.items() {
                regions.push(Region {
                    target: WidgetTarget::new(id.clone()),
                    rect,
                    clip: outer.map_or(rect, |c| intersection(rect, c)),
                    name: String::new(),
                });
                let horizontal = matches!(&control.spec,WidgetSpec::CheckboxGroup(g) if g.orientation==crate::ChoiceOrientation::Horizontal)
                    || matches!(&control.spec,WidgetSpec::RadioGroup(g) if g.orientation==crate::ChoiceOrientation::Horizontal);
                let mut x = rect.x;
                let mut y = rect.y + measured.header_height;
                for (i, item) in items.iter().enumerate() {
                    let size = measured.rows[i].2;
                    let row = Rect::new(
                        x,
                        y,
                        if horizontal { size.width } else { rect.width },
                        size.height,
                    );
                    let epoch = control
                        .item_epochs
                        .get(&item.id)
                        .copied()
                        .unwrap_or(control.epoch);
                    regions.push(Region {
                        target: WidgetTarget::item(id.clone(), item.id.clone()),
                        rect: row,
                        clip: intersection(row, outer.map_or(rect, |c| intersection(rect, c))),
                        name: if item.enabled {
                            format!(
                                "__avenger_widget_{}_{}_item",
                                self.candidate.namespace, epoch
                            )
                        } else {
                            format!(
                                "__avenger_widget_{}_{}_disabled_{}",
                                self.candidate.namespace, epoch, i
                            )
                        },
                    });
                    if horizontal {
                        x += size.width + self.candidate.theme.group.gap;
                    } else {
                        y += size.height + self.candidate.theme.group.gap;
                    }
                }
            } else {
                regions.push(Region {
                    target: WidgetTarget::new(id.clone()),
                    rect,
                    clip: outer.map_or(rect, |c| intersection(rect, c)),
                    name: format!(
                        "__avenger_widget_{}_{}",
                        self.candidate.namespace, control.epoch
                    ),
                });
            }
        }
        let geometry_changed = self.candidate.regions != regions;
        self.candidate.regions = regions;
        let mut update = self.update;
        self.candidate.reconcile_targets(&mut update);
        self.candidate.layout_text(&mut update)?;
        let mut scene = SceneGroup {
            name: format!("__avenger_widgets_{}", self.candidate.namespace),
            interactive: false,
            ..Default::default()
        };
        for id in &self.candidate.order {
            let (rect, clip) = self.placements[id];
            scene.marks.push(
                crate::paint::control(&self.candidate, id, rect, clip, &self.measurements[id])
                    .into(),
            );
        }
        self.candidate.publish_policy(&mut update);
        update.status.rebuild_geometry |= geometry_changed;
        Ok(WidgetFrame {
            scene,
            candidate: self.candidate,
            source_revision: self.source_revision,
            update,
        })
    }
}

/// Scene fragment and matching state. Clone the scene into the full application
/// scene, then install this frame only when that scene build succeeds.
pub struct WidgetFrame {
    pub scene: SceneGroup,
    pub(crate) candidate: WidgetRuntime,
    pub(crate) source_revision: u64,
    pub(crate) update: WidgetUpdate,
}

impl WidgetRuntime {
    /// Validate and measure descriptions without mutating the installed runtime.
    pub fn prepare(
        &self,
        specs: &[WidgetSpec],
        theme: &WidgetTheme,
        engine: &TextEngine,
    ) -> Result<PreparedWidgets, WidgetError> {
        theme.validate()?;
        let mut candidate = self.clone();
        candidate.theme = theme.clone();
        candidate.engine = Some(engine.clone());
        let mut seen = BTreeSet::new();
        let mut update = WidgetUpdate::default();
        let mut measurements = BTreeMap::new();
        for spec in specs {
            if !seen.insert(spec.id().clone()) {
                return Err(WidgetError::Invalid(format!(
                    "duplicate widget {}",
                    spec.id().as_str()
                )));
            }
            candidate.reconcile_spec(spec, &mut update)?;
            measurements.insert(spec.id().clone(), measure(spec, theme, engine)?);
        }
        for id in candidate
            .controls
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect::<Vec<_>>()
        {
            candidate.deactivate_text(
                &WidgetTarget::new(id),
                Some(crate::TextCancelReason::Removed),
                &mut update,
            );
        }
        candidate.controls.retain(|id, _| seen.contains(id));
        candidate.order = specs.iter().map(|s| s.id().clone()).collect();
        Ok(PreparedWidgets {
            source_revision: self.revision,
            update,
            candidate,
            measurements,
            placements: BTreeMap::new(),
        })
    }
    /// Commit one frame and return its lifecycle notifications and host effects.
    /// Install into the same state transaction as the complete scene graph.
    pub fn install(&mut self, mut frame: WidgetFrame) -> Result<WidgetUpdate, WidgetError> {
        if self.namespace != frame.candidate.namespace || self.revision != frame.source_revision {
            return Err(WidgetError::StaleFrame);
        }
        frame.candidate.revision = self.revision + 1;
        *self = frame.candidate;
        Ok(frame.update)
    }
}
pub(crate) fn validate_rect(rect: Rect) -> Result<(), WidgetError> {
    if ![
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.x + rect.width,
        rect.y + rect.height,
    ]
    .iter()
    .all(|v| v.is_finite())
        || rect.width < 0.0
        || rect.height < 0.0
    {
        return Err(WidgetError::Invalid(
            "rectangles require finite coordinates and nonnegative dimensions".into(),
        ));
    }
    Ok(())
}
pub(crate) fn intersection(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(
        x,
        y,
        (a.x + a.width).min(b.x + b.width).max(x) - x,
        (a.y + a.height).min(b.y + b.height).max(y) - y,
    )
}
pub(crate) fn visible(rect: Rect) -> bool {
    rect.width > 0.0 && rect.height > 0.0
}

fn measure(
    spec: &WidgetSpec,
    theme: &WidgetTheme,
    engine: &TextEngine,
) -> Result<Measured, WidgetError> {
    let (text, height, prefix, padding, min_width, focus) = match spec {
        WidgetSpec::Button(_) => {
            let s = &theme.button;
            (&s.text, s.height, 0.0, s.padding, s.min_width, &s.focus)
        }
        WidgetSpec::Checkbox(_) | WidgetSpec::CheckboxGroup(_) => {
            let s = &theme.checkbox;
            (
                &s.text,
                s.row_height,
                s.box_size + s.gap,
                0.0,
                s.box_size,
                &s.focus,
            )
        }
        WidgetSpec::RadioGroup(_) => {
            let s = &theme.radio;
            (
                &s.text,
                s.row_height,
                s.box_size + s.gap,
                0.0,
                s.box_size,
                &s.focus,
            )
        }
        WidgetSpec::TextInput(t) => {
            let s = &theme.text_input;
            (
                t.text_style.as_ref().unwrap_or(&s.text),
                s.height,
                0.0,
                0.0,
                s.padding * 2.0,
                &s.focus,
            )
        }
        WidgetSpec::Slider(_) => {
            let s = &theme.slider;
            (&s.text, s.height, 0.0, 0.0, s.thumb_size, &s.focus)
        }
    };
    let mut label = engine.measure_bounds(&if spec.items().is_some() {
        theme.group.text.config(spec.label())
    } else {
        text.config(spec.label())
    })?;
    let mut h = height.max(label.height);
    let mut w = min_width.max(prefix + label.width + 2.0 * padding);
    let mut rows = Vec::new();
    let mut header_height = 0.0;
    if let Some(items) = spec.items() {
        header_height = if spec.label().is_empty() {
            0.0
        } else {
            label.height + theme.group.label_gap
        };
        let horizontal = matches!(spec,WidgetSpec::CheckboxGroup(g) if g.orientation==crate::ChoiceOrientation::Horizontal)
            || matches!(spec,WidgetSpec::RadioGroup(g) if g.orientation==crate::ChoiceOrientation::Horizontal);
        w = 0.0;
        h = 0.0;
        for item in items {
            let bounds = engine.measure_bounds(&text.config(&item.label))?;
            let size = Size::new(prefix + bounds.width, height.max(bounds.height));
            if horizontal {
                w += size.width;
                h = h.max(size.height);
            } else {
                w = w.max(size.width);
                h += size.height;
            }
            rows.push((item.id.clone(), bounds, size));
        }
        let gaps = items.len().saturating_sub(1) as f32 * theme.group.gap;
        if horizontal {
            w += gaps;
        } else {
            h += gaps;
        }
        w = w.max(label.width);
        h += header_height;
    }
    if matches!(spec, WidgetSpec::TextInput(_)) {
        let font = engine.font_metrics(&avenger_text::measurement::FontMetricsConfig {
            font: &text.font,
            font_size: text.size,
            font_weight: text.weight,
            font_style: text.style,
        })?;
        label.height = font.height;
        label.ascent = font.ascent;
        label.descent = font.descent;
        label.line_height = font.line_height;
        h = height.max(font.height + 2.0 * theme.text_input.padding);
        w = theme.text_input.width.max(min_width);
    }
    if matches!(spec, WidgetSpec::Slider(_)) {
        w = theme.slider.width.max(min_width);
    }
    let o = focus.overflow();
    crate::style::lengths(&[w, h, o, label.ascent])?;
    Ok(Measured {
        metrics: WidgetMetrics {
            minimum: Size::new(min_width.min(w), h),
            preferred: Size::new(w, h),
            baseline: Some(if spec.items().is_some() {
                label.ascent
            } else {
                (h - label.height) / 2.0 + label.ascent
            }),
            paint_overflow: Edges::new(o, o, o, o),
        },
        label,
        rows,
        header_height,
    })
}
