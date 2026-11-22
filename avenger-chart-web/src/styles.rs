//! The styles of a view: the base stylesheet, then the page's stylesheet when it gives one.

use std::sync::Arc;

use avenger_style::{Cascade, Env, ParseOptions, Severity, Sheet, SourceId, StyleTable};

/// The source of the base stylesheet.
const BASE: SourceId = SourceId(1);
/// The source of the page's stylesheet.
const PAGE: SourceId = SourceId(2);

/// A diagnostic of the page's stylesheet, at a line and a column of its text, both from 1. The
/// column counts characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageDiagnostic {
    pub severity: &'static str,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// The cascade of the base stylesheet and then of `page`, and the diagnostics of `page`. The
/// page's rules win over the base rules. The rules of `page` that parse apply, and only `std:`
/// stylesheets import, because the library fetches no stylesheets.
pub fn cascade(page: Option<&str>) -> (Arc<Cascade>, Vec<PageDiagnostic>) {
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let mut no_fetch =
        |url: &str, _: Option<&str>| Err(format!("only `std:` stylesheets import, not `{url}`"));
    let (base, _) = Sheet::load(
        "@import 'std:base.css';",
        None,
        BASE,
        options,
        &mut no_fetch,
    );
    let mut table = StyleTable::new().with_author(base, Env::new());
    let mut diagnostics = Vec::new();
    if let Some(text) = page {
        let (sheet, found) = Sheet::load(text, None, PAGE, options, &mut no_fetch);
        table = table.with_author(sheet, Env::new());
        diagnostics = (found.into_iter())
            .filter(|diagnostic| diagnostic.span.source == PAGE)
            .map(|diagnostic| {
                let (line, column) = line_column(text, diagnostic.span.start);
                PageDiagnostic {
                    severity: match diagnostic.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    },
                    message: diagnostic.message,
                    line,
                    column,
                }
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
