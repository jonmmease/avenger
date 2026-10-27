//! The bindings between brushes and the values that the app stores, and the changes that
//! gestures make to them.

use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;

pub(crate) type Areas<State, A> = Box<dyn Fn(&State) -> Vec<(A, Rect)> + Send + Sync>;
pub(crate) type ValueOf<State, A, V> = Box<dyn Fn(&State, &A) -> V + Send + Sync>;
pub(crate) type Report<State, A, V> = Box<dyn Fn(&mut State, &A, V) -> Rebuild + Send + Sync>;
pub(crate) type Active<State> = Box<dyn Fn(&State) -> bool + Send + Sync>;

/// A brush's areas, how it reads each area's value from app state and reports the next one, and
/// when it claims presses.
pub(crate) struct Binding<State, A, V> {
    pub(crate) areas: Areas<State, A>,
    pub(crate) value: ValueOf<State, A, V>,
    pub(crate) on_change: Report<State, A, V>,
    pub(crate) on_settle: Report<State, A, V>,
    pub(crate) active: Active<State>,
}

impl<State, A, V> Default for Binding<State, A, V>
where
    State: 'static,
    A: 'static,
    V: Default + 'static,
{
    fn default() -> Self {
        Self {
            areas: Box::new(|_| Vec::new()),
            value: Box::new(|_, _| V::default()),
            on_change: Box::new(|_, _, _| Rebuild::none()),
            on_settle: Box::new(|_, _, _| Rebuild::none()),
            active: Box::new(|_| true),
        }
    }
}

impl<State, A, V> Binding<State, A, V>
where
    A: Clone + PartialEq,
    V: Clone + PartialEq,
{
    /// The areas that have room for a selection.
    pub(crate) fn usable_areas(&self, app: &State) -> Vec<(A, Rect)> {
        let mut areas = (self.areas)(app);
        areas.retain(|(_, area)| area.width > 0.0 && area.height > 0.0);
        areas
    }

    /// The areas where a press can start a gesture: none while the brush is inactive.
    pub(crate) fn pressable_areas(&self, app: &State) -> Vec<(A, Rect)> {
        if (self.active)(app) {
            self.usable_areas(app)
        } else {
            Vec::new()
        }
    }

    /// The rectangle of the area `key`, while it has room for a selection.
    pub(crate) fn rect(&self, app: &State, key: &A) -> Option<Rect> {
        (self.usable_areas(app).into_iter())
            .find(|(area, _)| area == key)
            .map(|(_, rect)| rect)
    }

    /// Start a gesture's change to the value of the area `area`.
    pub(crate) fn change(&self, app: &State, area: A) -> Change<A, V> {
        let others = (self.usable_areas(app).into_iter())
            .filter(|(key, _)| *key != area)
            .map(|(key, _)| {
                let value = (self.value)(app, &key);
                (key, value)
            })
            .collect();
        let start = (self.value)(app, &area);
        Change {
            area,
            current: start.clone(),
            start,
            reported: None,
            others,
        }
    }
}

/// The change that a gesture makes to the value of its area.
pub(crate) struct Change<A, V> {
    pub(crate) area: A,
    /// The value at the press, which a cancellation restores.
    pub(crate) start: V,
    /// The value at the pointer's last move, which the gesture reports at most once per change
    /// interval.
    current: V,
    /// The value that the gesture last reported through `on_change`, if any.
    reported: Option<V>,
    /// The other areas' values at the press, which a cancellation restores where a binding
    /// changed them with this area's value.
    others: Vec<(A, V)>,
}

impl<A, V> Change<A, V>
where
    A: Clone + PartialEq,
    V: Clone + PartialEq,
{
    /// The value that the gesture last reported, or the value at the press.
    fn last(&self) -> &V {
        self.reported.as_ref().unwrap_or(&self.start)
    }

    /// The value that a change interval keeps from the app while it waits to report it.
    pub(crate) fn kept(&self) -> Option<&V> {
        (self.current != *self.last()).then_some(&self.current)
    }

    /// Set the value at the pointer's last move to `next`, and report it through `on_change` if
    /// `report`. Return `None` when the value stays the same.
    pub(crate) fn update<State>(
        &mut self,
        app: &mut State,
        binding: &Binding<State, A, V>,
        next: V,
        report: bool,
    ) -> Option<Rebuild> {
        if next == self.current {
            return None;
        }
        self.current = next;
        if !report {
            return Some(Rebuild::none());
        }
        Some(self.report(app, binding).unwrap_or_default())
    }

    /// Report the value at the pointer's last move through `on_change`, if the gesture hasn't
    /// reported it.
    pub(crate) fn report<State>(
        &mut self,
        app: &mut State,
        binding: &Binding<State, A, V>,
    ) -> Option<Rebuild> {
        if self.current == *self.last() {
            return None;
        }
        self.reported = Some(self.current.clone());
        Some((binding.on_change)(app, &self.area, self.current.clone()))
    }

    /// End a drag: report its last value, and settle the value if the gesture reported one.
    pub(crate) fn settle<State>(
        mut self,
        app: &mut State,
        binding: &Binding<State, A, V>,
    ) -> Rebuild {
        let mut rebuild = self.report(app, binding).unwrap_or_default();
        if let Some(reported) = self.reported {
            rebuild.merge((binding.on_settle)(app, &self.area, reported));
        }
        rebuild
    }

    /// End a click: clear the area's value, unless it is empty.
    pub(crate) fn clear<State>(self, app: &mut State, binding: &Binding<State, A, V>) -> Rebuild
    where
        V: Default,
    {
        let empty = V::default();
        if self.start == empty {
            return Rebuild::none();
        }
        let mut rebuild = (binding.on_change)(app, &self.area, empty.clone());
        rebuild.merge((binding.on_settle)(app, &self.area, empty));
        rebuild
    }

    /// End a gesture with each area's value at the press, if the gesture reported a value: the
    /// gesture's area, then each other area that a binding changed with it, as one selection for
    /// the figure does. Settle the gesture's area and each restored area.
    pub(crate) fn cancel<State>(self, app: &mut State, binding: &Binding<State, A, V>) -> Rebuild {
        let mut rebuild = Rebuild::none();
        if self.reported.is_none() {
            return rebuild;
        }
        let at_press = std::iter::once((true, self.area, self.start)).chain(
            self.others
                .into_iter()
                .map(|(key, then)| (false, key, then)),
        );
        let mut settled = Vec::new();
        for (own, key, then) in at_press {
            let restored = (binding.value)(app, &key) != then;
            if restored {
                rebuild.merge((binding.on_change)(app, &key, then.clone()));
            }
            if restored || own {
                settled.push((key, then));
            }
        }
        for (key, then) in settled {
            rebuild.merge((binding.on_settle)(app, &key, then));
        }
        rebuild
    }
}
