//! The watch of a project's directory: the files that change on disk, in bursts, and not the
//! files that the editor wrote itself.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use avenger_sql_editor::watch::{Watch, QUIET};
use eframe::egui;

/// The paths of the next burst of `watch` that ends within a few seconds, or none.
fn burst(watch: &mut Watch, ctx: &egui::Context) -> Vec<PathBuf> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let changed = watch.changed(ctx);
        if !changed.is_empty() {
            return changed;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Vec::new()
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

#[test]
fn a_burst_of_changes_ends_once_with_each_path_and_leaves_out_the_editors_own_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical(directory.path());
    let ctx = egui::Context::default();
    let mut watch = Watch::new(&root, ctx.clone()).unwrap();
    // The watch starts after a moment on some platforms.
    std::thread::sleep(Duration::from_millis(300));

    // Two writes of one file and a write of another are one burst, which holds each path once,
    // and which does not end before QUIET passes.
    let (a, b) = (root.join("a.csv"), root.join("b.csv"));
    std::fs::write(&a, "x\n1\n").unwrap();
    std::fs::write(&a, "x\n2\n").unwrap();
    std::fs::write(&b, "x\n3\n").unwrap();
    let start = Instant::now();
    let changed = burst(&mut watch, &ctx);
    assert!(start.elapsed() >= QUIET);
    let paths: Vec<PathBuf> = changed.iter().map(|path| canonical(path)).collect();
    assert!(paths.contains(&a) && paths.contains(&b), "{paths:?}");
    assert_eq!(paths.iter().filter(|path| **path == a).count(), 1);

    // A file that the editor wrote is not a change, and a file that it did not write is.
    let own = root.join("own.csv");
    watch.own(own.clone());
    std::fs::write(&own, "x\n4\n").unwrap();
    let other = root.join("other.csv");
    std::fs::write(&other, "x\n5\n").unwrap();
    let changed = burst(&mut watch, &ctx);
    let paths: Vec<PathBuf> = changed.iter().map(|path| canonical(path)).collect();
    assert!(paths.contains(&other), "{paths:?}");
    assert!(!paths.contains(&own), "{paths:?}");

    // A removed file is a change.
    std::fs::remove_file(&b).unwrap();
    let changed = burst(&mut watch, &ctx);
    assert!(
        changed.iter().any(|path| path.file_name() == b.file_name()),
        "{changed:?}"
    );
}
