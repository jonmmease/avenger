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
