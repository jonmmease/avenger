//! The watch of a project's directory on disk: the paths of the files that change under it, in
//! bursts that end when no change comes for [`QUIET`], and what each changed path is to the
//! project, by [`watched`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use notify::event::{EventKind, MetadataKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::{manifest::MANIFEST, path::under_root, project::ProjectFiles};

/// How long a burst of changes waits for its next change before it ends.
pub const QUIET: Duration = Duration::from_millis(200);

/// How long the changes of a file that a tool wrote itself are not changes.
const OWN: Duration = Duration::from_secs(2);

/// The watch of a directory and its directories in turn, which ends when it drops.
pub struct Watch {
    _watcher: RecommendedWatcher,
    changes: Receiver<PathBuf>,
    /// The paths of the burst that has not ended.
    pending: BTreeSet<PathBuf>,
    /// When the last change of the burst came.
    last: Option<Instant>,
    /// The files that the tool wrote, and when.
    own: Vec<(PathBuf, Instant)>,
}

/// The state of the burst of changes of a [`Watch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Burst {
    /// No change came since the last burst ended.
    Quiet,
    /// A burst has not ended: it ends after this time, unless another change comes first.
    Waiting(Duration),
    /// A burst ended: the paths that changed in it, each once.
    Ended(Vec<PathBuf>),
}

impl Watch {
    /// Watches the directory `root`, and calls `wake` from the watch's thread when a file under
    /// it is created, changes, or is removed, so that a tool asks for [`Watch::changed`]. A read
    /// of a file, which a load of the files makes, is not a change.
    pub fn new(root: &Path, wake: impl Fn() + Send + 'static) -> notify::Result<Watch> {
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
                    wake();
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

    /// Takes the changes of the file at `path`, which the tool wrote itself, such as a data file
    /// that it copied, as no changes for a moment, since the tool loads what reads it.
    pub fn own(&mut self, path: PathBuf) {
        self.own.push((path, Instant::now()));
    }

    /// The burst of changes so far: the paths of a burst that ended, once no change came for
    /// [`QUIET`], or the time until the burst that has not ended would end, after which a tool
    /// asks again.
    pub fn changed(&mut self) -> Burst {
        self.own.retain(|(_, at)| at.elapsed() < OWN);
        while let Ok(path) = self.changes.try_recv() {
            if !self.own.iter().any(|(own, _)| *own == path) {
                self.pending.insert(path);
                self.last = Some(Instant::now());
            }
        }
        let Some(last) = self.last else {
            return Burst::Quiet;
        };
        let waited = last.elapsed();
        if waited < QUIET {
            return Burst::Waiting(QUIET - waited);
        }
        self.last = None;
        Burst::Ended(std::mem::take(&mut self.pending).into_iter().collect())
    }
}

/// A file under the root of a project that changed on disk, by its path relative to the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Watched {
    /// A file of the project's own, a script, a module, or a stylesheet, which a tool reads
    /// again.
    Project(String),
    /// Another file, such as a data file, which can change a table: a tool loads the data
    /// scripts that read it again, with [`crate::Loader::reload_files`].
    Data(String),
}

/// What the file at `path` on disk, which changed, is to the project `files`, whose root is
/// `root`, a canonical path: a file under the root that is not hidden, nor under a hidden
/// directory, of the project's own or another file. The manifest, a file that two roles match,
/// and a file outside the root are neither. A removed file, which has no canonical path, is
/// found by its directory.
pub fn watched(root: &Path, files: &ProjectFiles, path: &Path) -> Option<Watched> {
    let canonical = || match std::fs::canonicalize(path) {
        Ok(path) => Some(path),
        Err(_) => Some(
            std::fs::canonicalize(path.parent()?)
                .ok()?
                .join(path.file_name()?),
        ),
    };
    let path = match path.strip_prefix(root) {
        Ok(_) => path.to_path_buf(),
        Err(_) => canonical()?,
    };
    let relative = under_root(root, &path)?;
    let hidden = relative.split('/').any(|name| name.starts_with('.'));
    if relative.is_empty() || hidden || relative == MANIFEST {
        return None;
    }
    match files.manifest().role(&relative) {
        Ok(None) => Some(Watched::Data(relative)),
        Ok(Some(_)) => Some(Watched::Project(relative)),
        Err(_) => None,
    }
}
