use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::fs;

use avenger_app::error::AvengerAppError;
use avenger_eventstream::window::{WindowEvent, WindowFileChangedEvent};
use log::error;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use winit::event_loop::EventLoopProxy;

/// FileWatcher manages file system monitoring and sends events to the EventLoop when files change
pub struct FileWatcher {
    /// Files being watched
    watched_files: Vec<PathBuf>,
    /// The file watcher
    watcher: RecommendedWatcher,
    /// Event loop proxy for sending events
    event_proxy: EventLoopProxy<WindowEvent>,
}

impl FileWatcher {
    /// Create a new FileWatcher that sends events to the provided event loop proxy
    pub fn new(event_proxy: EventLoopProxy<WindowEvent>, watched_files: Vec<PathBuf>) -> Result<Self, AvengerAppError> {
        let event_proxy_clone = event_proxy.clone();
        
        // Canonicalize all paths to get absolute, normalized paths without symlinks
        let canonicalized_files: Result<Vec<PathBuf>, _> = watched_files
            .iter()
            .map(|path| fs::canonicalize(path)
                .map_err(|e| AvengerAppError::InternalError(
                    format!("Failed to canonicalize path '{}': {}", path.display(), e)
                ))
            )
            .collect();
        
        let canonicalized_files = canonicalized_files?;
        
        // Create set of watched files for check inside the event handler
        let watched_files_set = canonicalized_files.clone().into_iter().collect::<HashSet<_>>();

        // Create watcher with standard configuration
        let watcher_config = Config::default().with_poll_interval(Duration::from_secs(1));
        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    if matches!(event.kind, EventKind::Modify(_)) {
                        // Process each modified path
                        for path in event.paths {
                            // Canonicalize the event path for comparison
                            if let Ok(canonical_path) = fs::canonicalize(&path) {
                                // Check if this path is being watched
                                let is_watched = watched_files_set.contains(&canonical_path);
                                
                                if is_watched {
                                    // Create file changed event (no content, just the path)
                                    let file_event = WindowFileChangedEvent {
                                        file_path: path,
                                        error: None,
                                    };
                                    
                                    // Send event to the event loop
                                    let _ = event_proxy_clone.send_event(WindowEvent::FileChanged(file_event));
                                }
                            } else {
                                error!("Failed to canonicalize event path: {:?}", path);
                            }
                        }
                    }
                }
            },
            watcher_config,
        ).map_err(|e| AvengerAppError::InternalError(e.to_string()))?;

        // Watch the requested files
        for file in &canonicalized_files {
            watcher.watch(file, RecursiveMode::NonRecursive)
                .map_err(|e| AvengerAppError::InternalError(e.to_string()))?;
        }

        Ok(Self {
            watcher,
            watched_files: canonicalized_files,
            event_proxy,
        })
    }
}