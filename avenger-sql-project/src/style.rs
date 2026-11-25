//! The entry stylesheet of a project and the stylesheets that it imports.

use std::collections::{BTreeMap, BTreeSet};

use avenger_style::{
    resolve_import, Diagnostic, Loaded, ParseOptions, Sheet, SourceId, Stylesheet,
};

use crate::{
    manifest::ManifestError,
    project::{ProjectError, ProjectFiles},
};

/// The entry stylesheet of a project, loaded with its imports.
#[derive(Clone, Debug)]
pub struct ProjectStylesheet {
    /// The entry stylesheet and the stylesheets that it imports, or `None` when the manifest
    /// names no entry stylesheet and the project has no file at its default path, and so no
    /// stylesheet of its own.
    pub sheet: Option<Sheet>,
    /// The diagnostics of the entry stylesheet and of the stylesheets that it imports, each at
    /// the spans of its own source, whose id is the position of its file in `sources`.
    pub diagnostics: Vec<Diagnostic>,
    /// The paths of the sources: the project's stylesheets, in the order of
    /// [`ProjectFiles::stylesheets`].
    pub sources: Vec<String>,
    /// The stylesheets of the project that the entry stylesheet does not reach through its
    /// imports, which apply to no chart and which the load does not read, in the order of their
    /// paths. Without an entry stylesheet, this is every stylesheet.
    pub unreached: Vec<String>,
}

impl ProjectStylesheet {
    /// The path of the file of the source `source`.
    pub fn path(&self, source: SourceId) -> Option<&str> {
        self.sources.get(source.0 as usize).map(String::as_str)
    }
}

impl ProjectFiles {
    /// Loads the entry stylesheet with `Sheet::load` and its imports through the file source.
    /// An import is a path relative to the stylesheet that holds it, of a `.css` file of the
    /// project, or a standard stylesheet such as `std:base.css`. An import that does not load,
    /// that leaves the project, or that imports itself through other stylesheets is an error
    /// at the span of its URL. The load reads the stylesheets that the entry stylesheet reaches
    /// through its imports, and no other. A failure to read the entry stylesheet is the error,
    /// and so is an entry stylesheet that the manifest names and the project lacks, an error of
    /// the manifest at its value.
    pub async fn load_stylesheet(&self) -> Result<ProjectStylesheet, ProjectError> {
        let sources = self.stylesheets().to_vec();
        let style = &self.manifest().style;
        let entry = &style.stylesheet;
        let Some(entry_index) = sources.iter().position(|path| path == entry) else {
            if style.range.is_some() {
                return Err(ProjectError::Manifest(ManifestError {
                    message: format!("the project has no entry stylesheet '{entry}'"),
                    range: style.range.clone(),
                    line: style.line,
                }));
            }
            return Ok(ProjectStylesheet {
                sheet: None,
                diagnostics: Vec::new(),
                unreached: sources.clone(),
                sources,
            });
        };
        let options = ParseOptions {
            allow_import: true,
            ..Default::default()
        };
        let indices: BTreeMap<&str, usize> = (sources.iter().enumerate())
            .map(|(index, path)| (path.as_str(), index))
            .collect();
        // The text of each stylesheet that the entry stylesheet reaches through its imports, or
        // why it does not read, by path, read before the load, whose loader does not await.
        let entry_text = self.source().read_text(entry).await?;
        let mut texts = BTreeMap::from([(entry.clone(), Ok(entry_text.clone()))]);
        let mut pending = vec![(entry.clone(), entry_text.clone())];
        while let Some((path, text)) = pending.pop() {
            let (stylesheet, _) = Stylesheet::parse(&text, SourceId(0), options);
            for import in &stylesheet.imports {
                let Ok(import) = resolve_import(&import.url, &path) else {
                    continue;
                };
                if texts.contains_key(&import) || !indices.contains_key(import.as_str()) {
                    continue;
                }
                let text = self.source().read_text(&import).await;
                let text = text.map_err(|error| error.to_string());
                if let Ok(text) = &text {
                    pending.push((import.clone(), text.clone()));
                }
                texts.insert(import, text);
            }
        }
        let mut reached = BTreeSet::new();
        let mut loader = |url: &str, base: Option<&str>| -> Result<Loaded, String> {
            let path = resolve_import(url, base.unwrap_or(entry))?;
            let index = (indices.get(path.as_str()))
                .ok_or_else(|| format!("the project has no stylesheet '{path}'"))?;
            let text = (texts.get(&path))
                .ok_or_else(|| format!("the stylesheet '{path}' was not read"))?
                .clone()?;
            reached.insert(path.clone());
            Ok(Loaded {
                url: path,
                source: SourceId(*index as u32),
                text,
            })
        };
        let (sheet, diagnostics) = Sheet::load(
            &entry_text,
            Some(entry),
            SourceId(entry_index as u32),
            options,
            &mut loader,
        );
        let unreached = (sources.iter())
            .filter(|path| *path != entry && !reached.contains(*path))
            .cloned()
            .collect();
        Ok(ProjectStylesheet {
            sheet: Some(sheet),
            diagnostics,
            sources,
            unreached,
        })
    }
}
