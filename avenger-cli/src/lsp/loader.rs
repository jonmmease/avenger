//! The loader's thread: a `Loader` for each project, which loads the data scripts in DataFusion.
//! Its futures are not `Send`, so they run here, on a runtime of one thread, apart from the
//! server's main loop, which never waits for a load. Each request names its project by the id
//! that the server gave it when it opened, and comes back as a [`Loaded`] with the state of the
//! load, so the result of a load of a project that has opened again since has the old id.

use std::collections::BTreeMap;
use std::thread::JoinHandle;

use avenger_sql_project::{LoadState, Loader, ProjectFiles};
use crossbeam_channel::{Receiver, Sender};
use datafusion::prelude::SessionContext;

/// A request to the loader's thread.
pub enum Load {
    /// Load the data scripts of the project `files` in a new session.
    Project {
        project: u64,
        files: Box<ProjectFiles>,
    },
    /// Load the data script or the module at `path` again with `text`.
    Script {
        project: u64,
        path: String,
        text: String,
    },
    /// Load the data scripts that read the files or the directories at `paths` again.
    Files { project: u64, paths: Vec<String> },
    /// Drop the project, as when it opens again after its manifest changes.
    Close { project: u64 },
}

/// The result of a load: its project's id, and its state or its error.
pub struct Loaded {
    pub project: u64,
    pub state: Result<LoadState, String>,
}

/// The loader's thread, which ends when its handle drops.
pub struct LoaderThread {
    sender: Option<Sender<Load>>,
    thread: Option<JoinHandle<()>>,
}

impl LoaderThread {
    /// Starts the thread, which sends each result to `results`.
    pub fn start(results: Sender<Loaded>) -> LoaderThread {
        let (sender, requests) = crossbeam_channel::unbounded();
        let thread = std::thread::Builder::new()
            .name("avenger-loader".into())
            .spawn(move || run(requests, results))
            .expect("the loader's thread starts");
        LoaderThread {
            sender: Some(sender),
            thread: Some(thread),
        }
    }

    pub fn send(&self, load: Load) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(load);
        }
    }
}

impl Drop for LoaderThread {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The loop of the thread, until the server drops its sender.
fn run(requests: Receiver<Load>, results: Sender<Loaded>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return;
    };
    let mut loaders: BTreeMap<u64, Loader> = BTreeMap::new();
    for request in requests {
        let (project, state) = match request {
            Load::Project { project, files } => {
                let loaded = runtime.block_on(Loader::load(*files, SessionContext::new()));
                let state = match loaded {
                    Ok((loader, state)) => {
                        loaders.insert(project, loader);
                        Ok(state)
                    }
                    Err(error) => Err(error.to_string()),
                };
                (project, state)
            }
            Load::Script {
                project,
                path,
                text,
            } => {
                let Some(loader) = loaders.get(&project) else {
                    continue;
                };
                let reload = runtime.block_on(loader.reload(&path, text));
                (
                    project,
                    reload.map(|(_, state)| state).map_err(|e| e.to_string()),
                )
            }
            Load::Files { project, paths } => {
                let Some(loader) = loaders.get(&project) else {
                    continue;
                };
                match runtime.block_on(loader.reload_files(&paths)) {
                    Some((_, state)) => (project, Ok(state)),
                    None => continue,
                }
            }
            Load::Close { project } => {
                loaders.remove(&project);
                continue;
            }
        };
        if results.send(Loaded { project, state }).is_err() {
            return;
        }
    }
}
