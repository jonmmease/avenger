//! The watch of a project's directory: the files that change on disk, in bursts, without the
//! files that the tool wrote itself, and what each changed path is to the project.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use avenger_sql_project::watch::{watched, Burst, Watch, Watched, QUIET};
use avenger_sql_project::ProjectFiles;

/// The paths of the next burst of `watch` that ends within a few seconds, or none, waiting as
/// long as the watch says.
fn burst(watch: &mut Watch) -> Vec<PathBuf> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match watch.changed() {
            Burst::Ended(paths) => return paths,
            Burst::Waiting(wait) => std::thread::sleep(wait),
            Burst::Quiet => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    Vec::new()
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

#[test]
fn a_burst_of_changes_ends_once_with_each_path_and_leaves_out_the_tool_s_own_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical(directory.path());
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let mut watch = Watch::new(&root, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    // The watch starts after a moment on some platforms.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(watch.changed(), Burst::Quiet);

    // Two writes of one file and a write of another are one burst, which holds each path once,
    // and which does not end before QUIET passes.
    let (a, b) = (root.join("a.csv"), root.join("b.csv"));
    std::fs::write(&a, "x\n1\n").unwrap();
    std::fs::write(&a, "x\n2\n").unwrap();
    std::fs::write(&b, "x\n3\n").unwrap();
    let start = Instant::now();
    let changed = burst(&mut watch);
    assert!(start.elapsed() >= QUIET);
    assert!(wakes.load(Ordering::SeqCst) > 0, "a change wakes the tool");
    let paths: Vec<PathBuf> = changed.iter().map(|path| canonical(path)).collect();
    assert!(paths.contains(&a) && paths.contains(&b), "{paths:?}");
    assert_eq!(paths.iter().filter(|path| **path == a).count(), 1);

    // A file that the tool wrote is not a change, and a file that it did not write is.
    let own = root.join("own.csv");
    watch.own(own.clone());
    std::fs::write(&own, "x\n4\n").unwrap();
    let other = root.join("other.csv");
    std::fs::write(&other, "x\n5\n").unwrap();
    let changed = burst(&mut watch);
    let paths: Vec<PathBuf> = changed.iter().map(|path| canonical(path)).collect();
    assert!(paths.contains(&other), "{paths:?}");
    assert!(!paths.contains(&own), "{paths:?}");

    // A removed file is a change.
    std::fs::remove_file(&b).unwrap();
    let changed = burst(&mut watch);
    assert!(
        changed.iter().any(|path| path.file_name() == b.file_name()),
        "{changed:?}"
    );
}

#[tokio::test]
async fn a_changed_path_is_a_file_of_the_project_or_a_data_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/vega");
    let root = canonical(&root);
    let files = ProjectFiles::find(&root).await.unwrap();
    let watched = |path: &Path| watched(&root, &files, path);
    let data = |path: &str| Some(Watched::Data(path.to_string()));
    let project = |path: &str| Some(Watched::Project(path.to_string()));

    assert_eq!(
        watched(&root.join("data/files/regions.csv")),
        data("data/files/regions.csv")
    );
    assert_eq!(
        watched(&root.join("data/regions.av.sql")),
        project("data/regions.av.sql")
    );
    assert_eq!(
        watched(&root.join("styles/main.css")),
        project("styles/main.css")
    );
    // A document, which need not exist yet, is a file of the project.
    assert_eq!(
        watched(&root.join("reports/overview.qmd")),
        project("reports/overview.qmd")
    );
    // A removed file, whose directory is still there.
    assert_eq!(
        watched(&root.join("data/files/gone.csv")),
        data("data/files/gone.csv")
    );
    // The manifest, a hidden file or directory, and a file outside the root are neither.
    let neither = [
        root.join("avenger.toml"),
        root.join(".git/index"),
        root.join("data/files/.regions.csv.swp"),
        std::env::temp_dir().join("regions.csv"),
    ];
    for path in neither {
        assert_eq!(watched(&path), None, "{}", path.display());
    }
}
