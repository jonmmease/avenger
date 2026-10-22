//! The tooltip layer: it follows the pointer, and shows a tooltip of what its targets find under
//! it.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use avenger_app::app::{BuildContext, BuiltScene, Layer};
use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::{EventStreamHandler, LayerTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_geometry::data::Datum;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::content::{fields_of, value};
use crate::draw::{highlight, instance_color, tooltip, Content, Placement, TooltipStyle};
use crate::error::AvengerTooltipError;
use crate::target::{InstanceTarget, TooltipAnchor};

/// A layer of tooltips over the layers below it. At the pointer, the first of its targets that
/// finds an instance shows a tooltip of the instance's row, with an outline around the instance.
/// A target searches the trees below from the top layer down, and skips a stale tree, as during a
/// pan or a zoom, since a stale tree places the instances where they were. The layer builds again
/// only when the pointer moves, and its marks take no hits, so every event reaches the layers
/// below. Clones share the pointer.
#[derive(Clone)]
pub struct Tooltips {
    targets: Vec<InstanceTarget>,
    style: TooltipStyle,
    /// The pointer's position in the scene, while it lies in the window.
    pointer: Arc<Mutex<Option<[f32; 2]>>>,
}

impl Tooltips {
    /// Tooltips that try `targets` in order.
    pub fn over(targets: impl IntoIterator<Item = InstanceTarget>) -> Self {
        Self {
            targets: targets.into_iter().collect(),
            style: TooltipStyle::default(),
            pointer: Arc::default(),
        }
    }

    pub fn with_style(mut self, style: TooltipStyle) -> Self {
        self.style = style;
        self
    }

    /// The tooltip at `pointer`, inside `bounds`, `[x, y, width, height]`, or no marks when no
    /// target finds an instance there.
    fn marks(
        &self,
        context: &BuildContext<'_>,
        pointer: [f32; 2],
        bounds: [f32; 4],
    ) -> Result<Vec<SceneMark>, AvengerTooltipError> {
        let (trees, scenes) = (context.trees_below(), context.scenes_below());
        let engine = context.text_engine();
        for target in &self.targets {
            for layer in (0..trees.len()).rev() {
                if !context.tree_is_current(layer) {
                    continue;
                }
                let tree = &trees[layer];
                let Some(instance) = target.find(tree, pointer) else {
                    continue;
                };
                let mark = &instance.mark_instance;
                let Some(datum) = tree.datum(mark) else {
                    continue;
                };
                let swatch = (target.swatch)
                    .then(|| instance_color(mark, &scenes[layer]))
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
impl<State: Clone + Send + Sync + 'static> Layer<State> for Tooltips {
    /// The tooltip at the pointer, in a scene the size of the layers below. Its marks stand where
    /// the trees below and the pointer place them, which a scene's origin has already moved, so the
    /// scene has no origin of its own. A tooltip that can't describe its instance, such as one
    /// whose field names a column that the rows lack, logs why and draws nothing.
    async fn build(
        &self,
        _state: &mut State,
        context: &BuildContext<'_>,
    ) -> Result<BuiltScene, AvengerAppError> {
        let (width, height) = (context.scenes_below().first())
            .map_or((0.0, 0.0), |scene| (scene.width, scene.height));
        let pointer = *self.pointer.lock().unwrap_or_else(PoisonError::into_inner);
        let marks = match pointer {
            Some(pointer) => (self.marks(context, pointer, [0.0, 0.0, width, height]))
                .unwrap_or_else(|e| {
                    log::warn!("Failed to draw a tooltip: {e}");
                    Vec::new()
                }),
            None => Vec::new(),
        };
        Ok(SceneGraph {
            marks,
            width,
            height,
            origin: [0.0; 2],
        }
        .into())
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        let follow = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        vec![(follow, Arc::new(FollowPointer(self.pointer.clone())))]
    }
}

/// Keeps the pointer's position for the tooltips, and builds their layer again when it changes. A
/// scroll comes with a move to where the pointer already is, and a build for it would leave a
/// render pending that the host drops the scroll's wheel turn for.
struct FollowPointer(Arc<Mutex<Option<[f32; 2]>>>);

#[async_trait]
impl<State: Clone + Send + Sync + 'static> EventStreamHandler<State> for FollowPointer {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut State,
        _trees: &LayerTrees<'_>,
    ) -> UpdateStatus {
        let mut pointer = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let position = event.position();
        let moved = *pointer != position;
        *pointer = position;
        UpdateStatus {
            rerender: moved,
            rebuild_geometry: false,
        }
    }
}
