use avenger_panels::{Scope, Side};
use avenger_text::TextEngine;
use avenger_widgets::prelude::*;

/// Controls shared by the native host, browser, and PNG exporter.
#[derive(Clone)]
pub struct State {
    pub size: [f32; 2],
    pub engine: TextEngine,
    pub widgets: WidgetRuntime,
    pub y_scope: usize,
    pub title_scope: usize,
    pub legend_scope: usize,
    pub outer: bool,
    pub legend_bottom: bool,
    pub missing: usize,
    pub overlay: bool,
    pub preset: usize,
}
impl State {
    pub fn new(engine: TextEngine) -> Self {
        Self {
            size: [1280.0, 900.0],
            engine,
            widgets: WidgetRuntime::new(),
            y_scope: 1,
            title_scope: 2,
            legend_scope: 1,
            outer: true,
            legend_bottom: false,
            missing: 0,
            overlay: false,
            preset: 0,
        }
    }
    pub fn scope(index: usize) -> Scope {
        match index {
            0 => Scope::Panel,
            1 => Scope::ancestor(1).expect("positive depth"),
            _ => Scope::Root,
        }
    }
    pub fn scope_name(index: usize) -> &'static str {
        ["Each panel", "Each region", "Whole figure"][index]
    }
    pub fn legend_side(&self) -> Side {
        if self.legend_bottom {
            Side::Bottom
        } else {
            Side::Right
        }
    }
    pub fn columns(&self) -> usize {
        ((self.size[0] - 320.0) / 260.0).floor().clamp(1.0, 3.0) as usize
    }
    pub fn controls(&self) -> [(&'static str, String); 8] {
        [
            ("1  Y domain", Self::scope_name(self.y_scope).into()),
            (
                "2  Tick labels",
                if self.outer {
                    "Outer compatible axes"
                } else {
                    "Every panel"
                }
                .into(),
            ),
            ("3  Y title", Self::scope_name(self.title_scope).into()),
            ("4  Legend", Self::scope_name(self.legend_scope).into()),
            (
                "5  Legend side",
                if self.legend_bottom {
                    "Bottom"
                } else {
                    "Right"
                }
                .into(),
            ),
            (
                "6  Last product",
                ["Has data", "Empty panel", "Physical hole"][self.missing].into(),
            ),
            (
                "7  Coordination",
                if self.overlay {
                    "Show groups and owners"
                } else {
                    "Off"
                }
                .into(),
            ),
            (
                "8  Example",
                [
                    "Sales by channel",
                    "Independent domains",
                    "Equal bounds, different units",
                ][self.preset]
                    .into(),
            ),
        ]
    }
    pub fn widget_specs(&self) -> Vec<WidgetSpec> {
        self.controls()
            .into_iter()
            .enumerate()
            .map(|(index, (_, value))| {
                let id = format!("control-{index}");
                match index {
                    0 => RadioGroup::new(
                        id,
                        [
                            ChoiceItem::new("panel", "Panel"),
                            ChoiceItem::new("region", "Region"),
                            ChoiceItem::new("figure", "Figure"),
                        ],
                        Some(["panel", "region", "figure"][self.y_scope].into()),
                    )
                    .semantic_name("Y domain sharing")
                    .orientation(ChoiceOrientation::Horizontal)
                    .into(),
                    1 => Checkbox::new(id, "Outer compatible axes", self.outer).into(),
                    4 => Checkbox::new(id, "Place at bottom", self.legend_bottom).into(),
                    6 => Checkbox::new(id, "Show groups and owners", self.overlay).into(),
                    _ => Button::new(id, value).into(),
                }
            })
            .collect()
    }
    pub fn apply_widgets(&mut self, events: Vec<WidgetEvent>) -> bool {
        let mut changed = false;
        for event in events {
            match event.action {
                WidgetAction::SelectionChanged { item } if event.id.as_str() == "control-0" => {
                    self.y_scope = match item.as_str() {
                        "panel" => 0,
                        "region" => 1,
                        _ => 2,
                    };
                }
                WidgetAction::CheckedChanged { value } => match event.id.as_str() {
                    "control-1" => self.outer = value,
                    "control-4" => self.legend_bottom = value,
                    "control-6" => self.overlay = value,
                    _ => continue,
                },
                WidgetAction::Activated => {
                    if let Some(index) = event
                        .id
                        .as_str()
                        .strip_prefix("control-")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        self.activate(index);
                    }
                }
                _ => continue,
            }
            changed = true;
        }
        changed
    }
    pub fn activate(&mut self, index: usize) {
        match index {
            0 => self.y_scope = (self.y_scope + 1) % 3,
            1 => self.outer = !self.outer,
            2 => self.title_scope = (self.title_scope + 1) % 3,
            3 => self.legend_scope = (self.legend_scope + 1) % 3,
            4 => self.legend_bottom = !self.legend_bottom,
            5 => self.missing = (self.missing + 1) % 3,
            6 => self.overlay = !self.overlay,
            7 => {
                self.preset = (self.preset + 1) % 3;
                self.y_scope = if self.preset == 1 { 0 } else { 1 };
            }
            _ => {}
        }
    }
}
