//! The styles of a view: the base stylesheet, then, for a transparent view, a rule that leaves the
//! chart's background to the page, then the project's stylesheet when the page gives one, each
//! with the color scheme's background as its `env(background)`, as the CLI styles its images.

use std::{collections::BTreeMap, sync::Arc};

use avenger_style::{
    resolve_import, Cascade, ColorScheme, Env, Loaded, ParseOptions, Severity, Sheet, SourceId,
    StyleTable,
};

/// The source of the base stylesheet.
const BASE: SourceId = SourceId(u32::MAX);
/// The source of the rule of a transparent view.
const TRANSPARENT: SourceId = SourceId(u32::MAX - 1);

/// A project's stylesheets, as the page gives them: the path of the entry stylesheet, and the text
/// of each stylesheet by its path relative to the project's root. The entry stylesheet imports the
/// others by paths relative to itself, as a project's stylesheets do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectStyles {
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

/// A diagnostic of a project's stylesheet, at a line and a column of its file, both from 1. The
/// column counts characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageDiagnostic {
    pub severity: &'static str,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// The cascade of the base stylesheet, the rule of a `transparent` view, and the stylesheets of
/// `project`, for a reader who prefers `scheme`, and the diagnostics of `project`. The project's
/// rules win over the base rules. Its rules that parse apply, and its imports read its other files
/// and the `std:` stylesheets, since the library fetches no stylesheets.
pub fn cascade(
    project: Option<&ProjectStyles>,
    scheme: ColorScheme,
    transparent: bool,
) -> (Arc<Cascade>, Vec<PageDiagnostic>) {
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let env = Env::from([("background".to_string(), scheme.background().to_string())]);
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let (base, _) = Sheet::load(
        "@import 'std:base.css';",
        None,
        BASE,
        options,
        &mut no_files,
    );
    let mut table = StyleTable::new().with_author(base, env.clone());
    if transparent {
        let (sheet, _) = Sheet::load(
            "chart { background: transparent; }",
            None,
            TRANSPARENT,
            options,
            &mut no_files,
        );
        table = table.with_author(sheet, env.clone());
    }
    let mut diagnostics = Vec::new();
    if let Some(project) = project {
        let paths: Vec<&String> = project.files.keys().collect();
        let source = |path: &str| (paths.iter()).position(|known| known.as_str() == path);
        let mut loader = |url: &str, base: Option<&str>| -> Result<Loaded, String> {
            let path = resolve_import(url, base.unwrap_or(&project.entry))?;
            let (index, text) = (source(&path).zip(project.files.get(&path)))
                .ok_or_else(|| format!("the project has no stylesheet '{path}'"))?;
            Ok(Loaded {
                url: path,
                source: SourceId(index as u32),
                text: text.clone(),
            })
        };
        let entry_text = project
            .files
            .get(&project.entry)
            .cloned()
            .unwrap_or_default();
        let entry_source = SourceId(source(&project.entry).unwrap_or(paths.len()) as u32);
        let (sheet, found) = Sheet::load(
            &entry_text,
            Some(&project.entry),
            entry_source,
            options,
            &mut loader,
        );
        table = table.with_author(sheet, env);
        diagnostics = (found.into_iter())
            .filter_map(|diagnostic| {
                let path = paths.get(diagnostic.span.source.0 as usize)?;
                let (line, column) = line_column(&project.files[*path], diagnostic.span.start);
                Some(PageDiagnostic {
                    severity: match diagnostic.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    },
                    message: diagnostic.message,
                    file: (*path).clone(),
                    line,
                    column,
                })
            })
            .collect();
    }
    (Arc::new(Cascade::new(&table)), diagnostics)
}

/// The line and the column, both from 1, of the byte `offset` of `text`.
fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = before[start..].chars().count() + 1;
    (line as u32, column as u32)
}
