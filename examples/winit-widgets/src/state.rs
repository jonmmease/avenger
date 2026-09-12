use avenger_common::time::{Duration, Instant};
use avenger_text::{
    TextEngine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextSyntaxMode},
};
use avenger_widgets::prelude::*;
use std::collections::BTreeSet;

pub const DEFAULT_SOURCE: &str = "*Radius* $sqrt(x^2+y^2)$";
#[derive(Clone)]
pub struct State {
    pub size: [f32; 2],
    pub engine: TextEngine,
    pub widgets: WidgetRuntime,
    pub layers: BTreeSet<ChoiceItemId>,
    pub palette: ChoiceItemId,
    pub marker_size: f64,
    pub opacity: f64,
    pub title: String,
    pub source: String,
    pub accepted_source: String,
    pub annotation_error: Option<String>,
    pub locked: bool,
    pub last_action: String,
    pub committed_opacity: f64,
    pub pan: [f32; 2],
    pub plot_drag: Option<([f32; 2], [f32; 2])>,
}
impl State {
    pub fn new(engine: TextEngine) -> Self {
        Self {
            size: [1120.0, 840.0],
            engine,
            widgets: WidgetRuntime::new(),
            layers: ["grid".into(), "points".into(), "annotations".into()]
                .into_iter()
                .collect(),
            palette: "ocean".into(),
            marker_size: 8.0,
            opacity: 0.80,
            title: "Signal and variation".into(),
            source: DEFAULT_SOURCE.into(),
            accepted_source: DEFAULT_SOURCE.into(),
            annotation_error: None,
            locked: false,
            last_action: "Ready to edit".into(),
            committed_opacity: 0.80,
            pan: [0.0; 2],
            plot_drag: None,
        }
    }
    pub fn plot(&self) -> Rect {
        Rect::new(
            66.0,
            152.0,
            (self.size[0] - 464.0).max(300.0),
            (self.size[1] - 260.0).max(400.0),
        )
    }
    pub fn controls(&self) -> Vec<WidgetSpec> {
        let enabled = !self.locked;
        vec![
            Checkbox::new("lock", "Lock styling", self.locked).into(),
            CheckboxGroup::new(
                "layers",
                vec![
                    ChoiceItem::new("grid", "Grid lines"),
                    ChoiceItem::new("points", "Point marks"),
                    ChoiceItem::new("annotations", "Annotation"),
                ],
                self.layers.clone(),
            )
            .label("Visible layers")
            .enabled(enabled)
            .into(),
            RadioGroup::new(
                "palette",
                vec![
                    ChoiceItem::new("ocean", "Ocean"),
                    ChoiceItem::new("sunset", "Sunset"),
                    ChoiceItem::new("mono", "Mono"),
                ],
                Some(self.palette.clone()),
            )
            .label("Palette")
            .enabled(enabled)
            .orientation(ChoiceOrientation::Horizontal)
            .into(),
            Slider::new(
                "size",
                SliderDomain::stepped(2.0, 15.0, 3.0).unwrap(),
                self.marker_size,
            )
            .value_label(format!("{:.0} px", self.marker_size))
            .semantic_name("Marker size")
            .enabled(enabled)
            .into(),
            Slider::new(
                "opacity",
                SliderDomain::continuous(0.0, 1.0).unwrap(),
                self.opacity,
            )
            .value_label(format!("{:.0}%", self.opacity * 100.0))
            .semantic_name("Opacity")
            .enabled(enabled)
            .into(),
            TextInput::new("title", &self.title)
                .semantic_name("Plot title")
                .placeholder("Give this plot a title")
                .enabled(enabled)
                .into(),
            TextInput::new("source", &self.source)
                .semantic_name("Typst annotation source")
                .commit_policy(TextCommitPolicy::Debounced(Duration::from_millis(350)))
                .invalid(self.annotation_error.is_some())
                .enabled(enabled)
                .into(),
            Button::new("reset", "Reset defaults")
                .variant(ButtonVariant::Accent)
                .into(),
        ]
    }
    pub fn apply(
        &mut self,
        events: Vec<WidgetEvent>,
        now: Instant,
    ) -> Vec<avenger_eventstream::runtime::RuntimeHostCommand> {
        let mut commands = Vec::new();
        for event in events {
            let id = event.id.as_str();
            match event.action {
                WidgetAction::CheckedChanged { value } if id == "lock" => {
                    self.locked = value;
                    self.last_action = if value {
                        "Styling locked"
                    } else {
                        "Styling unlocked"
                    }
                    .into();
                }
                WidgetAction::CheckedItemsChanged { item, checked } => {
                    self.layers = checked;
                    self.last_action = format!(
                        "{} layer {}",
                        item.as_str(),
                        if self.layers.contains(&item) {
                            "shown"
                        } else {
                            "hidden"
                        }
                    );
                }
                WidgetAction::SelectionChanged { item } => {
                    self.palette = item;
                    self.last_action = format!("{} palette selected", self.palette.as_str());
                }
                WidgetAction::SliderChanged { value } if id == "size" => {
                    self.marker_size = value;
                    self.last_action = format!("Marker size: {value:.0} px");
                }
                WidgetAction::SliderChanged { value } if id == "opacity" => {
                    self.opacity = value;
                    self.last_action = format!("Opacity preview: {:.0}%", value * 100.0);
                }
                WidgetAction::SliderCommitted { value } => {
                    if id == "opacity" {
                        self.committed_opacity = value;
                    }
                    self.last_action = format!(
                        "{} committed: {:.0}{}",
                        if id == "opacity" {
                            "Opacity"
                        } else {
                            "Marker size"
                        },
                        if id == "opacity" {
                            value * 100.0
                        } else {
                            value
                        },
                        if id == "opacity" { "%" } else { " px" }
                    );
                }
                WidgetAction::SliderCancelled { reason, .. } => {
                    self.last_action = format!("Slider cancelled: {reason:?}")
                }
                WidgetAction::TextChanged { value } if id == "title" => {
                    self.title = value;
                    self.last_action = "Title updated".into();
                }
                WidgetAction::TextChanged { value } if id == "source" => {
                    self.source = value;
                    self.annotation_error = None;
                    self.last_action = "Annotation draft changed".into();
                }
                WidgetAction::TextCommitted { value, .. } if id == "source" => {
                    match self.engine.measure_bounds(&annotation_config(&value)) {
                        Ok(_) => {
                            self.accepted_source = value;
                            self.annotation_error = None;
                            self.last_action = "Annotation applied".into();
                        }
                        Err(error) => {
                            self.annotation_error = Some(error.to_string());
                            self.last_action = "Annotation needs a correction".into();
                        }
                    }
                }
                WidgetAction::TextSubmitted { .. } => self.last_action = format!("{id} submitted"),
                WidgetAction::Activated if id == "reset" => {
                    let defaults = Self::new(self.engine.clone());
                    self.layers = defaults.layers;
                    self.palette = defaults.palette;
                    self.marker_size = defaults.marker_size;
                    self.opacity = defaults.opacity;
                    self.committed_opacity = defaults.committed_opacity;
                    self.title = defaults.title;
                    self.source = defaults.source;
                    self.accepted_source = defaults.accepted_source;
                    self.annotation_error = None;
                    self.locked = false;
                    self.pan = [0.0; 2];
                    self.last_action = "Defaults restored".into();
                    for (id, value) in [
                        ("title", self.title.clone()),
                        ("source", self.source.clone()),
                    ] {
                        if let Ok(update) = self.widgets.reset_text(id, value, now) {
                            commands.extend(update.status.commands);
                        }
                    }
                }
                _ => {}
            }
        }
        commands
    }
    /// Read-only demo diagnostics for tests and browser inspection.
    pub fn inspection(&self) -> serde_json::Value {
        serde_json::json!({"layers":self.layers.iter().map(ChoiceItemId::as_str).collect::<Vec<_>>(),"palette":self.palette.as_str(),"size":self.marker_size,"opacity":self.opacity,"committedOpacity":self.committed_opacity,"title":self.title,"source":self.source,"acceptedSource":self.accepted_source,"invalid":self.annotation_error.is_some(),"locked":self.locked,"pan":self.pan,"lastAction":self.last_action,"focus":self.widgets.focused().map(|t|serde_json::json!({"id":t.widget.as_str(),"item":t.item.as_ref().map(ChoiceItemId::as_str)})),"controls":self.widgets.semantics().iter().map(|s|serde_json::json!({"id":s.target.widget.as_str(),"item":s.target.item.as_ref().map(ChoiceItemId::as_str),"x":s.bounds.x,"y":s.bounds.y,"width":s.bounds.width,"height":s.bounds.height})).collect::<Vec<_>>()})
    }
}
pub fn annotation_config(text: &str) -> TextMeasurementConfig<'_> {
    TextMeasurementConfig {
        text,
        font: "sans-serif",
        font_size: 18.0,
        font_weight: FontWeight::default(),
        font_style: FontStyle::Normal,
        syntax_mode: TextSyntaxMode::TypstMarkup,
        params: avenger_text::empty_label_params(),
        number_locale: None,
        number_locale_specs: None,
        datetime_locale: None,
        datetime_timezone: None,
        datetime_locale_specs: None,
    }
}
