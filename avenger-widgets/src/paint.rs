use crate::frame::Measured;
use crate::{ButtonVariant, Rect, TextStyle, WidgetId, WidgetRuntime, WidgetSpec, WidgetTarget};
use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    rect::SceneRectMark,
    rule::SceneRuleMark,
    text::SceneTextMark,
};

pub(crate) fn rect(
    r: Rect,
    fill: [f32; 4],
    border: [f32; 4],
    width: f32,
    radius: f32,
) -> SceneMark {
    SceneRectMark {
        interactive: false,
        clip: true,
        len: 1,
        x: r.x.into(),
        y: r.y.into(),
        width: Some(r.width.into()),
        height: Some(r.height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(border).into(),
        stroke_width: width.into(),
        corner_radius: radius.into(),
        ..Default::default()
    }
    .into()
}
pub(crate) fn rule(a: [f32; 2], b: [f32; 2], color: [f32; 4], width: f32) -> SceneMark {
    SceneRuleMark {
        interactive: false,
        clip: true,
        len: 1,
        x: a[0].into(),
        y: a[1].into(),
        x2: b[0].into(),
        y2: b[1].into(),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: width.into(),
        ..Default::default()
    }
    .into()
}
pub(crate) fn text(
    value: &str,
    x: f32,
    baseline: f32,
    style: &TextStyle,
    color: [f32; 4],
) -> SceneMark {
    SceneTextMark {
        interactive: false,
        clip: true,
        len: 1,
        text: value.to_owned().into(),
        x: x.into(),
        y: baseline.into(),
        font: style.font.clone().into(),
        font_size: style.size.into(),
        font_weight: style.weight.into(),
        font_style: style.style.into(),
        color: ColorOrGradient::Color(color).into(),
        ..Default::default()
    }
    .into()
}
pub(crate) fn clip(rect: Rect) -> Clip {
    Clip::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
pub(crate) fn control(
    runtime: &WidgetRuntime,
    id: &WidgetId,
    r: Rect,
    outer: Option<Rect>,
    measured: &Measured,
) -> SceneGroup {
    let c = &runtime.controls[id];
    if matches!(c.spec, WidgetSpec::TextInput(_)) {
        return crate::text_runtime::paint(runtime, id, r, outer);
    }
    if c.spec.items().is_some() {
        return choice_group(runtime, id, r, outer, measured);
    }

    let target = WidgetTarget::new(id.clone());
    let hovered = runtime.hovered.as_ref() == Some(&target);
    let pressed = runtime.pressed(&target);
    let enabled = c.spec.options().enabled;
    let mut group = SceneGroup {
        interactive: false,
        clip: outer.map_or(Clip::None, clip),
        ..Default::default()
    };
    let (focus, radius) = match &c.spec {
        WidgetSpec::Button(_) => (&runtime.theme.button.focus, runtime.theme.button.radius),
        WidgetSpec::Checkbox(_) => (&runtime.theme.checkbox.focus, runtime.theme.checkbox.radius),
        WidgetSpec::Slider(_) => (
            &runtime.theme.slider.focus,
            runtime.theme.slider.thumb_size / 2.0,
        ),
        _ => unreachable!("groups and text fields were handled above"),
    };
    if runtime.focus_visible && runtime.focused.as_ref() == Some(&target) {
        let d = focus.gap + focus.width / 2.0;
        group.marks.push(rect(
            Rect::new(r.x - d, r.y - d, r.width + 2.0 * d, r.height + 2.0 * d),
            [0.0; 4],
            focus.color,
            focus.width,
            radius + d,
        ));
    }
    let mut content = SceneGroup {
        interactive: false,
        clip: clip(outer.map_or(r, |c| crate::frame::intersection(r, c))),
        ..Default::default()
    };
    let baseline = r.y + (r.height - measured.label.height) / 2.0 + measured.label.ascent;
    match &c.spec {
        WidgetSpec::Button(b) => {
            let s = &runtime.theme.button;
            let colors = match b.variant {
                ButtonVariant::Neutral => &s.neutral,
                ButtonVariant::Accent => &s.accent,
            }
            .resolve(enabled, hovered, pressed);
            content.marks.push(rect(
                r,
                colors.fill,
                colors.border,
                s.border_width,
                s.radius,
            ));
            content.marks.push(text(
                &b.label,
                r.x + (r.width - measured.label.width) / 2.0,
                baseline,
                &s.text,
                colors.foreground,
            ));
        }
        WidgetSpec::Slider(s) => {
            let style = &runtime.theme.slider;
            let paint = style.paint.resolve(enabled, hovered, pressed);
            let (left, length) = crate::slider::track(r, style, s.value_label.is_some());
            let x = left + length * s.domain.fraction(s.value);
            let cy = r.y + r.height / 2.0;
            let size = style.thumb_size.min(r.height).min(
                (r.width
                    - if s.value_label.is_some() {
                        style.readout_width + style.readout_gap
                    } else {
                        0.0
                    })
                .max(0.0),
            );
            content.marks.push(rect(
                Rect::new(
                    left,
                    cy - style.track_height / 2.0,
                    length,
                    style.track_height,
                ),
                paint.track,
                [0.0; 4],
                0.0,
                style.track_height / 2.0,
            ));
            content.marks.push(rect(
                Rect::new(
                    left,
                    cy - style.track_height / 2.0,
                    x - left,
                    style.track_height,
                ),
                paint.progress,
                [0.0; 4],
                0.0,
                style.track_height / 2.0,
            ));
            content.marks.push(rect(
                Rect::new(x - size / 2.0, cy - size / 2.0, size, size),
                paint.thumb,
                paint.thumb_border,
                1.0,
                size / 2.0,
            ));
            if let Some(label) = &s.value_label {
                content.marks.push(text(
                    label,
                    r.x + (r.width - style.readout_width).max(0.0),
                    baseline,
                    &style.text,
                    paint.foreground,
                ));
            }
        }
        WidgetSpec::TextInput(_) | WidgetSpec::CheckboxGroup(_) | WidgetSpec::RadioGroup(_) => {
            unreachable!("choice groups were handled above")
        }
        WidgetSpec::Checkbox(c) => {
            let s = &runtime.theme.checkbox;
            let colors = if c.checked { &s.checked } else { &s.unchecked }
                .resolve(enabled, hovered, pressed);
            let size = s.box_size.min(r.height);
            let bx = r.x;
            let by = r.y + (r.height - size) / 2.0;
            content.marks.push(rect(
                Rect::new(bx, by, size, size),
                colors.fill,
                colors.border,
                s.border_width,
                s.radius,
            ));
            if c.checked {
                content.marks.push(rule(
                    [bx + size * 0.22, by + size * 0.52],
                    [bx + size * 0.43, by + size * 0.73],
                    colors.foreground,
                    2.0,
                ));
                content.marks.push(rule(
                    [bx + size * 0.43, by + size * 0.73],
                    [bx + size * 0.80, by + size * 0.27],
                    colors.foreground,
                    2.0,
                ));
            }
            let label = runtime
                .theme
                .checkbox
                .unchecked
                .resolve(enabled, hovered, pressed)
                .foreground;
            content.marks.push(text(
                &c.label,
                r.x + s.box_size + s.gap,
                baseline,
                &s.text,
                label,
            ));
        }
    }
    let region = runtime.regions.iter().find(|v| v.target == target).unwrap();
    content.marks.push(
        SceneRectMark {
            name: region.name.clone(),
            interactive: true,
            clip: true,
            len: 1,
            x: r.x.into(),
            y: r.y.into(),
            width: Some(r.width.into()),
            height: Some(r.height.into()),
            fill: ColorOrGradient::Color([0.0; 4]).into(),
            ..Default::default()
        }
        .into(),
    );
    group.marks.push(content.into());
    group
}

fn choice_group(
    runtime: &WidgetRuntime,
    id: &WidgetId,
    r: Rect,
    outer: Option<Rect>,
    measured: &Measured,
) -> SceneGroup {
    let c = &runtime.controls[id];
    let radio = matches!(c.spec, WidgetSpec::RadioGroup(_));
    let mut group = SceneGroup {
        interactive: false,
        clip: outer.map_or(Clip::None, clip),
        ..Default::default()
    };
    if !c.spec.label().is_empty() {
        let mut label = SceneGroup {
            interactive: false,
            clip: clip(outer.map_or(r, |o| crate::frame::intersection(r, o))),
            ..Default::default()
        };
        let foreground = if radio {
            runtime
                .theme
                .radio
                .paint
                .resolve(c.spec.options().enabled, false, false)
                .foreground
        } else {
            runtime
                .theme
                .checkbox
                .unchecked
                .resolve(c.spec.options().enabled, false, false)
                .foreground
        };
        label.marks.push(text(
            c.spec.label(),
            r.x,
            r.y + measured.label.ascent,
            &runtime.theme.group.text,
            foreground,
        ));
        group.marks.push(label.into());
    }
    for ((id, bounds, _), item) in measured.rows.iter().zip(c.spec.items().unwrap()) {
        let target = WidgetTarget::item(c.spec.id().clone(), id.clone());
        let region = runtime.regions.iter().find(|r| r.target == target).unwrap();
        let checked = match &c.spec {
            WidgetSpec::CheckboxGroup(g) => g.checked.contains(id),
            WidgetSpec::RadioGroup(g) => g.selected.as_ref() == Some(id),
            _ => false,
        };
        group.marks.push(
            choice_row(
                runtime,
                &target,
                &item.label,
                checked,
                radio,
                region.rect,
                bounds,
            )
            .into(),
        );
    }
    group
}
fn choice_row(
    runtime: &WidgetRuntime,
    target: &WidgetTarget,
    label: &str,
    checked: bool,
    radio: bool,
    r: Rect,
    bounds: &avenger_text::measurement::TextBounds,
) -> SceneGroup {
    let enabled = runtime.eligible(target);
    let hovered = runtime.hovered.as_ref() == Some(target);
    let pressed = runtime.pressed(target);
    let (style, box_size, gap, border_width, focus, radius, fill, border, indicator, foreground) =
        if radio {
            let s = &runtime.theme.radio;
            let p = s.paint.resolve(enabled, hovered, pressed);
            (
                &s.text,
                s.diameter,
                s.gap,
                s.border_width,
                &s.focus,
                s.diameter / 2.0,
                p.fill,
                if checked { p.indicator } else { p.border },
                p.indicator,
                p.foreground,
            )
        } else {
            let s = &runtime.theme.checkbox;
            let p =
                if checked { &s.checked } else { &s.unchecked }.resolve(enabled, hovered, pressed);
            (
                &s.text,
                s.box_size,
                s.gap,
                s.border_width,
                &s.focus,
                s.radius,
                p.fill,
                p.border,
                p.foreground,
                s.unchecked.resolve(enabled, hovered, pressed).foreground,
            )
        };
    let region = runtime
        .regions
        .iter()
        .find(|r| &r.target == target)
        .unwrap();
    let mut group = SceneGroup {
        interactive: false,
        ..Default::default()
    };
    if runtime.focus_visible && runtime.focused.as_ref() == Some(target) {
        let d = focus.gap + focus.width / 2.0;
        group.marks.push(rect(
            Rect::new(r.x - d, r.y - d, r.width + 2.0 * d, r.height + 2.0 * d),
            [0.0; 4],
            focus.color,
            focus.width,
            3.0,
        ));
    }
    let mut content = SceneGroup {
        interactive: false,
        clip: clip(region.clip),
        ..Default::default()
    };
    let size = box_size.min(r.height);
    let bx = r.x;
    let by = r.y + (r.height - size) / 2.0;
    content.marks.push(rect(
        Rect::new(bx, by, size, size),
        fill,
        border,
        border_width,
        if radio { size / 2.0 } else { radius },
    ));
    if checked {
        if radio {
            content.marks.push(rect(
                Rect::new(bx + size * 0.25, by + size * 0.25, size * 0.5, size * 0.5),
                indicator,
                [0.0; 4],
                0.0,
                size,
            ));
        } else {
            content.marks.push(rule(
                [bx + size * 0.22, by + size * 0.52],
                [bx + size * 0.43, by + size * 0.73],
                indicator,
                2.0,
            ));
            content.marks.push(rule(
                [bx + size * 0.43, by + size * 0.73],
                [bx + size * 0.80, by + size * 0.27],
                indicator,
                2.0,
            ));
        }
    }
    content.marks.push(text(
        label,
        r.x + box_size + gap,
        r.y + (r.height - bounds.height) / 2.0 + bounds.ascent,
        style,
        foreground,
    ));
    content.marks.push(
        SceneRectMark {
            name: region.name.clone(),
            interactive: true,
            clip: true,
            len: 1,
            x: r.x.into(),
            y: r.y.into(),
            width: Some(r.width.into()),
            height: Some(r.height.into()),
            fill: ColorOrGradient::Color([0.0; 4]).into(),
            ..Default::default()
        }
        .into(),
    );
    group.marks.push(content.into());
    group
}
