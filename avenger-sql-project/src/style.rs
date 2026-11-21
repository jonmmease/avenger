//! The entry stylesheet of a project and the stylesheets that it imports.

use std::collections::{BTreeMap, BTreeSet};

use avenger_style::{Diagnostic, Loaded, ParseOptions, Sheet, SourceId};

use crate::{project::ProjectFiles, source::SourceError};

/// The entry stylesheet of a project, loaded with its imports.
#[derive(Clone, Debug)]
pub struct ProjectStylesheet {
    /// The entry stylesheet and the stylesheets that it imports, or `None` when the project has
    /// no file at the manifest's path of the entry stylesheet, and so no stylesheet of its own.
    pub sheet: Option<Sheet>,
    /// The diagnostics of the entry stylesheet and of the stylesheets that it imports, each at
    /// the spans of its own source, whose id is the position of its file in `sources`.
    pub diagnostics: Vec<Diagnostic>,
    /// The paths of the sources: the project's stylesheets, in the order of
    /// [`ProjectFiles::stylesheets`].
    pub sources: Vec<String>,
    /// The `.css` files of the project that the entry stylesheet does not reach through its
    /// imports, which apply to no chart, in the order of their paths. Without an entry
    /// stylesheet, this is every `.css` file.
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
    /// at the span of its URL. A failure to read the entry stylesheet is the error.
    pub async fn load_stylesheet(&self) -> Result<ProjectStylesheet, SourceError> {
        let sources = self.stylesheets().to_vec();
        let entry = &self.manifest().style.stylesheet;
        let Some(entry_index) = sources.iter().position(|path| path == entry) else {
            return Ok(ProjectStylesheet {
                sheet: None,
                diagnostics: Vec::new(),
                unreached: sources.clone(),
                sources,
            });
        };
        let entry_text = self.source().read_text(entry).await?;
        let mut texts = BTreeMap::new();
        for (index, path) in sources.iter().enumerate() {
            let text = match index == entry_index {
                true => Ok(entry_text.clone()),
                false => (self.source().read_text(path).await).map_err(|error| error.to_string()),
            };
            texts.insert(path.as_str(), (index, text));
        }
        let mut reached = BTreeSet::new();
        let mut loader = |url: &str, base: Option<&str>| -> Result<Loaded, String> {
            let path = resolve(url, base.unwrap_or(entry))?;
            let (index, text) = (texts.get(path.as_str()))
                .ok_or_else(|| format!("the project has no stylesheet '{path}'"))?;
            let text = text.clone()?;
            reached.insert(path.clone());
            Ok(Loaded {
                url: path,
                source: SourceId(*index as u32),
                text,
            })
        };
        let options = ParseOptions {
            allow_import: true,
            ..Default::default()
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

/// The path, relative to the project root, of the import `url` of the stylesheet at `importer`.
fn resolve(url: &str, importer: &str) -> Result<String, String> {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    if url.starts_with('/') || scheme.is_some_and(|scheme| !scheme.contains('/')) {
        return Err("an import is a path relative to the stylesheet that holds it".into());
    }
    let mut names: Vec<&str> = importer.split('/').collect();
    names.pop();
    for name in url.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop().ok_or("the path goes above the project root")?;
            }
            name => names.push(name),
        }
    }
    Ok(names.join("/"))
}
