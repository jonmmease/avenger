use avenger_panels::{Scope, Side};
use avenger_text::TextEngine;
use avenger_widgets::prelude::*;

const SCOPE_IDS: [&str; 3] = ["panel", "region", "figure"];
const PRODUCT_IDS: [&str; 3] = ["data", "empty", "hole"];
const PRESET_IDS: [&str; 3] = ["sales", "independent", "units"];
pub const CONTROL_LABELS: [&str; 8] = [
    "1  Y domain",
    "2  Tick labels",
    "3  Y title",
    "4  Legend",
    "5  Legend side",
    "6  Last product",
    "7  Coordination",
    "8  Example",
];

fn scope_control(id: &str, name: &str, selected: usize) -> WidgetSpec {
    RadioGroup::new(
        id,
        [
            ChoiceItem::new("panel", "Panel"),
            ChoiceItem::new("region", "Region"),
            ChoiceItem::new("figure", "Figure"),
        ],
        Some(SCOPE_IDS[selected].into()),
    )
    .semantic_name(name)
    .orientation(ChoiceOrientation::Horizontal)
    .into()
}

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
    pub fn widget_specs(&self) -> Vec<WidgetSpec> {
        vec![
            scope_control("control-0", "Y domain sharing", self.y_scope),
            Checkbox::new("control-1", "Outer compatible axes", self.outer).into(),
            scope_control("control-2", "Y title sharing", self.title_scope),
            scope_control("control-3", "Legend sharing", self.legend_scope),
            Checkbox::new("control-4", "Place at bottom", self.legend_bottom).into(),
            RadioGroup::new(
                "control-5",
                [
                    ChoiceItem::new("data", "Data"),
                    ChoiceItem::new("empty", "Empty"),
                    ChoiceItem::new("hole", "Hole"),
                ],
                Some(PRODUCT_IDS[self.missing].into()),
            )
            .semantic_name("Last product")
            .orientation(ChoiceOrientation::Horizontal)
            .into(),
            Checkbox::new("control-6", "Show groups and owners", self.overlay).into(),
            RadioGroup::new(
                "control-7",
                [
                    ChoiceItem::new("sales", "Sales by channel"),
                    ChoiceItem::new("independent", "Independent domains"),
                    ChoiceItem::new("units", "Equal bounds, different units"),
                ],
                Some(PRESET_IDS[self.preset].into()),
            )
            .semantic_name("Example")
            .into(),
        ]
    }
    pub fn apply_widgets(&mut self, events: Vec<WidgetEvent>) -> bool {
        let mut changed = false;
        for event in events {
            match event.action {
                WidgetAction::SelectionChanged { item } => match event.id.as_str() {
                    "control-0" | "control-2" | "control-3" => {
                        let Some(value) = SCOPE_IDS.iter().position(|id| *id == item.as_str())
                        else {
                            continue;
                        };
                        match event.id.as_str() {
                            "control-0" => self.y_scope = value,
                            "control-2" => self.title_scope = value,
                            _ => self.legend_scope = value,
                        }
                    }
                    "control-5" => {
                        let Some(value) = PRODUCT_IDS.iter().position(|id| *id == item.as_str())
                        else {
                            continue;
                        };
                        self.missing = value;
                    }
                    "control-7" => {
                        let Some(value) = PRESET_IDS.iter().position(|id| *id == item.as_str())
                        else {
                            continue;
                        };
                        self.select_preset(value);
                    }
                    _ => continue,
                },
                WidgetAction::CheckedChanged { value } => match event.id.as_str() {
                    "control-1" => self.outer = value,
                    "control-4" => self.legend_bottom = value,
                    "control-6" => self.overlay = value,
                    _ => continue,
                },
                _ => continue,
            }
            changed = true;
        }
        changed
    }
    fn select_preset(&mut self, index: usize) {
        self.preset = index;
        self.y_scope = if index == 1 { 0 } else { 1 };
    }
    /// Read-only demo state and widget bounds for browser inspection.
    pub fn inspection(&self) -> serde_json::Value {
        serde_json::json!({
            "size": self.size,
            "yScope": self.y_scope,
            "titleScope": self.title_scope,
            "legendScope": self.legend_scope,
            "outer": self.outer,
            "legendBottom": self.legend_bottom,
            "missing": self.missing,
            "overlay": self.overlay,
            "preset": self.preset,
            "focus": self.widgets.focused().map(|target| serde_json::json!({
                "id": target.widget.as_str(),
                "item": target.item.as_ref().map(ChoiceItemId::as_str),
            })),
            "controls": self.widgets.semantics().iter().map(|control| serde_json::json!({
                "id": control.target.widget.as_str(),
                "item": control.target.item.as_ref().map(ChoiceItemId::as_str),
                "x": control.bounds.x,
                "y": control.bounds.y,
                "width": control.bounds.width,
                "height": control.bounds.height,
            })).collect::<Vec<_>>(),
        })
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
            7 => self.select_preset((self.preset + 1) % 3),
            _ => {}
        }
    }
}
