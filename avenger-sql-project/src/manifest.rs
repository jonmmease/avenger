//! The manifest `avenger.toml`, which marks a project's root and gives the paths of its roles.

use std::{fmt, ops::Range, sync::Arc};

use serde::Deserialize;
use toml::Spanned;

/// The name of the manifest, at the root of a project.
pub const MANIFEST: &str = "avenger.toml";

/// The suffix of the name of a script, a data script or a chart script, which the manifest's
/// patterns tell apart: `.av.sql`, which keeps `.sql` for tools that read SQL and tells an editor
/// an avenger-sql script from other SQL by its name alone.
pub const SCRIPT_SUFFIX: &str = ".av.sql";

/// The suffix of the name of a module: `.av.sqlm`.
pub const MODULE_SUFFIX: &str = ".av.sqlm";

/// The version of the project format that this crate reads.
pub const FORMAT: i64 = 1;

/// The manifest of a project, `avenger.toml`. Every section but `[project]` and every field but
/// `name` is optional, with the defaults of [`Manifest::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub project: ProjectSection,
    pub data: DataSection,
    pub charts: ChartsSection,
    pub style: StyleSection,
    pub modules: ModulesSection,
}

/// `[project]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSection {
    pub name: String,
    /// The version of the project format, which a later avenger reads to migrate an older
    /// project. This crate reads [`FORMAT`] only.
    pub format: i64,
}

/// `[data]`: the data scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataSection {
    /// The patterns of the data scripts, `data/**/*.av.sql` by default. Their order is not the
    /// order of execution, which comes from the dependencies of the scripts.
    pub scripts: Vec<Pattern>,
    /// The patterns that no data script matches, `data/files/**` by default, the data's own
    /// folder.
    pub exclude: Vec<Pattern>,
}

/// `[charts]`: the chart scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartsSection {
    /// The patterns of the chart scripts, `charts/**/*.av.sql` by default.
    pub scripts: Vec<Pattern>,
}

/// `[style]`: the entry stylesheet.
#[derive(Clone, Debug)]
pub struct StyleSection {
    /// The path of the entry stylesheet, `styles/main.css` by default, whose `@import` rules
    /// load the project's other stylesheets in cascade order.
    pub stylesheet: String,
    /// The range of the stylesheet's string in the manifest, or `None` for the default.
    pub range: Option<Range<usize>>,
    /// The line of the start of `range`, from 1.
    pub line: Option<usize>,
}

impl PartialEq for StyleSection {
    /// Sections are equal when their stylesheets are, wherever the manifest writes them.
    fn eq(&self, other: &StyleSection) -> bool {
        self.stylesheet == other.stylesheet
    }
}

impl Eq for StyleSection {}

/// `[modules]`: the modules that `IMPORT MODULE` reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModulesSection {
    /// The directory where `IMPORT MODULE a.b` finds `a/b.av.sqlm`, `modules` by default, and the
    /// empty path for the root.
    pub root: String,
}

/// A glob pattern of paths relative to the project root, in which `*` and `?` match within a
/// name, and `**` as a whole name matches any number of directories, none included. A pattern
/// matches with case, and `*`, `?`, and `**` match no name that starts with `.`.
#[derive(Clone, Debug)]
pub struct Pattern {
    glob: Arc<glob::Pattern>,
    range: Option<Range<usize>>,
    line: Option<usize>,
}

impl Pattern {
    /// The pattern of `text`, a path relative to the root by the rules of the manifest's paths,
    /// without its `.` names and its empty names, at `range` and `line` of the manifest.
    fn new(
        text: &str,
        range: Option<Range<usize>>,
        line: Option<usize>,
    ) -> Result<Pattern, ManifestError> {
        let error = |message: String| ManifestError {
            message,
            range: range.clone(),
            line: None,
        };
        let path = crate::path::relative(text).ok_or_else(|| {
            error(format!(
                "'{text}' is not a pattern of paths relative to the project root"
            ))
        })?;
        if path.is_empty() {
            return Err(error(format!(
                "'{text}' is not a pattern of paths: it names no file"
            )));
        }
        let glob = glob::Pattern::new(&path).map_err(|invalid| {
            error(format!(
                "'{text}' is not a pattern of paths: {}",
                invalid.msg
            ))
        })?;
        Ok(Pattern {
            glob: Arc::new(glob),
            range,
            line,
        })
    }

    pub fn as_str(&self) -> &str {
        self.glob.as_str()
    }

    /// The range of the pattern's string in the manifest, or `None` for a default.
    pub fn range(&self) -> Option<Range<usize>> {
        self.range.clone()
    }

    /// The line of the start of the pattern's range in the manifest, from 1, or `None` for a
    /// default.
    pub fn line(&self) -> Option<usize> {
        self.line
    }

    /// The directory of the pattern: its names before the first that holds a character of a
    /// glob, `*`, `?`, or `[`, or that is the last, as `charts` for `charts/**/*.av.sql`, and the
    /// empty path for a pattern of the root's files, as `*.av.sql`.
    pub fn directory(&self) -> &str {
        let pattern = self.as_str();
        let (mut start, mut end) = (0, 0);
        for name in pattern.split('/') {
            if name.contains(['*', '?', '[']) || start + name.len() == pattern.len() {
                break;
            }
            end = start + name.len();
            start = end + 1;
        }
        &pattern[..end]
    }

    /// Whether the pattern matches the path `path`, relative to the project root.
    pub fn matches(&self, path: &str) -> bool {
        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        self.glob.matches_with(path, options)
    }
}

impl PartialEq for Pattern {
    /// Patterns are equal when their texts are, wherever the manifest writes them.
    fn eq(&self, other: &Pattern) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Pattern {}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error of a manifest, at a range of its text when it has one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ManifestError {
    pub message: String,
    /// The range of the error's key or value in the manifest's text.
    pub range: Option<Range<usize>>,
    /// The line of the start of `range`, from 1.
    pub line: Option<usize>,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{MANIFEST}:{line}: {}", self.message),
            None => write!(f, "{MANIFEST}: {}", self.message),
        }
    }
}

/// The role of a file of a project. A hidden file, one with a name on its path that starts
/// with `.`, has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// A `.av.sql` file that a pattern of `[data]` matches, and no pattern of its `exclude`.
    DataScript,
    /// A `.av.sql` file that a pattern of `[charts]` matches.
    ChartScript,
    /// A `.av.sqlm` file under the modules root, with its module path: `a/b.av.sqlm` is `a.b`.
    Module(Vec<String>),
    /// A `.css` file that no pattern of `[data]`'s `exclude` matches, or the entry stylesheet.
    Stylesheet,
}

/// A file that the patterns of two roles match.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "'{path}' is a data script by {} and a chart script by {}",
    placed(.data),
    placed(.chart)
)]
pub struct RoleConflict {
    pub path: String,
    /// The first pattern of `[data]` that matches the file.
    pub data: Pattern,
    /// The first pattern of `[charts]` that matches the file.
    pub chart: Pattern,
}

/// The pattern `pattern` and its place: its line in the manifest, or that it is a default.
fn placed(pattern: &Pattern) -> String {
    match pattern.line {
        Some(line) => format!("the pattern '{pattern}' at {MANIFEST}:{line}"),
        None => format!("the default pattern '{pattern}'"),
    }
}

impl Manifest {
    /// The manifest of a project named `name` with the default of every field.
    pub fn new(name: impl Into<String>) -> Manifest {
        let pattern = |text: &str| Pattern::new(text, None, None).expect("a valid default pattern");
        Manifest {
            project: ProjectSection {
                name: name.into(),
                format: FORMAT,
            },
            data: DataSection {
                scripts: vec![pattern("data/**/*.av.sql")],
                exclude: vec![pattern("data/files/**")],
            },
            charts: ChartsSection {
                scripts: vec![pattern("charts/**/*.av.sql")],
            },
            style: StyleSection {
                stylesheet: "styles/main.css".into(),
                range: None,
                line: None,
            },
            modules: ModulesSection {
                root: "modules".into(),
            },
        }
    }

    /// The text of the smallest manifest of a project named `name`: the `[project]` section with
    /// the name alone, which [`Manifest::parse`] reads as [`Manifest::new`] of the name. An
    /// editor writes it for a new project.
    pub fn smallest(name: &str) -> String {
        let mut quoted = String::from('"');
        for c in name.chars() {
            match c {
                '"' => quoted.push_str("\\\""),
                '\\' => quoted.push_str("\\\\"),
                c if c.is_control() => quoted.push_str(&format!("\\u{:04X}", c as u32)),
                c => quoted.push(c),
            }
        }
        quoted.push('"');
        format!("[project]\nname = {quoted}\n")
    }

    /// Parses the text of a manifest. An unknown section or key, a value of the wrong type, an
    /// empty `name`, a `format` other than [`FORMAT`], an invalid pattern, and a path or a
    /// pattern that is not relative to the root are errors. A path or a pattern is relative to
    /// the root when it does not start with `/` and has no `..` name and no `\`, and it reads
    /// without its `.` names and its empty names, so `./charts//*.av.sql` is `charts/*.av.sql`.
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let line = |error: ManifestError| ManifestError {
            line: (error.range.as_ref()).map(|range| line_of(text, range.start)),
            ..error
        };
        let raw: RawManifest = toml::from_str(text).map_err(|error| {
            line(ManifestError {
                message: error.message().to_string(),
                range: error.span(),
                line: None,
            })
        })?;
        raw.validate(text).map_err(line)
    }

    /// The role of the file at `path`, relative to the root, or `None` for a file without one,
    /// such as a data file or a hidden file.
    pub fn role(&self, path: &str) -> Result<Option<Role>, RoleConflict> {
        if path.split('/').any(|name| name.starts_with('.')) {
            return Ok(None);
        }
        let excluded = self.data.exclude.iter().any(|p| p.matches(path));
        if path.ends_with(SCRIPT_SUFFIX) {
            let first = |patterns: &[Pattern]| patterns.iter().find(|p| p.matches(path)).cloned();
            let data = first(&self.data.scripts).filter(|_| !excluded);
            let chart = first(&self.charts.scripts);
            return match (data, chart) {
                (Some(data), Some(chart)) => Err(RoleConflict {
                    path: path.into(),
                    data,
                    chart,
                }),
                (Some(_), None) => Ok(Some(Role::DataScript)),
                (None, Some(_)) => Ok(Some(Role::ChartScript)),
                (None, None) => Ok(None),
            };
        }
        if (path.ends_with(".css") && !excluded) || path == self.style.stylesheet {
            return Ok(Some(Role::Stylesheet));
        }
        let module = match self.modules.root.as_str() {
            "" => Some(path),
            root => (path.strip_prefix(root)).and_then(|rest| rest.strip_prefix('/')),
        };
        let names = (module.and_then(|module| module.strip_suffix(MODULE_SUFFIX)))
            .map(|module| module.split('/').map(str::to_string).collect());
        Ok(names.map(Role::Module))
    }
}

/// The line of the byte `offset` of `text`, from 1.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    project: RawProject,
    #[serde(default)]
    data: RawData,
    #[serde(default)]
    charts: RawCharts,
    #[serde(default)]
    style: RawStyle,
    #[serde(default)]
    modules: RawModules,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    name: Spanned<String>,
    format: Option<Spanned<i64>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawData {
    scripts: Option<Vec<Spanned<String>>>,
    exclude: Option<Vec<Spanned<String>>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCharts {
    scripts: Option<Vec<Spanned<String>>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStyle {
    stylesheet: Option<Spanned<String>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModules {
    root: Option<Spanned<String>>,
}

impl RawManifest {
    /// The manifest of the raw manifest, which `Manifest::parse` read from `text`, the text of
    /// the lines of its places.
    fn validate(self, text: &str) -> Result<Manifest, ManifestError> {
        let name = self.project.name;
        if name.get_ref().is_empty() {
            return Err(ManifestError {
                message: "the name of the project is empty".into(),
                range: Some(name.span()),
                line: None,
            });
        }
        let mut manifest = Manifest::new(name.into_inner());
        if let Some(format) = self.project.format {
            if *format.get_ref() != FORMAT {
                return Err(ManifestError {
                    message: format!(
                        "the project format {} is not one that this avenger reads, which is {FORMAT}",
                        format.get_ref()
                    ),
                    range: Some(format.span()),
                    line: None,
                });
            }
        }
        let patterns = |patterns: Option<Vec<Spanned<String>>>, default: Vec<Pattern>| {
            patterns.map_or(Ok(default), |patterns| {
                (patterns.into_iter())
                    .map(|p| {
                        Pattern::new(
                            p.get_ref(),
                            Some(p.span()),
                            Some(line_of(text, p.span().start)),
                        )
                    })
                    .collect()
            })
        };
        manifest.data.scripts = patterns(self.data.scripts, manifest.data.scripts)?;
        manifest.data.exclude = patterns(self.data.exclude, manifest.data.exclude)?;
        manifest.charts.scripts = patterns(self.charts.scripts, manifest.charts.scripts)?;
        if let Some(stylesheet) = self.style.stylesheet {
            let path = relative(&stylesheet)?;
            if path.is_empty() {
                return Err(ManifestError {
                    message: "the entry stylesheet is a file, not the root".into(),
                    range: Some(stylesheet.span()),
                    line: None,
                });
            }
            manifest.style = StyleSection {
                stylesheet: path,
                range: Some(stylesheet.span()),
                line: Some(line_of(text, stylesheet.span().start)),
            };
        }
        if let Some(root) = self.modules.root {
            manifest.modules.root = relative(&root)?;
        }
        Ok(manifest)
    }
}

/// The path `path` relative to the project root, as [`crate::path::relative`] reads it, and
/// the empty path for the root itself. A path that is not relative to the root is an error.
fn relative(path: &Spanned<String>) -> Result<String, ManifestError> {
    let text = path.get_ref();
    crate::path::relative(text).ok_or_else(|| ManifestError {
        message: format!("'{text}' is not a path relative to the project root"),
        range: Some(path.span()),
        line: None,
    })
}
