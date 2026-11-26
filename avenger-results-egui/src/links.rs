//! The selections that the live charts of a view share: each change that a live chart reports
//! gives its clauses to the view's other live charts, and a live chart that builds later takes
//! them too, so a brush in one chart restyles every chart of the result.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use avenger_chart_app::ChartChange;
use avenger_selection::Clause;
use eframe::egui;

use crate::live::{LiveChart, OnChange};

/// A change that a live chart reported, with the id of the chart.
type Reported = (egui::Id, ChartChange);

pub(crate) struct Links {
    /// The clauses of each selection, as the last change that wrote it left them.
    clauses: BTreeMap<String, Vec<Clause>>,
    /// The changes that the live charts reported, by the id of the chart that reported each.
    changes: (Sender<Reported>, Receiver<Reported>),
}

impl Default for Links {
    fn default() -> Self {
        Self {
            clauses: BTreeMap::new(),
            changes: channel(),
        }
    }
}

impl Links {
    /// The callback of the changes of the live chart `id`, which an app calls from its own
    /// thread: it sends each change to the view and wakes it.
    pub(crate) fn on_change(&self, id: egui::Id, ctx: &egui::Context) -> OnChange {
        let (changes, ctx) = (self.changes.0.clone(), ctx.clone());
        Arc::new(move |change: &ChartChange| {
            let _ = changes.send((id, change.clone()));
            ctx.request_repaint();
        })
    }

    /// Gives `chart`, which just built, the clauses of every selection.
    pub(crate) fn join(&self, chart: &LiveChart) {
        for (name, clauses) in &self.clauses {
            chart.set_clauses(name, clauses.clone());
        }
    }

    /// The changes that the live charts reported since the last call, each after its clauses
    /// went to the charts of `charts` other than the chart that reported it.
    pub(crate) fn take<'c>(&mut self, charts: impl Fn() -> Vec<&'c LiveChart>) -> Vec<ChartChange> {
        let mut taken = Vec::new();
        while let Ok((source, change)) = self.changes.1.try_recv() {
            for (name, clauses) in &change.selections {
                self.clauses.insert(name.clone(), clauses.clone());
                for chart in charts().into_iter().filter(|chart| chart.id() != source) {
                    chart.set_clauses(name, clauses.clone());
                }
            }
            taken.push(change);
        }
        taken
    }
}
