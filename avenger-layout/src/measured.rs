//! A measured layout, which keeps its measurements so that it can be solved again at other
//! sizes without asking them again.

use std::collections::HashSet;
use std::hash::Hash;

use crate::geometry::Size;
use crate::measure::{AnswerKey, History, Measure};
use crate::node::{Layout, SolveOptions};
use crate::solution::Solution;
use crate::solve::{self, Answers, Measurements, Prepared};
use crate::validate::LayoutError;

/// A layout solved with its measured leaves, which keeps every answer that it used.
///
/// For each frame of an interactive resize, [`remeasure`](Measured::remeasure) solves again at
/// another canvas size and asks [`Measure`] only for the answers that no longer hold, and
/// [`resize`](Measured::resize) solves again with the stored answers only. Both reuse the
/// prepared layout. [`Layout::measure_reusing`] measures a new layout and reuses the answers
/// that still hold.
#[derive(Clone, Debug)]
pub struct Measured<Id> {
    prepared: Prepared<Id>,
    options: SolveOptions,
    measurements: Measurements,
    history: History<AnswerKey<Id>>,
    solution: Solution<Id>,
}

impl<Id: Clone + Eq + Hash> Measured<Id> {
    fn new(
        layout: &Layout<Id>,
        options: &SolveOptions,
        measure: &mut impl Measure<Id>,
        mut history: History<AnswerKey<Id>>,
    ) -> Result<Self, LayoutError> {
        let prepared = Prepared::new(layout, options)?;
        let (measurements, solution) =
            measure_prepared(&prepared, *options, measure, &mut history)?;
        let present: HashSet<AnswerKey<Id>> = prepared
            .base
            .tree
            .nodes
            .iter()
            .filter_map(|node| {
                let id = node.id.as_ref()?;
                Some(AnswerKey::of(id, node.content_key.as_ref()))
            })
            .collect();
        history.retain(|key| present.contains(key));
        Ok(Self {
            prepared,
            options: *options,
            measurements,
            history,
            solution,
        })
    }

    /// The solution at the size that the layout was last measured at.
    pub fn solution(&self) -> &Solution<Id> {
        &self.solution
    }

    pub fn into_solution(self) -> Solution<Id> {
        self.solution
    }

    /// The options that the layout was last measured with.
    pub fn options(&self) -> &SolveOptions {
        &self.options
    }

    /// Solves the layout at a canvas of `size` without asking [`Measure`]: a leaf takes the
    /// stored answer that holds at its new size, or else the answer from the last measure, and
    /// the solution's [`stale`](Solution::stale) lists the leaves that took one of those. When
    /// the list is empty, the solution is the one that [`remeasure`](Self::remeasure) gives.
    pub fn resize(&self, size: Size) -> Result<Solution<Id>, LayoutError> {
        let options = self.sized(size)?;
        self.stored(options, false)
    }

    /// Solves the layout again at a canvas of `size`, asking `measure` only for the answers
    /// that no longer hold, and keeps the new solution and answers. The solution is the one
    /// that [`Layout::measure_reusing`] gives for the same layout, without preparing the layout
    /// again. On an error, the solution and options stay as they were.
    pub fn remeasure(
        &mut self,
        size: Size,
        measure: &mut impl Measure<Id>,
    ) -> Result<&Solution<Id>, LayoutError> {
        let options = self.sized(size)?;
        let mut history = std::mem::take(&mut self.history);
        let measured = measure_prepared(&self.prepared, options, measure, &mut history);
        self.history = history;
        (self.measurements, self.solution) = measured?;
        self.options = options;
        Ok(&self.solution)
    }

    /// The smallest canvas at which no leaf falls below its minimum, with the stored answers:
    /// the narrowest width that keeps every leaf at its minimum width, and the height at that
    /// width. A narrower canvas makes the solution overflow.
    pub fn min_size(&self) -> Result<Size, LayoutError> {
        let options = SolveOptions {
            width: None,
            height: None,
            explain: false,
            ..self.options
        };
        self.stored(options, true).map(|solution| solution.size())
    }

    /// Drops the stored answers of `id`, so that the next [`remeasure`](Self::remeasure) or
    /// [`Layout::measure_reusing`] asks for them again, while [`resize`](Self::resize) keeps
    /// the answers of the last measure. Call it when the leaf's content changes, such as a
    /// legend that gets new entries. A node with a [`ContentKey`](crate::ContentKey) stores
    /// its answers under the key instead, and a content change arrives as a new key.
    pub fn forget(&mut self, id: &Id) {
        self.history.forget(&AnswerKey::Id(id.clone()));
    }

    /// The options of the last measure at a canvas of `size`.
    fn sized(&self, size: Size) -> Result<SolveOptions, LayoutError> {
        let options = SolveOptions {
            width: Some(size.width),
            height: Some(size.height),
            ..self.options
        };
        crate::validate::validate_options(&options)?;
        Ok(options)
    }

    /// Solves with the stored answers only.
    fn stored(&self, options: SolveOptions, minimums: bool) -> Result<Solution<Id>, LayoutError> {
        let count = self.prepared.base.tree.nodes.len();
        let mut answers = Answers::Stored {
            history: &self.history,
            last: &self.measurements,
            stale: vec![[false; 2]; count],
        };
        let mut measurements = Measurements::new(count);
        solve::solve(
            &self.prepared,
            options,
            &mut measurements,
            &mut answers,
            minimums,
        )
    }
}

/// Solves a prepared layout, asking `measure` for the answers that `history` does not hold.
fn measure_prepared<Id: Clone + Eq + Hash>(
    prepared: &Prepared<Id>,
    options: SolveOptions,
    measure: &mut impl Measure<Id>,
    history: &mut History<AnswerKey<Id>>,
) -> Result<(Measurements, Solution<Id>), LayoutError> {
    let mut measurements = Measurements::new(prepared.base.tree.nodes.len());
    let mut answers = Answers::Live { measure, history };
    let solution = solve::solve(prepared, options, &mut measurements, &mut answers, false)?;
    Ok((measurements, solution))
}

impl<Id: Clone + Eq + Hash> Layout<Id> {
    /// Solves a layout that has no measured leaves.
    pub fn solve(&self, options: &SolveOptions) -> Result<Solution<Id>, LayoutError> {
        Ok(self.measure(options, &mut ())?.into_solution())
    }

    /// Solves the layout at the options' canvas, asking `measure` for the leaves whose size
    /// depends on the space they get.
    pub fn measure(
        &self,
        options: &SolveOptions,
        measure: &mut impl Measure<Id>,
    ) -> Result<Measured<Id>, LayoutError> {
        Measured::new(self, options, measure, History::default())
    }

    /// Measures the layout as [`measure`](Layout::measure) does, with the same solution, but
    /// first looks for an answer of `previous` that holds, and asks `measure` only when none
    /// does.
    pub fn measure_reusing(
        &self,
        previous: &Measured<Id>,
        options: &SolveOptions,
        measure: &mut impl Measure<Id>,
    ) -> Result<Measured<Id>, LayoutError> {
        Measured::new(self, options, measure, previous.history.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solution::Solution;

    fn send_sync<T: Send + Sync>() {}

    #[test]
    fn layouts_and_solutions_are_send_and_sync() {
        send_sync::<Layout<String>>();
        send_sync::<Measured<String>>();
        send_sync::<Solution<String>>();
    }
}
