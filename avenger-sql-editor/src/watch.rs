//! The watch of a project's directory on disk: the paths of the files that change under it, in
//! bursts that end when no change comes for [`QUIET`], which the editor maps to the data scripts
//! that read them with [`Workspace::watched`](crate::workspace::Workspace::watched).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::event::{EventKind, MetadataKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// How long a burst of changes waits for its next change before it ends.
pub const QUIET: Duration = Duration::from_millis(200);

/// How long the changes of a file that the editor wrote itself are not changes.
const OWN: Duration = Duration::from_secs(2);

/// The watch of a directory and its directories in turn, which ends when it drops.
pub struct Watch {
    _watcher: RecommendedWatcher,
    changes: Receiver<PathBuf>,
    /// The paths of the burst that has not ended.
    pending: BTreeSet<PathBuf>,
    /// When the last change of the burst came.
    last: Option<Instant>,
    /// The files that the editor wrote, and when.
    own: Vec<(PathBuf, Instant)>,
}

impl Watch {
    /// Watches the directory `root`, and asks `ctx` for a repaint when a file under it is
    /// created, changes, or is removed. A read of a file, which a load of the files makes, is not
    /// a change.
    pub fn new(root: &Path, ctx: egui::Context) -> notify::Result<Watch> {
        let (sender, changes) = channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else {
                    return;
                };
                let change = match event.kind {
                    EventKind::Create(_) | EventKind::Remove(_) => true,
                    EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => false,
                    EventKind::Modify(_) | EventKind::Any => true,
                    EventKind::Access(_) | EventKind::Other => false,
                };
                if change {
                    for path in event.paths {
                        let _ = sender.send(path);
                    }
                    ctx.request_repaint();
                }
            })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Watch {
            _watcher: watcher,
            changes,
            pending: BTreeSet::new(),
            last: None,
            own: Vec::new(),
        })
    }

    /// Takes the changes of the file at `path`, which the editor wrote itself, such as a data
    /// file that Add file copied, as no changes for a moment, since the editor loads what reads
    /// it.
    pub fn own(&mut self, path: PathBuf) {
        self.own.push((path, Instant::now()));
    }

    /// The paths that changed in a burst that ended, once no change came for [`QUIET`], each once.
    /// While a burst has not ended, it asks `ctx` for a repaint when it would end.
    pub fn changed(&mut self, ctx: &egui::Context) -> Vec<PathBuf> {
        self.own.retain(|(_, at)| at.elapsed() < OWN);
        while let Ok(path) = self.changes.try_recv() {
            if !self.own.iter().any(|(own, _)| *own == path) {
                self.pending.insert(path);
                self.last = Some(Instant::now());
            }
        }
        let Some(last) = self.last else {
            return Vec::new();
        };
        let waited = last.elapsed();
        if waited < QUIET {
            ctx.request_repaint_after(QUIET - waited);
            return Vec::new();
        }
        self.last = None;
        std::mem::take(&mut self.pending).into_iter().collect()
    }
}
