use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use avenger_lang_compiler::{
    Compiler, CompilerLimits, DefaultSourceLoader, DependencyRole, LocalResourceLimits,
};
use avenger_lang_core::{
    ExpansionLimits, ImportCapabilities, ModuleGraphLoadLimits, SourceLoader, SourceOrigin,
};

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

fn project_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

#[tokio::test]
async fn compiler_resolves_projects_and_preserves_dependency_attempts() {
    let root = fixture_dir("phase-four");
    write(
        root.join("chart.avenger"),
        r#"avenger 1; chart cartesian as chart {
            param float64 as size { value: 4; }
            data: { values: [{ x: 1.0; y: 2.0; }]; }
            mark symbol as points { x: "x"; y: "y"; }
        }"#,
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.resolve_file_project_attempt("chart.avenger").await;
    assert_eq!(attempt.dependencies.iter().count(), 1);
    let resolved = attempt.result.unwrap();
    assert_eq!(resolved.charts.len(), 1);
    assert_eq!(resolved.params.len(), 1);
    assert!(compiler.check_project(&root).await.is_ok());

    let artifact = compiler.compile_file("chart.avenger").await.unwrap();
    assert_eq!(artifact.name.as_deref(), Some("chart"));
    assert!(artifact.interface.params.contains_key("size"));

    write(
        root.join("chart.avenger"),
        r#"avenger 1; chart cartesian as chart {
            resource tiles as missing_url { kind: xyz; }
        }"#,
    );
    let invalid = compiler.resolve_file_project_attempt("chart.avenger").await;
    assert_eq!(invalid.dependencies.iter().count(), 1);
    assert!(
        invalid
            .result
            .unwrap_err()
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "AVENGER-RESOLVE-022" })
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_discovers_project_and_relative_imports_deterministically() {
    let root = project_fixture("relative-import");

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
    assert_eq!(first.source_modules.len(), 3);
    assert_eq!(first.requested_modules.len(), 1);
    assert_eq!(first.ambient_data_modules.len(), 1);
    assert_eq!(first.ambient_catalog.len(), 1);
    assert_eq!(first.fingerprint, second.fingerprint);
}

#[tokio::test]
async fn source_loader_loads_versioned_bundled_std_definition() {
    let root = project_fixture("stdlib");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.load_file_project_attempt("chart.avenger").await;
    let project = attempt.result.unwrap();
    assert!(
        project
            .source_modules
            .keys()
            .any(|id| id.as_str() == "std:marks/error_bar.mark.avenger")
    );
    assert!(attempt.dependencies.iter().any(|dependency| {
        dependency
            .content_version
            .as_deref()
            .is_some_and(|version| version.starts_with("stdlib-1:sha256:"))
    }));
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
    let rendered = failed.result.as_ref().unwrap_err().render();
    assert!(rendered.contains("chart.avenger:"), "{rendered}");
    assert!(
        rendered.contains("import 'nested/badge.mark.avenger'"),
        "{rendered}"
    );
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
    let repaired_fingerprint = repaired.result.as_ref().unwrap().fingerprint.clone();
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
    write(root.join("nested/rows.csv"), "x\n2\n");
    let changed = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap();
    assert_ne!(repaired_fingerprint, changed.fingerprint);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_directory_and_glob_fingerprints_track_content_and_membership() {
    let root = fixture_dir("data-membership");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        r#"avenger 1;
schema tables as local {
  table csv as directory { path: 'parts'; }
  table csv as globbed { path: 'parts/*.csv'; }
}"#,
    );
    write(root.join("parts/one.csv"), "x\n1\n");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let first = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap()
        .fingerprint;
    let stable = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap()
        .fingerprint;
    assert_eq!(first, stable);

    write(root.join("parts/one.csv"), "x\n2\n");
    let content_changed = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap()
        .fingerprint;
    assert_ne!(first, content_changed);

    write(root.join("parts/two.csv"), "x\n3\n");
    let membership_changed = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap()
        .fingerprint;
    assert_ne!(content_changed, membership_changed);
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

#[tokio::test]
async fn source_loader_records_remote_and_glob_table_resources_without_providers() {
    let root = fixture_dir("resource-origins");
    fs::create_dir_all(root.join("data")).unwrap();
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; table parquet as remote { path: 's3://bucket/rows.parquet'; } table csv as local { path: 'data/*.csv'; }",
    );
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.load_project_graph_attempt(&root).await;
    assert!(attempt.result.is_ok());
    assert!(attempt.dependencies.iter().any(|dependency| {
        dependency.role == DependencyRole::RemoteResource
            && dependency.requested_origin.display_name() == "s3://bucket/rows.parquet"
    }));
    assert!(attempt.dependencies.iter().any(|dependency| {
        dependency.role == DependencyRole::LocalResource
            && dependency
                .content_version
                .as_deref()
                .is_some_and(|version| version.starts_with("glob-sha256:"))
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiler_enforces_local_resource_file_size_limit() {
    let root = fixture_dir("resource-file-limit");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; table csv as rows { path: 'rows.csv'; }",
    );
    write(root.join("rows.csv"), "value\n12345\n");
    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits::default(),
            resources: LocalResourceLimits {
                max_file_bytes: 4,
                ..LocalResourceLimits::default()
            },
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-019");
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("per-file limit")
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiler_enforces_aggregate_local_resource_budget() {
    let root = fixture_dir("aggregate-resource-limit");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; table csv as one { path: 'one.csv'; } table csv as two { path: 'two.csv'; }",
    );
    write(root.join("one.csv"), "1234");
    write(root.join("two.csv"), "5678");
    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits::default(),
            resources: LocalResourceLimits {
                max_total_bytes: 7,
                ..LocalResourceLimits::default()
            },
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-019");
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("totals 8 bytes")
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiler_enforces_project_discovery_limits_before_loading() {
    let root = fixture_dir("project-discovery-limits");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("other.avenger"),
        "avenger 1; chart cartesian as other {}",
    );

    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits {
                max_sources: 1,
                ..ModuleGraphLoadLimits::default()
            },
            resources: LocalResourceLimits::default(),
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-017");
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("exceeds 1 Avenger source files")
    );

    fs::create_dir(root.join("nested")).unwrap();
    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits {
                max_sources: 10,
                max_project_directory_depth: 0,
                ..ModuleGraphLoadLimits::default()
            },
            resources: LocalResourceLimits::default(),
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("directory depth exceeds 0")
    );

    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits {
                max_sources: 10,
                max_project_directory_depth: 10,
                max_project_directory_entries: 2,
                ..ModuleGraphLoadLimits::default()
            },
            resources: LocalResourceLimits::default(),
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("exceeds 2 directory entries")
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiler_applies_syntax_and_expansion_limits() {
    let root = fixture_dir("compiler-language-limits");
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'badge.mark.avenger'; chart cartesian as chart { mark badge {} }",
    );
    write(
        root.join("badge.mark.avenger"),
        "avenger 1; define mark badge { mark symbol as glyph {} }",
    );

    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            expansion: ExpansionLimits {
                max_declarations: 0,
                ..ExpansionLimits::default()
            },
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .resolve_file_project_attempt("chart.avenger")
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-EXPAND-005");

    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits {
                syntax: avenger_lang_core::syntax::SyntaxLimits {
                    max_declarations: 0,
                    ..avenger_lang_core::syntax::SyntaxLimits::default()
                },
                ..ModuleGraphLoadLimits::default()
            },
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_file_project_attempt("chart.avenger")
        .await
        .result
        .unwrap_err();
    assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PARSE-024");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn expanded_chart_resolves_external_theme_against_authored_file() {
    let root = fixture_dir("expanded-external-theme");
    write(
        root.join("chart.avenger"),
        "avenger 1; import 'badge.mark.avenger'; chart cartesian as chart { theme css from 'theme.css'; mark badge {} }",
    );
    write(
        root.join("badge.mark.avenger"),
        "avenger 1; define mark badge { mark symbol as glyph {} }",
    );
    write(root.join("theme.css"), "mark { opacity: 0.8; }");

    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let attempt = compiler.compile_file_attempt("chart.avenger").await;
    if let Err(failure) = attempt.result {
        panic!("external theme compilation failed: {failure:?}");
    }
    assert!(attempt.dependencies.iter().any(|dependency| {
        dependency.role == DependencyRole::LocalResource
            && dependency
                .requested_origin
                .display_name()
                .ends_with("theme.css")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compiler_enforces_local_resource_tree_limits() {
    let root = fixture_dir("resource-tree-limits");
    write(
        root.join("chart.avenger"),
        "avenger 1; chart cartesian as chart {}",
    );
    write(
        root.join("catalog.data.avenger"),
        "avenger 1; table parquet as rows { path: 'data'; }",
    );
    write(root.join("data/one.bin"), "1234");
    write(root.join("data/two.bin"), "5678");

    for (limits, expected) in [
        (
            LocalResourceLimits {
                max_files: 1,
                ..LocalResourceLimits::default()
            },
            "exceeds 1 files",
        ),
        (
            LocalResourceLimits {
                max_total_bytes: 7,
                ..LocalResourceLimits::default()
            },
            "totals 8 bytes",
        ),
        (
            LocalResourceLimits {
                max_directory_entries: 1,
                ..LocalResourceLimits::default()
            },
            "exceeds 1 directory entries",
        ),
    ] {
        let compiler = Compiler::builder()
            .project_root(&root)
            .limits(CompilerLimits {
                project: ModuleGraphLoadLimits::default(),
                resources: limits,
                ..CompilerLimits::default()
            })
            .build()
            .unwrap();
        let failure = compiler
            .load_project_graph_attempt(&root)
            .await
            .result
            .unwrap_err();
        assert_eq!(failure.diagnostics[0].code.as_str(), "AVENGER-PROJECT-019");
        assert!(failure.diagnostics[0].primary.message.contains(expected));
    }

    fs::create_dir(root.join("data/nested")).unwrap();
    write(root.join("data/nested/three.bin"), "9");
    let compiler = Compiler::builder()
        .project_root(&root)
        .limits(CompilerLimits {
            project: ModuleGraphLoadLimits::default(),
            resources: LocalResourceLimits {
                max_directory_depth: 0,
                ..LocalResourceLimits::default()
            },
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    let failure = compiler
        .load_project_graph_attempt(&root)
        .await
        .result
        .unwrap_err();
    assert!(
        failure.diagnostics[0]
            .primary
            .message
            .contains("directory depth exceeds 0")
    );
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

#[tokio::test]
async fn source_loader_enforces_http_redirect_limit_without_live_internet() {
    let root = fixture_dir("redirect-limit");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let location = format!("http://{address}/again");
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    let loader = DefaultSourceLoader::with_limits(
        &root,
        avenger_lang_compiler::SourceLoaderLimits {
            max_source_bytes: 1024,
            max_redirects: 2,
        },
    )
    .unwrap();
    let mut capabilities = ImportCapabilities::project(&root);
    capabilities.allow_http = true;
    let result = loader
        .load(
            &SourceOrigin::Http(format!("http://{address}/start")),
            &capabilities,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("redirect"));
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_loader_enforces_http_size_limit_without_live_internet() {
    let root = fixture_dir("http-size-limit");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 2048\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let loader = DefaultSourceLoader::with_limits(
        &root,
        avenger_lang_compiler::SourceLoaderLimits {
            max_source_bytes: 1024,
            max_redirects: 2,
        },
    )
    .unwrap();
    let mut capabilities = ImportCapabilities::project(&root);
    capabilities.allow_http = true;
    let result = loader
        .load(
            &SourceOrigin::Http(format!("http://{address}/large")),
            &capabilities,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("byte limit"));
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
