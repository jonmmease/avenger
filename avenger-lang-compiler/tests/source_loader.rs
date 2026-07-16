use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use avenger_lang_compiler::{Compiler, DefaultSourceLoader};

fn fixture_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "avenger-lang-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

#[tokio::test]
async fn source_loader_discovers_project_and_relative_imports_deterministically() {
    let root = fixture_dir("discovery");
    write(
        root.join("charts/chart.avenger"),
        "avenger 1; import '../marks/badge.mark.avenger'; chart cartesian as chart {}",
    );
    write(
        root.join("marks/badge.mark.avenger"),
        "avenger 1; define mark badge { mark symbol {} }",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; schema tables as samples { table csv as rows { path: 'rows.csv'; } }",
    );
    write(root.join("rows.csv"), "x,y\n1,2\n");

    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let first = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap();
    let second = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap();
    assert_eq!(first.files.len(), 3);
    assert_eq!(first.chart_roots.len(), 1);
    assert_eq!(first.ambient_data.len(), 1);
    assert_eq!(first.fingerprint, second.fingerprint);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_loads_versioned_bundled_std_definition() {
    let root = fixture_dir("stdlib");
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'std:marks/error_bar'; chart cartesian as chart {}",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.load_file_project_attempt("chart.avenger").await;
    let project = attempt.result.unwrap();
    assert!(
        project
            .files
            .keys()
            .any(|id| id.as_str() == "std:marks/error_bar.mark.avenger")
    );
    assert!(attempt.dependencies.iter().any(|dependency| {
        dependency
            .content_version
            .as_deref()
            .is_some_and(|version| version.starts_with("stdlib-1:sha256:"))
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_missing_import_keeps_watch_anchor_and_repairs() {
    let root = fixture_dir("repair");
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'nested/badge.mark.avenger'; chart cartesian as chart {}",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let failed = compiler.load_file_project_attempt("chart.avenger").await;
    assert!(failed.result.is_err());
    let missing = failed
        .dependencies
        .iter()
        .find(|dependency| {
            dependency
                .requested_origin
                .display_name()
                .ends_with("nested/badge.mark.avenger")
        })
        .unwrap();
    assert_eq!(
        missing.nearest_existing_parent.as_deref(),
        Some(root.as_path())
    );
    assert!(missing.content_version.is_none());

    write(
        root.join("nested/badge.mark.avenger"),
        "avenger 1; define mark badge { mark symbol {} }",
    );
    assert!(
        compiler
            .load_file_project_attempt("chart.avenger")
            .await
            .result
            .is_ok()
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_rejects_path_escape_and_root_kind_mismatch() {
    let parent = fixture_dir("escape");
    let root = parent.join("project");
    fs::create_dir_all(&root).unwrap();
    write(
        parent.join("outside.mark.avenger"),
        "avenger 1; define mark outside { mark symbol {} }",
    );
    write(
        root.join("chart.avenger"),
        "avenger 1; import '../outside.mark.avenger'; chart cartesian as chart {}",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let failed = compiler.load_file_project_attempt("chart.avenger").await;
    assert!(
        failed.result.unwrap_err().diagnostics[0]
            .primary
            .message
            .contains("capability denied")
    );
    assert!(failed.dependencies.iter().any(|dependency| {
        dependency
            .requested_origin
            .display_name()
            .ends_with("outside.mark.avenger")
    }));

    write(
        root.join("wrong.mark.avenger"),
        "avenger 1; chart cartesian as wrong {}",
    );
    let failed = compiler
        .load_file_project_attempt("wrong.mark.avenger")
        .await
        .result
        .unwrap_err();
    assert_eq!(failed.diagnostics[0].code.as_str(), "AVENGER-PROJECT-002");
    fs::remove_dir_all(parent).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn source_loader_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let parent = fixture_dir("symlink-escape");
    let root = parent.join("project");
    fs::create_dir_all(&root).unwrap();
    write(
        parent.join("outside.mark.avenger"),
        "avenger 1; define mark outside { mark symbol {} }",
    );
    symlink(
        parent.join("outside.mark.avenger"),
        root.join("linked.mark.avenger"),
    )
    .unwrap();
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'linked.mark.avenger'; chart cartesian as chart {}",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let failed = compiler.load_file_project_attempt("chart.avenger").await;
    assert!(
        failed.result.unwrap_err().diagnostics[0]
            .primary
            .message
            .contains("capability denied")
    );
    fs::remove_dir_all(parent).unwrap();
}

#[tokio::test]
async fn source_loader_rejects_duplicate_ambient_catalog_paths() {
    let root = fixture_dir("duplicate-catalog");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("a.data.avenger"),
        "avenger 1; schema tables as samples { table csv as rows { path: 'a.csv'; } }",
    );
    write(
        root.join("b.data.avenger"),
        "avenger 1; schema tables as samples { table csv as other { path: 'b.csv'; } }",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let failed = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert_eq!(failed.diagnostics[0].code.as_str(), "AVENGER-PROJECT-014");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_reports_malformed_transitive_source_with_prefix() {
    let root = fixture_dir("malformed-transitive");
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'bad.mark.avenger'; chart cartesian as chart {}",
    );
    write(
        root.join("bad.mark.avenger"),
        "avenger 1; define mark bad {",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.load_file_project_attempt("chart.avenger").await;
    let failure = attempt.result.unwrap_err();
    assert!(
        failure.diagnostics[0]
            .code
            .as_str()
            .starts_with("AVENGER-PARSE-")
    );
    assert_eq!(attempt.dependencies.iter().count(), 2);
    assert!(
        attempt
            .dependencies
            .iter()
            .all(|dependency| dependency.content_version.is_some())
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_missing_local_data_resource_keeps_anchor_and_repairs() {
    let root = fixture_dir("missing-data");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; table csv as rows { path: 'nested/rows.csv'; }",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let failed = compiler.load_project_graph_attempt(&root).await;
    assert_eq!(
        failed.result.unwrap_err().diagnostics[0].code.as_str(),
        "AVENGER-PROJECT-019"
    );
    let missing = failed
        .dependencies
        .iter()
        .find(|dependency| {
            dependency
                .requested_origin
                .display_name()
                .ends_with("nested/rows.csv")
        })
        .unwrap();
    assert_eq!(
        missing.nearest_existing_parent.as_deref(),
        Some(root.as_path())
    );
    assert!(missing.content_version.is_none());

    write(root.join("nested/rows.csv"), "x\n1\n");
    let repaired = compiler.load_project_graph_attempt(&root).await;
    assert!(repaired.result.is_ok());
    let resource = repaired
        .dependencies
        .iter()
        .find(|dependency| {
            dependency
                .requested_origin
                .display_name()
                .ends_with("nested/rows.csv")
        })
        .unwrap();
    assert!(
        resource
            .content_version
            .as_deref()
            .unwrap()
            .starts_with("sha256:")
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_failed_root_parse_keeps_root_dependency() {
    let root = fixture_dir("bad-root");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.load_file_project_attempt("chart.avenger").await;
    assert!(attempt.result.is_err());
    let dependencies = attempt.dependencies.iter().collect::<Vec<_>>();
    assert_eq!(dependencies.len(), 1);
    assert!(dependencies[0].content_version.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_loader_limits_are_configurable_without_network_access() {
    let root = fixture_dir("limits");
    let loader = DefaultSourceLoader::with_limits(
        &root,
        avenger_lang_compiler::SourceLoaderLimits {
            max_source_bytes: 1024,
            max_redirects: 2,
        },
    );
    assert!(loader.is_ok());
    fs::remove_dir_all(root).unwrap();
}
