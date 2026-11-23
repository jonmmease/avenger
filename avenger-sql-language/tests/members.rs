//! The members of a project's modules by their qualified names: definition, hover, signature
//! help, completion, references, and rename, in a project of a module and two scripts that import
//! it, one by an alias.

use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::{FileRole, Offset, Project, Severity, TextRange};

const MODULE: &str = "modules/weather.av.sqlm";
const PLAIN: &str = "charts/plain.av.sql";
const ALIASED: &str = "charts/aliased.av.sql";
const TYPING: &str = "charts/typing.av.sql";

const WEATHER: &str = "MODULE weather;

CREATE PRIVATE FUNCTION threshold() AS (10.0);

-- Whether a day is warm.
CREATE PUBLIC FUNCTION warm(t FLOAT64) AS (t >= threshold());

CREATE PUBLIC CONSTANT freezing = 0.0;

CREATE PUBLIC FUNCTION cold(t FLOAT64) AS (NOT warm(t));
";

const PLAIN_TEXT: &str = "IMPORT MODULE weather;
FROM UNNEST([1.0, 20.0]) AS t
|> WHERE weather.warm(t)
|> EXTEND weather.freezing AS f
";

const ALIASED_TEXT: &str = "IMPORT MODULE weather AS w;
FROM UNNEST([5.0]) AS t
|> WHERE w.warm(t)
";

fn project() -> Project {
    let module = FileRole::Module {
        names: vec!["weather".to_string()],
    };
    Project::new(
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_files(vec![
        (MODULE, module, WEATHER),
        (PLAIN, FileRole::Chart, PLAIN_TEXT),
        (ALIASED, FileRole::Chart, ALIASED_TEXT),
    ])
}

/// The offset of the `nth` occurrence of `needle` in the file at `path`, and `delta` bytes on.
fn at(project: &Project, path: &str, needle: &str, nth: usize, delta: usize) -> Offset {
    let text = project.document(path).unwrap().text();
    let start = (text.match_indices(needle).nth(nth))
        .unwrap_or_else(|| panic!("{needle} #{nth} in {path}"))
        .0;
    Offset::from((start + delta) as u32)
}

/// The range of the `nth` occurrence of `needle` in the file at `path`.
fn range(project: &Project, path: &str, needle: &str, nth: usize) -> TextRange {
    let start = at(project, path, needle, nth, 0);
    TextRange::new(start, start + Offset::from(needle.len() as u32))
}

/// The range of the name of the module's member `name`, in its `CREATE` statement.
fn defined(project: &Project, name: &str) -> TextRange {
    let text = project.document(MODULE).unwrap().text();
    let statement = (text.find(&format!("FUNCTION {name}(")))
        .or_else(|| text.find(&format!("CONSTANT {name} ")))
        .unwrap();
    let start = statement + text[statement..].find(' ').unwrap() + 1;
    let start = Offset::from(start as u32);
    TextRange::new(start, start + Offset::from(name.len() as u32))
}

#[test]
fn the_files_have_no_errors() {
    let project = project();
    for path in [MODULE, PLAIN, ALIASED] {
        let errors: Vec<String> = (project.diagnostics(path).into_iter())
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .map(|diagnostic| diagnostic.message)
            .collect();
        assert!(errors.is_empty(), "{path}: {errors:?}");
    }
}

#[test]
fn a_qualified_name_goes_to_its_member_and_its_qualifier_to_its_import() {
    let project = project();
    let link = project
        .definition(PLAIN, at(&project, PLAIN, "weather.warm", 0, 9))
        .unwrap();
    assert_eq!(link.origin, range(&project, PLAIN, "warm", 0));
    assert_eq!(link.path, MODULE);
    assert_eq!(link.target, defined(&project, "warm"));
    let constant = project
        .definition(PLAIN, at(&project, PLAIN, "weather.freezing", 0, 9))
        .unwrap();
    assert_eq!(
        (constant.path.as_str(), constant.target),
        (MODULE, defined(&project, "freezing"))
    );
    // A qualifier goes to its import: to the module's path, or to its alias.
    let qualifier = project
        .definition(PLAIN, at(&project, PLAIN, "weather.warm", 0, 1))
        .unwrap();
    assert_eq!(
        (qualifier.path.as_str(), qualifier.target),
        (PLAIN, range(&project, PLAIN, "weather", 0))
    );
    let alias = project
        .definition(ALIASED, at(&project, ALIASED, "w.warm", 0, 0))
        .unwrap();
    assert_eq!(alias.target, range(&project, ALIASED, "w", 1));
}

#[test]
fn a_member_s_hover_gives_its_header_its_module_and_its_comment() {
    let project = project();
    let hover = project
        .hover(PLAIN, at(&project, PLAIN, "weather.warm", 0, 9))
        .unwrap();
    assert_eq!(hover.range, range(&project, PLAIN, "warm", 0));
    for part in [
        "warm(t FLOAT64)",
        "module `weather`",
        MODULE,
        "Whether a day is warm.",
    ] {
        assert!(hover.markdown.contains(part), "{part}: {}", hover.markdown);
    }
    let constant = project
        .hover(PLAIN, at(&project, PLAIN, "weather.freezing", 0, 9))
        .unwrap();
    assert!(
        constant.markdown.contains("CONSTANT freezing = 0.0"),
        "{}",
        constant.markdown
    );
}

#[test]
fn a_call_of_a_member_takes_its_signature() {
    let project = project();
    let help = project
        .signature_help(ALIASED, at(&project, ALIASED, "w.warm(t", 0, 7))
        .unwrap();
    let signature = &help.signatures[0];
    assert_eq!(signature.label, "w.warm(t FLOAT64)");
    let parameter = &signature.parameters[signature.active_parameter.unwrap()];
    assert_eq!(&signature.label[parameter.range.clone()], "t FLOAT64");
}

#[test]
fn a_qualifier_completes_the_public_members_of_its_module() {
    let mut project = project();
    let typing = "IMPORT MODULE weather;\nIMPORT MODULE weather AS w;\nFROM UNNEST([1.0]) AS t\n|> WHERE weather.\n|> WHERE w.\n|> WHERE t.\n";
    project.add(TYPING, FileRole::Chart, typing);
    let labels = |needle: &str| -> Vec<String> {
        let offset = at(&project, TYPING, needle, 0, needle.len());
        (project.completions(TYPING, offset).items.into_iter())
            .map(|item| item.label)
            .collect()
    };
    let members = ["cold", "freezing", "warm"].map(String::from).to_vec();
    assert_eq!(labels("weather."), members);
    assert_eq!(labels("w."), members);
    // A range variable's columns, and no members.
    assert!(!labels("t.").contains(&"warm".to_string()));
}

#[test]
fn a_member_s_references_are_in_its_module_and_every_importer() {
    let project = project();
    // By files in the order of their paths: the module's name of it and its use in `cold`.
    let expected = vec![
        (ALIASED.to_string(), range(&project, ALIASED, "warm", 0)),
        (PLAIN.to_string(), range(&project, PLAIN, "warm", 0)),
        (MODULE.to_string(), defined(&project, "warm")),
        (MODULE.to_string(), {
            let start = at(&project, MODULE, "warm(t)", 0, 0);
            TextRange::new(start, start + Offset::from(4))
        }),
    ];
    let from_call = project.references(PLAIN, at(&project, PLAIN, "weather.warm", 0, 9));
    let found: Vec<(String, TextRange)> = (from_call.into_iter())
        .map(|reference| (reference.path, reference.range))
        .collect();
    assert_eq!(found, expected);
    let name = defined(&project, "warm").start() + Offset::from(1);
    let from_module = project.references(MODULE, name);
    assert_eq!(from_module.len(), expected.len());
}

#[test]
fn a_rename_of_a_member_edits_every_file_and_refuses_a_collision() {
    let project = project();
    let edits = project
        .rename(PLAIN, at(&project, PLAIN, "weather.warm", 0, 9), "hot")
        .unwrap();
    let paths: Vec<&str> = edits.keys().map(String::as_str).collect();
    assert_eq!(paths, [ALIASED, PLAIN, MODULE]);
    assert_eq!(edits[MODULE].len(), 2);
    assert!((edits.values().flatten()).all(|edit| edit.replacement == "hot"));
    let collision = project.rename(PLAIN, at(&project, PLAIN, "weather.warm", 0, 9), "cold");
    assert_eq!(
        collision.unwrap_err(),
        "The module `weather` has a function `cold` already"
    );
}

#[test]
fn a_rename_of_a_qualifier_renames_its_alias_or_adds_one() {
    let project = project();
    let alias = project
        .rename(ALIASED, at(&project, ALIASED, "w.warm", 0, 0), "wx")
        .unwrap();
    let ranges: Vec<TextRange> = alias[ALIASED].iter().map(|edit| edit.range).collect();
    assert_eq!(
        ranges,
        [
            range(&project, ALIASED, "w", 1),
            range(&project, ALIASED, "w", 2)
        ]
    );
    let plain = project
        .rename(PLAIN, at(&project, PLAIN, "weather.warm", 0, 1), "wx")
        .unwrap();
    let edits = &plain[PLAIN];
    assert_eq!(edits[0].replacement, " AS wx");
    assert_eq!(edits.len(), 3);
}

#[test]
fn a_private_member_takes_none_of_them() {
    let mut project = project();
    let private =
        "IMPORT MODULE weather;\nFROM UNNEST([1.0]) AS t\n|> WHERE weather.threshold() > t\n";
    project.add(TYPING, FileRole::Chart, private);
    let offset = at(&project, TYPING, "weather.threshold", 0, 9);
    assert_eq!(project.definition(TYPING, offset), None);
    let hover = project.hover(TYPING, offset);
    assert!(hover.is_none_or(|hover| !hover.markdown.contains(MODULE)));
}
