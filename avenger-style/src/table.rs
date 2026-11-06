//! The style table: the stylesheets that style the charts of an output, beside the charts.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

use crate::{
    source::{Diagnostic, Problem},
    stylesheet::{Import, ParseOptions, Stylesheet},
    SourceId,
};

/// The values of the `env()` names of a stylesheet, as CSS text, which the caller writes from
/// its folded values, such as `teal` or `14px`. A name that the map lacks is undefined, so
/// `env()` gives its fallback.
pub type Env = BTreeMap<String, String>;

/// A parsed stylesheet with the stylesheets that its `@import` rules load, one for each import,
/// in their order. `Sheet::load` builds it with a `Loader`, and a stylesheet without imports
/// converts into one.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    stylesheet: Stylesheet,
    imports: Vec<Sheet>,
}

impl From<Stylesheet> for Sheet {
    /// A sheet of a stylesheet whose imports load nothing, as for a stylesheet without imports.
    fn from(stylesheet: Stylesheet) -> Self {
        let imports = vec![Sheet::default(); stylesheet.imports.len()];
        Sheet {
            stylesheet,
            imports,
        }
    }
}

impl Sheet {
    /// Parses `text`, the text of the source `source` at the URL `url` when it has one, and loads
    /// the stylesheets of its `@import` rules with `loader`, and theirs in turn. The standard
    /// stylesheets `std:base.css`, `std:reset.css`, and `std:vegalite.css` load without the
    /// loader. An import that does not load, of another `std:` URL, or that imports itself through
    /// other stylesheets is an error at its span, and it loads nothing. The diagnostics are those of every stylesheet,
    /// each at the spans of its own source.
    pub fn load(
        text: &str,
        url: Option<&str>,
        source: SourceId,
        options: ParseOptions,
        loader: &mut impl Loader,
    ) -> (Sheet, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let mut chain: Vec<String> = url.map(str::to_string).into_iter().collect();
        let sheet = load_tree(
            text,
            url,
            source,
            options,
            loader,
            &mut chain,
            &mut diagnostics,
        );
        (sheet, diagnostics)
    }

    pub fn stylesheet(&self) -> &Stylesheet {
        &self.stylesheet
    }

    /// The stylesheets of the imports, one for each `@import` rule, in their order.
    pub fn imports(&self) -> &[Sheet] {
        &self.imports
    }

    /// The names that the stylesheet and the stylesheets that it imports read with `env()`,
    /// which are the parameters of its entry of a style table, because an imported stylesheet
    /// reads the `env()` values of the stylesheet that imports it.
    pub fn env_names(&self) -> BTreeSet<&str> {
        let mut names: BTreeSet<&str> = self.stylesheet.env_names().collect();
        for imported in &self.imports {
            names.extend(imported.env_names());
        }
        names
    }
}

/// A stylesheet that a `Loader` gives for an `@import` URL.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The URL that identifies the stylesheet, such as the absolute path of a file. The URLs of
    /// its own imports resolve against it, and an import of a URL of the stylesheets that import
    /// it is a cycle.
    pub url: String,
    /// The source id of the text, which the spans of its diagnostics carry.
    pub source: SourceId,
    pub text: String,
}

/// Loads the stylesheets that `@import` rules name, such as files or resources of a host.
pub trait Loader {
    /// The stylesheet of `url`, as an `@import` rule writes it, resolved against `base`, the URL
    /// of the importing stylesheet when it has one, or why it does not load.
    fn load(&mut self, url: &str, base: Option<&str>) -> Result<Loaded, String>;
}

impl<F: FnMut(&str, Option<&str>) -> Result<Loaded, String>> Loader for F {
    fn load(&mut self, url: &str, base: Option<&str>) -> Result<Loaded, String> {
        self(url, base)
    }
}

fn load_tree(
    text: &str,
    url: Option<&str>,
    source: SourceId,
    options: ParseOptions,
    loader: &mut impl Loader,
    chain: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Sheet {
    let (stylesheet, parsed) = Stylesheet::parse(text, source, options);
    diagnostics.extend(parsed);
    let imports = (stylesheet.imports.iter())
        .map(
            |import| match load_import(import, url, loader, chain, diagnostics) {
                Ok(sheet) => sheet,
                Err(problem) => {
                    diagnostics.push(problem.at(import.span));
                    Sheet::default()
                }
            },
        )
        .collect();
    Sheet {
        stylesheet,
        imports,
    }
}

fn load_import(
    import: &Import,
    importer: Option<&str>,
    loader: &mut impl Loader,
    chain: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Sheet, Problem> {
    if let Some(name) = import.url.strip_prefix("std:") {
        return match name {
            "base.css" if import.layer.is_some() => Err(Problem::new(
                "`std:base.css` loads in the user-agent origin, outside every layer, so its \
                 import takes no layer",
            )),
            "base.css" => Ok(base().clone()),
            "reset.css" => Ok(reset().clone()),
            "vegalite.css" => Ok(vegalite().clone()),
            _ => Err(
                Problem::new(format!("`{}` is not a standard stylesheet", import.url)).with_help(
                    "the standard stylesheets are std:base.css, std:reset.css, and \
                         std:vegalite.css",
                ),
            ),
        };
    }
    let loaded = (loader.load(&import.url, importer))
        .map_err(|reason| Problem::new(format!("cannot load `{}`: {reason}", import.url)))?;
    if let Some(start) = chain.iter().position(|url| *url == loaded.url) {
        let mut cycle = chain[start..].to_vec();
        cycle.push(loaded.url);
        return Err(Problem::new(format!(
            "`{}` imports itself: {}",
            cycle[0],
            cycle.join(" → ")
        )));
    }
    chain.push(loaded.url.clone());
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let sheet = load_tree(
        &loaded.text,
        Some(&loaded.url),
        loaded.source,
        options,
        loader,
        chain,
        diagnostics,
    );
    chain.pop();
    Ok(sheet)
}

/// The origin of a stylesheet in the cascade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// The base stylesheet, which a stylesheet of any origin imports as `std:base.css`.
    UserAgent,
    /// The host theme.
    User,
    /// The stylesheets of a document.
    Author,
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub origin: Origin,
    pub sheet: Sheet,
    pub env: Env,
}

/// The stylesheets of an output in cascade order: the author stylesheets in statement order,
/// each with the values of its `env()` names. A table without stylesheets gives every node its
/// initial values. A stylesheet that imports `std:base.css` gives the base stylesheet, in the
/// user-agent origin, with the values of its own `env()` names, such as `background` or
/// `accent`.
#[derive(Clone, Debug, Default)]
pub struct StyleTable {
    pub(crate) entries: Vec<Entry>,
}

impl StyleTable {
    /// A table without stylesheets.
    pub fn new() -> Self {
        StyleTable::default()
    }

    /// Adds an author stylesheet after the others.
    pub fn with_author(mut self, sheet: impl Into<Sheet>, env: Env) -> Self {
        self.entries.push(Entry {
            origin: Origin::Author,
            sheet: sheet.into(),
            env,
        });
        self
    }
}

fn std_sheet(text: &str, source: u32, options: ParseOptions) -> Sheet {
    let (stylesheet, diagnostics) = Stylesheet::parse(text, SourceId(source), options);
    debug_assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    stylesheet.into()
}

/// The source ids of the standard stylesheets, which the caller's ids should not reuse.
pub const STD_BASE: SourceId = SourceId(u32::MAX);
pub const STD_RESET: SourceId = SourceId(u32::MAX - 1);
pub const STD_VEGALITE: SourceId = SourceId(u32::MAX - 2);

/// The text of `std:base.css`.
pub const BASE_CSS: &str = include_str!("std/base.css");
/// The text of `std:reset.css`.
pub const RESET_CSS: &str = include_str!("std/reset.css");
/// The text of `std:vegalite.css`.
pub const VEGALITE_CSS: &str = include_str!("std/vegalite.css");

/// `std:base.css`, the base stylesheet, which gives the look of Vega-Lite. A stylesheet imports it
/// into the user-agent origin.
pub fn base() -> &'static Sheet {
    static SHEET: OnceLock<Sheet> = OnceLock::new();
    SHEET.get_or_init(|| std_sheet(BASE_CSS, STD_BASE.0, ParseOptions::default()))
}

/// `std:reset.css`, which removes the normal declarations of the base stylesheet and the host
/// theme when a document imports it.
pub fn reset() -> &'static Sheet {
    static SHEET: OnceLock<Sheet> = OnceLock::new();
    SHEET.get_or_init(|| {
        std_sheet(
            RESET_CSS,
            STD_RESET.0,
            ParseOptions {
                allow_reserved_layers: true,
                ..Default::default()
            },
        )
    })
}

/// `std:vegalite.css`, the looks of Vega-Lite that the base stylesheet gives otherwise.
pub fn vegalite() -> &'static Sheet {
    static SHEET: OnceLock<Sheet> = OnceLock::new();
    SHEET.get_or_init(|| std_sheet(VEGALITE_CSS, STD_VEGALITE.0, ParseOptions::default()))
}
