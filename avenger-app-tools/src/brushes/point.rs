//! Point brushes, which select instances by clicking them.

use std::sync::Arc;

use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;

use super::binding::Active;
use super::brush::{Brush, Clicked, ErasedBrush, Own};
use super::instances::{fixed, Instances, InstancesOf};

type ValueOf<State, K> = Box<dyn Fn(&State) -> Vec<K> + Send + Sync>;
type Report<State, K> = Box<dyn Fn(&mut State, Vec<K>) -> Rebuild + Send + Sync>;

/// Selects instances in the scenes below the brushes by clicking them.
///
/// A click selects the topmost instance under the pointer, by its key, and replaces the
/// selection. A Shift+click adds the instance's key, or removes it if the selection holds it. A
/// click away from every instance clears the selection. The brush acts on a click whose pointer
/// stays within 4 pixels of its press, so the release of a pan selects nothing. It never claims
/// presses, and draws nothing: the app highlights the selected instances in its own marks.
///
/// The brush searches the trees of the scenes below the brushes. A click selects nothing while a
/// target tree is stale, as during a zoom that hasn't settled, since the tree doesn't show which
/// instance is on screen. Request a scene's tree whenever its marks settle, as after a pan.
pub struct PointBrush<State, K> {
    instances: InstancesOf<State, K>,
    value: ValueOf<State, K>,
    on_change: Report<State, K>,
    active: Active<State>,
}

impl<State, K> PointBrush<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush that selects `instances`.
    pub fn new(instances: Instances<K>) -> Self {
        Self {
            instances: fixed(instances),
            value: Box::new(|_| Vec::new()),
            on_change: Box::new(|_, _| Rebuild::none()),
            active: Box::new(|_| true),
        }
    }

    /// Set the instances that the brush selects from the app state, which the brush reads at
    /// each click, so that an app whose marks change, such as an app that swaps its charts,
    /// selects among the marks that it draws.
    pub fn instances_from(
        mut self,
        instances: impl Fn(&State) -> Instances<K> + Send + Sync + 'static,
    ) -> Self {
        self.instances = Box::new(instances);
        self
    }

    /// Set how the brush reads the selected keys from app state, in selection order.
    pub fn value(mut self, value: impl Fn(&State) -> Vec<K> + Send + Sync + 'static) -> Self {
        self.value = Box::new(value);
        self
    }

    /// Set the callback for a click that changes the selection. Store the keys in app state and
    /// return the scenes to rebuild, such as the chart scene with its tree.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, Vec<K>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Box::new(on_change);
        self
    }

    /// Set when the brush acts on clicks, such as in a selection mode. Defaults to always.
    pub fn active(mut self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.active = Box::new(active);
        self
    }
}

impl<State, K> ErasedBrush<State> for PointBrush<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    fn new_state(&self) -> Own {
        Box::new(())
    }

    fn click(
        &self,
        app: &mut State,
        _own: &mut Own,
        at: [f32; 2],
        shift: bool,
        trees: &SceneTrees<'_>,
    ) -> Option<Clicked> {
        if !(self.active)(app) {
            return None;
        }
        let key = (self.instances)(app).at(trees.below(), at).ok()?;
        let selected = (self.value)(app);
        let next = match (&key, shift) {
            (Some(key), false) => vec![key.clone()],
            (Some(key), true) if selected.contains(key) => {
                (selected.iter()).filter(|k| *k != key).cloned().collect()
            }
            (Some(key), true) => selected.iter().chain([key]).cloned().collect(),
            (None, false) => Vec::new(),
            (None, true) => selected.clone(),
        };
        let rebuild = if next != selected {
            (self.on_change)(app, next)
        } else {
            Rebuild::none()
        };
        Some(Clicked {
            rebuild,
            hit: key.is_some(),
        })
    }
}

impl<State, K> From<PointBrush<State, K>> for Brush<State>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    fn from(brush: PointBrush<State, K>) -> Self {
        Brush(Arc::new(brush))
    }
}
