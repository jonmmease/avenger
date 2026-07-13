//! Stable part manifests for Avenger's built-in widgets.

use avenger_chart::prelude::{WidgetPartManifest, WidgetStyleProperty};

/// Built-in widget kinds whose public part topology is stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinWidgetKind {
    Checkbox,
    Button,
    CheckboxList,
    RadioButtonList,
    Slider,
    TextInput,
}

impl BuiltinWidgetKind {
    pub const ALL: &'static [Self] = &[
        Self::Checkbox,
        Self::Button,
        Self::CheckboxList,
        Self::RadioButtonList,
        Self::Slider,
        Self::TextInput,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Checkbox => "checkbox",
            Self::Button => "button",
            Self::CheckboxList => "checkbox-list",
            Self::RadioButtonList => "radio-button-list",
            Self::Slider => "slider",
            Self::TextInput => "text-input",
        }
    }

    /// Returns the ordered public `::part()` and interaction contract.
    pub fn part_manifest(self) -> Vec<WidgetPartManifest> {
        use WidgetStyleProperty as P;

        let control = &[
            P::Fill,
            P::Stroke,
            P::StrokeWidth,
            P::CornerRadius,
            P::ControlHeight,
            P::BorderWidth,
            P::Cursor,
        ];
        let label = &[
            P::Fill,
            P::Opacity,
            P::FontFamily,
            P::FontSize,
            P::FontWeight,
            P::ControlLabelGap,
            P::VisualLabelGap,
            P::Cursor,
        ];
        let focus = &[
            P::Stroke,
            P::StrokeWidth,
            P::FocusRingWidth,
            P::CornerRadius,
            P::Opacity,
            P::FocusGap,
        ];
        match self {
            Self::Checkbox => vec![
                part(
                    "box",
                    "rect",
                    &with_properties(control, &[P::ChoiceControlSize]),
                    CHECK_STATES,
                    true,
                ),
                part(
                    "check",
                    "rule",
                    &[P::Stroke, P::StrokeWidth, P::Opacity, P::Cursor],
                    CHECK_STATES,
                    true,
                ),
                part("label", "text", label, CHECK_STATES, true),
                part("focus-ring", "rect", focus, CHECK_STATES, false),
            ],
            Self::Button => vec![
                part(
                    "box",
                    "rect",
                    &with_properties(
                        control,
                        &[
                            P::ButtonMinWidth,
                            P::ButtonInlinePadding,
                            P::ButtonLineHeight,
                            P::ButtonRadius,
                            P::ButtonBorderWidth,
                        ],
                    ),
                    BUTTON_STATES,
                    true,
                ),
                part("label", "text", label, BUTTON_STATES, true),
                part("focus-ring", "rect", focus, BUTTON_STATES, false),
            ],
            Self::CheckboxList => vec![
                part(
                    "row",
                    "rect",
                    &[P::Fill, P::Opacity, P::Cursor],
                    LIST_STATES,
                    true,
                ),
                part(
                    "box",
                    "rect",
                    &with_properties(control, &[P::ChoiceControlSize]),
                    LIST_STATES,
                    true,
                ),
                part(
                    "check",
                    "rule",
                    &[P::Stroke, P::StrokeWidth, P::Opacity, P::Cursor],
                    LIST_STATES,
                    true,
                ),
                part("label", "text", label, LIST_STATES, true),
                part("focus-ring", "rect", focus, LIST_STATES, false),
            ],
            Self::RadioButtonList => vec![
                part(
                    "row",
                    "rect",
                    &[P::Fill, P::Opacity, P::Cursor],
                    LIST_STATES,
                    true,
                ),
                part(
                    "control",
                    "symbol",
                    &with_properties(
                        control,
                        &[
                            P::ChoiceControlSize,
                            P::RadioSelectedBorderWidth,
                            P::RadioCenterSize,
                        ],
                    ),
                    LIST_STATES,
                    true,
                ),
                part(
                    "center",
                    "symbol",
                    &[P::Fill, P::Stroke, P::Opacity, P::Cursor],
                    LIST_STATES,
                    true,
                ),
                part("label", "text", label, LIST_STATES, true),
                part("focus-ring", "symbol", focus, LIST_STATES, false),
            ],
            Self::Slider => vec![
                part(
                    "track",
                    "rect",
                    &[
                        P::Fill,
                        P::Stroke,
                        P::StrokeWidth,
                        P::CornerRadius,
                        P::SliderMinWidth,
                        P::SliderTrackHeight,
                        P::Cursor,
                    ],
                    SLIDER_STATES,
                    true,
                ),
                part(
                    "fill",
                    "rect",
                    &[P::Fill, P::Opacity, P::CornerRadius, P::Cursor],
                    SLIDER_STATES,
                    true,
                ),
                part(
                    "handle",
                    "symbol",
                    &[
                        P::Fill,
                        P::Stroke,
                        P::StrokeWidth,
                        P::Opacity,
                        P::SliderHandleSize,
                        P::SliderHandleBorderWidth,
                        P::SliderHandlePressedBorderWidth,
                        P::Cursor,
                    ],
                    SLIDER_STATES,
                    true,
                ),
                part("label", "text", label, SLIDER_STATES, true),
                part(
                    "value-label",
                    "text",
                    &with_properties(label, &[P::SliderValuePadding]),
                    SLIDER_STATES,
                    true,
                ),
                part("focus-ring", "symbol", focus, SLIDER_STATES, false),
            ],
            Self::TextInput => vec![
                part(
                    "box",
                    "rect",
                    &with_properties(control, &[P::InputInlineInset]),
                    INPUT_STATES,
                    true,
                ),
                part("text", "text", label, INPUT_STATES, true),
                part(
                    "placeholder",
                    "text",
                    &with_properties(label, &[P::InputPlaceholderColor]),
                    INPUT_STATES,
                    true,
                ),
                part(
                    "selection",
                    "rect",
                    &[P::Fill, P::Opacity, P::InputSelectionOpacity],
                    INPUT_STATES,
                    false,
                ),
                part("preedit", "text", label, INPUT_STATES, false),
                part(
                    "caret",
                    "rule",
                    &[P::Stroke, P::StrokeWidth, P::Opacity, P::InputCaretWidth],
                    INPUT_STATES,
                    false,
                ),
                part("focus-ring", "rect", focus, INPUT_STATES, false),
            ],
        }
    }
}

const CHECK_STATES: &[&str] = &["checked", "disabled", "focus-visible", "hover", "pressed"];
const BUTTON_STATES: &[&str] = &["variant", "disabled", "focus-visible", "hover", "pressed"];
const LIST_STATES: &[&str] = &[
    "selected",
    "orientation",
    "disabled",
    "focus-visible",
    "hover",
    "pressed",
];
const SLIDER_STATES: &[&str] = &[
    "orientation",
    "disabled",
    "focus-visible",
    "hover",
    "pressed",
];
const INPUT_STATES: &[&str] = &["disabled", "focus-visible", "hover", "pressed"];

fn part(
    name: &str,
    scene_mark_kind: &str,
    properties: &[WidgetStyleProperty],
    states: &[&str],
    interactive: bool,
) -> WidgetPartManifest {
    WidgetPartManifest {
        name: name.to_string(),
        scene_mark_kind: scene_mark_kind.to_string(),
        style_properties: properties.to_vec(),
        states: states.iter().map(|state| (*state).to_string()).collect(),
        interactive,
    }
}

fn with_properties(
    base: &[WidgetStyleProperty],
    additional: &[WidgetStyleProperty],
) -> Vec<WidgetStyleProperty> {
    base.iter().chain(additional).copied().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use avenger_chart::prelude::is_decorative_widget_part;

    use super::*;

    #[test]
    fn built_in_part_manifests_are_unique_and_interaction_safe() {
        for kind in BuiltinWidgetKind::ALL {
            let manifest = kind.part_manifest();
            let mut names = HashSet::new();
            for part in &manifest {
                assert!(
                    names.insert(part.name.as_str()),
                    "duplicate {} part {}",
                    kind.name(),
                    part.name
                );
                assert!(
                    !part.style_properties.is_empty(),
                    "unstyled {} part {}",
                    kind.name(),
                    part.name
                );
                assert_eq!(
                    part.interactive,
                    !is_decorative_widget_part(&part.name),
                    "interaction convention drift for {} part {}",
                    kind.name(),
                    part.name
                );
            }
        }
    }

    #[test]
    fn every_typed_style_property_has_a_public_consumer() {
        let mut consumed = WidgetStyleProperty::ALL
            .iter()
            .copied()
            .filter(|property| property.applies_to_host())
            .collect::<HashSet<_>>();
        for kind in BuiltinWidgetKind::ALL {
            for part in kind.part_manifest() {
                consumed.extend(part.style_properties);
            }
        }
        let missing = WidgetStyleProperty::ALL
            .iter()
            .copied()
            .filter(|property| !consumed.contains(property))
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "unconsumed widget properties: {missing:?}"
        );
    }

    #[test]
    fn built_in_part_names_are_a_stable_public_contract() {
        let actual = BuiltinWidgetKind::ALL
            .iter()
            .map(|kind| {
                (
                    kind.name(),
                    kind.part_manifest()
                        .into_iter()
                        .map(|part| part.name)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (
                    "checkbox",
                    strings(&["box", "check", "label", "focus-ring"])
                ),
                ("button", strings(&["box", "label", "focus-ring"])),
                (
                    "checkbox-list",
                    strings(&["row", "box", "check", "label", "focus-ring"])
                ),
                (
                    "radio-button-list",
                    strings(&["row", "control", "center", "label", "focus-ring"])
                ),
                (
                    "slider",
                    strings(&[
                        "track",
                        "fill",
                        "handle",
                        "label",
                        "value-label",
                        "focus-ring"
                    ])
                ),
                (
                    "text-input",
                    strings(&[
                        "box",
                        "text",
                        "placeholder",
                        "selection",
                        "preedit",
                        "caret",
                        "focus-ring"
                    ])
                ),
            ]
        );
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }
}
