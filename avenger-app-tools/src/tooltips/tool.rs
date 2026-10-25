//! Displays tooltips for instances found under the pointer.

use async_trait::async_trait;
use avenger_app::app::{Scenes, Tool, ToolHandler, ToolInstall};
use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_geometry::data::Datum;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use super::content::{fields_of, value};
use super::draw::{highlight, instance_color, tooltip, Content, Placement, TooltipStyle};
use super::error::AvengerTooltipError;
use super::target::{InstanceTarget, TooltipAnchor};

/// Displays a tooltip and outline for the first target that finds an instance.
///
/// Targets search lower scenes from top to bottom and skip stale trees. The tooltip
/// scene updates when requested or when a lower scene changes. Its marks are
/// noninteractive, so the tooltip does not block input to lower scenes.
#[derive(Clone)]
pub struct Tooltips {
    scene: String,
    z: i32,
    targets: Vec<InstanceTarget>,
    style: TooltipStyle,
}

/// The pointer position retained between tooltip updates.
#[derive(Debug, Default)]
pub struct TooltipState {
    pointer: Option<[f32; 2]>,
}

impl Tooltips {
    /// Create a tooltip tool with scene name `scene`, z-order `z`, and no targets.
    pub fn new(scene: &str, z: i32) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            targets: Vec::new(),
            style: TooltipStyle::default(),
        }
    }

    /// Set the targets in search order.
    pub fn over(mut self, targets: impl IntoIterator<Item = InstanceTarget>) -> Self {
        self.targets = targets.into_iter().collect();
        self
    }

    /// Set the tooltip box and outline style.
    pub fn with_style(mut self, style: TooltipStyle) -> Self {
        self.style = style;
        self
    }

    /// The tooltip at `pointer`, inside `bounds`, `[x, y, width, height]`, or no marks when no
    /// target finds an instance there.
    fn marks(
        &self,
        scenes: &Scenes<'_>,
        pointer: [f32; 2],
        bounds: [f32; 4],
    ) -> Result<Vec<SceneMark>, AvengerTooltipError> {
        let engine = scenes.text_engine();
        for target in &self.targets {
            for below in scenes.below(&self.scene) {
                if !below.is_current() {
                    continue;
                }
                let tree = below.tree();
                let Some(instance) = target.find(tree, pointer) else {
                    continue;
                };
                let mark = &instance.mark_instance;
                let Some(datum) = tree.datum(mark) else {
                    continue;
                };
                let swatch = (target.swatch)
                    .then(|| instance_color(mark, below.graph()))
                    .flatten();
                let content = content(target, &datum, swatch, engine)?;
                let placement = match target.anchor {
                    TooltipAnchor::Data => {
                        let envelope = instance.envelope();
                        let [[left, top], [right, bottom]] = [envelope.lower(), envelope.upper()];
                        Placement::Beside([left, top, right, bottom])
                    }
                    TooltipAnchor::Pointer => Placement::AtPointer(pointer),
                };
                let mut marks: Vec<SceneMark> = Vec::new();
                if let Some(outline) = highlight(instance, tree.clip(&mark.mark_path), &self.style)
                {
                    marks.push(outline.into());
                }
                marks.push(tooltip(&content, placement, bounds, &self.style, engine)?.into());
                let tooltips = SceneGroup {
                    name: "tooltips".to_string(),
                    interactive: Some(false),
                    marks,
                    ..Default::default()
                };
                return Ok(vec![tooltips.into()]);
            }
        }
        Ok(Vec::new())
    }
}

/// What a tooltip of `datum` shows for `target`: its header field's value, and a row for each of
/// its fields.
fn content(
    target: &InstanceTarget,
    datum: &Datum,
    swatch: Option<[f32; 4]>,
    engine: &LabelEngine,
) -> Result<Content, AvengerTooltipError> {
    let header = match &target.header {
        Some(field) => Some(value(datum, field, engine)?),
        None => None,
    };
    let rows = fields_of(target.fields.as_deref(), datum)
        .iter()
        .map(|field| Ok((field.title().to_string(), value(datum, field, engine)?)))
        .collect::<Result<_, AvengerTooltipError>>()?;
    Ok(Content {
        header,
        swatch,
        rows,
    })
}

#[async_trait]
impl<State: Send + Sync + 'static> Tool<State> for Tooltips {
    type State = TooltipState;

    fn install(&self, tool: &mut ToolInstall<'_, State, TooltipState>) {
        let follow = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        let pointer = FollowPointer {
            scene: self.scene.clone(),
        };
        tool.scene(&self.scene, self.z)
            .stream(&self.scene, follow, pointer);
    }

    /// Update the tooltip when requested or when a lower scene changes.
    /// Hit-test positions already include scene origins, so the tooltip scene uses a zero
    /// origin. Formatting errors, such as a missing data column, are logged and hide the tooltip.
    async fn update(
        &self,
        _app: &State,
        own: &mut TooltipState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let below_changed = scenes.below(&self.scene).any(|below| below.changed());
        if !scenes.requested(&self.scene) && !below_changed {
            return Ok(());
        }
        let [width, height] = scenes.size();
        let marks = match own.pointer {
            Some(pointer) => (self.marks(scenes, pointer, [0.0, 0.0, width, height]))
                .unwrap_or_else(|e| {
                    log::warn!("Failed to draw a tooltip: {e}");
                    Vec::new()
                }),
            None => Vec::new(),
        };
        if marks.is_empty() {
            return scenes.clear(&self.scene);
        }
        let scene = SceneGraph {
            marks,
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(&self.scene, scene.into())
    }
}

/// Tracks pointer changes and requests tooltip updates.
/// Ignoring repeated positions avoids scheduling a redundant render before a wheel
/// event, which the host could then drop while that render is pending.
struct FollowPointer {
    scene: String,
}

#[async_trait]
impl<State: Send + Sync + 'static> ToolHandler<State, TooltipState> for FollowPointer {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _app: &mut State,
        own: &mut TooltipState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let position = event.position();
        if own.pointer == position {
            return Rebuild::none();
        }
        own.pointer = position;
        Rebuild::scene(&self.scene)
    }
}
