use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin,
    sql::{parse_sql_expression, parse_sql_projection, parse_sql_query, tokenize},
    syntax::{
        SqlIslandContext, SqlIslandRoot, SqlIslandSite, TolerantSyntaxNodeKind, parse_file,
        parse_file_tolerant,
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct BoundaryManifest {
    schema_version: u32,
    contexts: Vec<BoundaryContext>,
    sites: Vec<BoundarySite>,
    boundary_cases: Vec<BoundaryCase>,
    structural_cases: Vec<StructuralCase>,
}

#[derive(Debug, Deserialize)]
struct BoundaryContext {
    name: String,
    root: String,
    sites: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BoundarySite {
    name: String,
    context: String,
    outer_delimiters: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BoundaryCase {
    id: String,
    site: String,
    source: String,
    island: String,
    outer: String,
    #[serde(default = "default_true")]
    tolerant_sql_island: bool,
    accepted: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct StructuralCase {
    id: String,
    source: String,
    accepted: bool,
}

#[derive(Debug, Deserialize)]
struct StructuralManifest {
    schema_version: u32,
    authority: StructuralAuthority,
    snapshot_source_count: usize,
    sources: Vec<StructuralSource>,
}

#[derive(Debug, Deserialize)]
struct StructuralAuthority {
    discovery_roots: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StructuralSource {
    path: String,
    sha256: String,
    classification: String,
    root: Option<String>,
}

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tree_sitter")
        .join(relative)
}

fn memory_source(name: &str, text: &str) -> SourceFile {
    SourceFile::new(
        SourceId::new(1),
        SourceOrigin::Memory(name.to_owned()),
        text,
    )
}

fn boundary_manifest() -> BoundaryManifest {
    serde_json::from_str(&fs::read_to_string(fixture("sql_island_boundaries.json")).unwrap())
        .unwrap()
}

fn structural_manifest() -> StructuralManifest {
    serde_json::from_str(&fs::read_to_string(fixture("structural_sources.json")).unwrap()).unwrap()
}

fn collect_avenger_sources(
    directory: &std::path::Path,
    workspace: &std::path::Path,
    paths: &mut BTreeSet<String>,
) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_avenger_sources(&path, workspace, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "avenger")
        {
            paths.insert(
                path.strip_prefix(workspace)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

#[test]
fn sql_island_manifest_covers_every_context_and_call_site() {
    let manifest = boundary_manifest();
    assert_eq!(manifest.schema_version, 2);

    let contexts: BTreeMap<_, _> = manifest
        .contexts
        .iter()
        .map(|context| (context.name.as_str(), context))
        .collect();
    assert_eq!(contexts.len(), SqlIslandContext::ALL.len());
    for context in SqlIslandContext::ALL {
        let entry = contexts
            .get(context.manifest_name())
            .unwrap_or_else(|| panic!("missing SQL-island context {context:?}"));
        let expected_root = match context.root() {
            SqlIslandRoot::Query => "query",
            SqlIslandRoot::Projection => "projection",
            SqlIslandRoot::Expression => "expression",
        };
        assert_eq!(entry.root, expected_root, "root for {context:?}");
    }

    let manifest_sites: BTreeSet<_> = manifest
        .contexts
        .iter()
        .flat_map(|context| {
            context
                .sites
                .iter()
                .map(move |site| (site.as_str(), context.name.as_str()))
        })
        .collect();
    assert_eq!(manifest_sites.len(), SqlIslandSite::ALL.len());
    for site in SqlIslandSite::ALL {
        assert!(
            manifest_sites.contains(&(site.manifest_name(), site.context().manifest_name())),
            "missing or misclassified SQL-island call site {site:?}"
        );
    }

    let sites: BTreeMap<_, _> = manifest
        .sites
        .iter()
        .map(|site| (site.name.as_str(), site))
        .collect();
    assert_eq!(sites.len(), SqlIslandSite::ALL.len());
    for site in SqlIslandSite::ALL {
        let entry = sites
            .get(site.manifest_name())
            .unwrap_or_else(|| panic!("missing SQL-island site {site:?}"));
        assert_eq!(entry.context, site.context().manifest_name());
        assert_eq!(entry.outer_delimiters, site.outer_delimiters());
    }
}

#[test]
fn sql_island_boundaries_leave_outer_tokens_unconsumed() {
    let manifest = boundary_manifest();
    let sites: BTreeMap<_, _> = SqlIslandSite::ALL
        .into_iter()
        .map(|site| (site.manifest_name(), site))
        .collect();

    let covered_sites = manifest
        .boundary_cases
        .iter()
        .map(|case| case.site.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(covered_sites.len(), SqlIslandSite::ALL.len());

    for case in manifest.boundary_cases {
        assert!(
            case.source.starts_with(&case.island),
            "island must be a source prefix for {}",
            case.id
        );
        let site = sites
            .get(case.site.as_str())
            .copied()
            .unwrap_or_else(|| panic!("unknown site for {}", case.id));
        assert!(
            site.outer_delimiters().contains(&case.outer.as_str()),
            "outer token is not legal for {}",
            case.id
        );
        let source = memory_source(&case.id, &case.source);
        let stream = tokenize(&source).unwrap_or_else(|error| {
            panic!("boundary case {} failed tokenization: {error}", case.id)
        });

        let parsed = match site.context().root() {
            SqlIslandRoot::Query => {
                parse_sql_query(&stream, 0).map(|parsed| (parsed.span, parsed.next_token))
            }
            SqlIslandRoot::Projection => {
                parse_sql_projection(&stream, 0).map(|parsed| (parsed.span, parsed.next_token))
            }
            SqlIslandRoot::Expression => {
                parse_sql_expression(&stream, 0).map(|parsed| (parsed.span, parsed.next_token))
            }
        };
        assert_eq!(
            parsed.is_ok(),
            case.accepted,
            "boundary acceptance for {}",
            case.id
        );
        if let Ok((span, next_token)) = parsed {
            assert_eq!(span.range.start, 0, "island start for {}", case.id);
            assert_eq!(
                span.range.end,
                case.island.len(),
                "island end for {}",
                case.id
            );
            let outer = stream
                .token(next_token)
                .unwrap_or_else(|| panic!("missing outer token for {}", case.id));
            assert_eq!(stream.raw(outer), case.outer, "outer token for {}", case.id);
        }
    }
}

#[test]
fn tolerant_and_strict_frontends_agree_on_authored_island_spans() {
    let sites: BTreeMap<_, _> = SqlIslandSite::ALL
        .into_iter()
        .map(|site| (site.manifest_name(), site))
        .collect();

    for case in boundary_manifest().boundary_cases {
        let site = sites[case.site.as_str()];
        let (prefix, suffix) = match site {
            SqlIslandSite::QueryProperty => ("avenger 1; chart cartesian { sql: ", " }"),
            SqlIslandSite::ProjectionProperty => (
                "avenger 1; chart cartesian { transform calculate { expressions: ",
                " } }",
            ),
            SqlIslandSite::ChannelModePayload => (
                "avenger 1; chart cartesian { mark symbol { x: encoded ",
                " } }",
            ),
            SqlIslandSite::PropertyValue => (
                "avenger 1; chart cartesian { transform filter { predicate: ",
                " } }",
            ),
            SqlIslandSite::ArrayElement => (
                "avenger 1; chart cartesian { table inline { values: [",
                "]; } }",
            ),
            SqlIslandSite::ParamInitializer => ("avenger 1; chart cartesian { param ", "; }"),
            SqlIslandSite::OutputSource => ("avenger 1; define transform sample { output ", "; }"),
            SqlIslandSite::CursorActionRhs => (
                "avenger 1; chart cartesian { on click { set cursor to ",
                " } }",
            ),
            SqlIslandSite::StateActionRhs => (
                "avenger 1; chart cartesian { param 1 as width; on click { set width to ",
                " } }",
            ),
        };
        let text = format!("{prefix}{}{suffix}", case.source);
        let source = memory_source(&format!("tolerant-{}", case.id), &text);
        let parsed = parse_file_tolerant(&source);
        let expected = prefix.len()..prefix.len() + case.island.len();
        let matches = parsed
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    TolerantSyntaxNodeKind::SqlIsland { site: actual, .. } if actual == site
                ) && node.span.range.as_range() == expected
            })
            .count();
        assert_eq!(
            matches,
            usize::from(case.tolerant_sql_island),
            "tolerant/strict island span agreement for {}: expected {:?}; found {:?}",
            case.id,
            expected,
            parsed
                .nodes
                .iter()
                .filter_map(|node| match node.kind {
                    TolerantSyntaxNodeKind::SqlIsland { site: actual, .. } if actual == site => {
                        Some(node.span.range)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn sql_island_structural_cases_match_the_strict_parser() {
    for case in boundary_manifest().structural_cases {
        let source = memory_source(&case.id, &case.source);
        assert_eq!(
            parse_file(&source).is_ok(),
            case.accepted,
            "structural boundary case {}",
            case.id
        );
    }
}

#[test]
fn structural_source_manifest_is_complete_hashed_and_parse_checked() {
    let manifest = structural_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.snapshot_source_count, manifest.sources.len());

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut discovered = BTreeSet::new();
    for root in &manifest.authority.discovery_roots {
        collect_avenger_sources(&workspace.join(root), &workspace, &mut discovered);
    }
    let declared: BTreeSet<_> = manifest
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect();
    assert_eq!(
        declared, discovered,
        "structural source manifest must explicitly classify every source"
    );

    for (index, source) in manifest.sources.iter().enumerate() {
        let path = workspace.join(&source.path);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            source.sha256,
            "content hash for {}",
            source.path
        );

        let expects_parse = match source.classification.as_str() {
            "strict_valid" | "canonical_valid" | "stdlib_valid" | "example_valid" => Some(true),
            "strict_invalid" => Some(false),
            "token_fragment" => None,
            classification => panic!(
                "unknown structural source classification `{classification}` for {}",
                source.path
            ),
        };
        let Some(expects_parse) = expects_parse else {
            assert!(
                source.root.is_none(),
                "token fragment root for {}",
                source.path
            );
            continue;
        };
        let text = String::from_utf8(bytes).unwrap();
        let parsed = parse_file(&SourceFile::new(
            SourceId::new(index as u32 + 1),
            SourceOrigin::File(path),
            text,
        ));
        assert_eq!(
            parsed.is_ok(),
            expects_parse,
            "strict parse classification for {}: {parsed:?}",
            source.path
        );
        if let Ok(parsed) = parsed {
            let root = match parsed.ast.items[0].declaration.keyword.as_str() {
                "chart" => "chart",
                "define" => "definition",
                "catalog" | "schema" | "table" => "data",
                keyword => panic!("unexpected module item `{keyword}` in {}", source.path),
            };
            assert_eq!(
                source.root.as_deref(),
                Some(root),
                "root for {}",
                source.path
            );
        } else {
            assert!(
                source.root.is_none(),
                "invalid source root for {}",
                source.path
            );
        }
    }
}
