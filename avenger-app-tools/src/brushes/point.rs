//! Point brushes, which select instances by clicking them.

use std::sync::Arc;

use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;

use super::binding::Active;
use super::brush::{Brush, Clicked, ErasedBrush, Own};
use super::instances::{fixed, Instances, InstancesOf};

type ValueOf<State, I> = Box<dyn Fn(&State) -> Vec<I> + Send + Sync>;
type Report<State, I> = Box<dyn Fn(&mut State, Vec<I>) -> Rebuild + Send + Sync>;
type AreasOf<State, A> = Box<dyn Fn(&State) -> Vec<(A, Rect)> + Send + Sync>;
type AreaInstancesOf<State, A, I> = Box<dyn Fn(&State, &A) -> Instances<I> + Send + Sync>;
type AreaValueOf<State, A, I> = Box<dyn Fn(&State, &A) -> Vec<I> + Send + Sync>;
type AreaReport<State, A, I> = Box<dyn Fn(&mut State, &A, Vec<I>) -> Rebuild + Send + Sync>;

/// Selects instances in the scenes below the brushes by clicking them.
///
/// A click selects the topmost instance under the pointer, by its id, and replaces the
/// selection. A Shift+click adds the instance's id, or removes it if the selection holds it. A
/// click away from every instance clears the selection. The brush acts on a click whose pointer
/// stays within 4 pixels of its press, so the release of a pan selects nothing. It never claims
/// presses, and draws nothing: the app highlights the selected instances in its own marks.
///
/// The brush searches the trees of the scenes below the brushes. A click selects nothing while a
/// target tree is stale, as during a zoom that hasn't settled, since the tree doesn't show which
/// instance is on screen. Request a scene's tree whenever its marks settle, as after a pan.
pub struct PointBrush<State, I> {
    instances: InstancesOf<State, I>,
    value: ValueOf<State, I>,
    on_change: Report<State, I>,
    active: Active<State>,
}

impl<State, I> PointBrush<State, I>
where
    State: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush that selects `instances`.
    pub fn new(instances: Instances<I>) -> Self {
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
        instances: impl Fn(&State) -> Instances<I> + Send + Sync + 'static,
    ) -> Self {
        self.instances = Box::new(instances);
        self
    }

    /// Set how the brush reads the selected ids from app state, in selection order.
    pub fn value(mut self, value: impl Fn(&State) -> Vec<I> + Send + Sync + 'static) -> Self {
        self.value = Box::new(value);
        self
    }

    /// Set the callback for a click that changes the selection. Store the ids in app state and
    /// return the scenes to rebuild, such as the chart scene with its tree.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, Vec<I>) -> Rebuild + Send + Sync + 'static,
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

impl<State, I> ErasedBrush<State> for PointBrush<State, I>
where
    State: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
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
        let id = (self.instances)(app).at(trees.below(), at).ok()?;
        let selected = (self.value)(app);
        let next = next(id.as_ref(), &selected, shift);
        let rebuild = if next != selected {
            (self.on_change)(app, next)
        } else {
            Rebuild::none()
        };
        Some(Clicked {
            rebuild,
            hit: id.is_some(),
        })
    }
}

impl<State, I> From<PointBrush<State, I>> for Brush<State>
where
    State: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
{
    fn from(brush: PointBrush<State, I>) -> Self {
        Brush(Arc::new(brush))
    }
}

/// Selects instances by clicking them in each of a set of areas, as a [`PointBrush`] does
/// anywhere. The app names each area with a key of type `A`, and each area has its own instances
/// and its own selection. A click acts on each area that contains it, among the area's instances,
/// so a click away from every instance clears the selections of those areas alone.
pub struct KeyedPointBrush<State, A, I> {
    areas: AreasOf<State, A>,
    instances: AreaInstancesOf<State, A, I>,
    value: AreaValueOf<State, A, I>,
    on_change: AreaReport<State, A, I>,
    active: Active<State>,
}

impl<State, A, I> KeyedPointBrush<State, A, I>
where
    State: Send + Sync + 'static,
    A: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush. Set its areas, the instances of each area, and the selection of each.
    pub fn new() -> Self {
        Self {
            areas: Box::new(|_| Vec::new()),
            instances: Box::new(|_, _| Instances::marks(Vec::<String>::new())),
            value: Box::new(|_, _| Vec::new()),
            on_change: Box::new(|_, _, _| Rebuild::none()),
            active: Box::new(|_| true),
        }
    }

    /// Set the areas where the brush acts on clicks, each with its key and its rectangle in
    /// scene coordinates.
    pub fn areas(
        mut self,
        areas: impl Fn(&State) -> Vec<(A, Rect)> + Send + Sync + 'static,
    ) -> Self {
        self.areas = Box::new(areas);
        self
    }

    /// Set the instances that the brush selects in each area, which the brush reads at each
    /// click.
    pub fn instances(
        mut self,
        instances: impl Fn(&State, &A) -> Instances<I> + Send + Sync + 'static,
    ) -> Self {
        self.instances = Box::new(instances);
        self
    }

    /// Set how the brush reads the selected ids of each area from app state, in selection
    /// order.
    pub fn value(mut self, value: impl Fn(&State, &A) -> Vec<I> + Send + Sync + 'static) -> Self {
        self.value = Box::new(value);
        self
    }

    /// Set the callback for a click that changes the selection of an area. Store the ids in
    /// app state and return the scenes to rebuild, such as the chart scene with its tree.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, &A, Vec<I>) -> Rebuild + Send + Sync + 'static,
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

impl<State, A, I> Default for KeyedPointBrush<State, A, I>
where
    State: Send + Sync + 'static,
    A: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<State, A, I> ErasedBrush<State> for KeyedPointBrush<State, A, I>
where
    State: Send + Sync + 'static,
    A: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
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
        let areas: Vec<A> = ((self.areas)(app).into_iter())
            .filter(|(_, rect)| rect.contains(at))
            .map(|(area, _)| area)
            .collect();
        let mut clicked: Option<Clicked> = None;
        for area in areas {
            let Ok(id) = (self.instances)(app, &area).at(trees.below(), at) else {
                continue;
            };
            let selected = (self.value)(app, &area);
            let next = next(id.as_ref(), &selected, shift);
            let rebuild = if next != selected {
                (self.on_change)(app, &area, next)
            } else {
                Rebuild::none()
            };
            let clicked = clicked.get_or_insert(Clicked {
                rebuild: Rebuild::none(),
                hit: false,
            });
            clicked.rebuild.merge(rebuild);
            clicked.hit |= id.is_some();
        }
        clicked
    }
}

impl<State, A, I> From<KeyedPointBrush<State, A, I>> for Brush<State>
where
    State: Send + Sync + 'static,
    A: Send + Sync + 'static,
    I: Clone + PartialEq + Send + Sync + 'static,
{
    fn from(brush: KeyedPointBrush<State, A, I>) -> Self {
        Brush(Arc::new(brush))
    }
}

/// The selection after a click on the instance with `id`, or away from every instance, of the
/// selection `selected`. A click replaces the selection, a Shift+click adds or removes the id,
/// and a click away from every instance clears the selection, unless Shift is held.
fn next<I: Clone + PartialEq>(id: Option<&I>, selected: &[I], shift: bool) -> Vec<I> {
    match (id, shift) {
        (Some(id), false) => vec![id.clone()],
        (Some(id), true) if selected.contains(id) => {
            (selected.iter()).filter(|i| *i != id).cloned().collect()
        }
        (Some(id), true) => selected.iter().chain([id]).cloned().collect(),
        (None, false) => Vec::new(),
        (None, true) => selected.to_vec(),
    }
}
