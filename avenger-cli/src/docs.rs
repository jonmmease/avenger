//! `avenger docs`: the reference of avenger-sql, as Markdown, from the library that hover and
//! completion read, the properties and the elements of the style language, and the sections of
//! `avenger-sql/SPEC.md`. A name prints its page: a function, a constructor under the name that a
//! call writes, a parameter by its dotted path, `style:` a property or an element, or a section
//! of the spec that holds a syntax form. `search` ranks the entries for words, `--at` prints the
//! hover at a position, and `--all` prints the whole reference.

use avenger_sql::library::{
    self,
    search::{Entry, Match},
    Accept, Family, Function, FunctionKind, Library, Param,
};
use avenger_style::{vocab::Element, Property, Shorthand};
use serde_json::json;

use crate::{at::At, report::Output, Status};

/// The options of `docs`.
pub struct Options {
    /// The words of the command: a name, a path, `search` and its words, or a syntax form.
    pub words: Vec<String>,
    /// A position, `PATH:LINE:COLUMN`, whose hover prints.
    pub at: Option<String>,
    pub all: bool,
    /// The forms only.
    pub brief: bool,
    /// The examples only.
    pub examples: bool,
    pub kind: Option<Kind>,
    pub family: Option<FamilyName>,
    pub returns: Option<String>,
    pub takes: Option<String>,
}

/// The kinds of functions that `--kind` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Kind {
    Scalar,
    Aggregate,
    Window,
    Table,
}

impl Kind {
    fn of(self) -> FunctionKind {
        match self {
            Kind::Scalar => FunctionKind::Scalar,
            Kind::Aggregate => FunctionKind::Aggregate,
            Kind::Window => FunctionKind::Window,
            Kind::Table => FunctionKind::Table,
        }
    }
}

/// The families of entries that `--family` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum FamilyName {
    Googlesql,
    Mark,
    Constructor,
    Transform,
    Composition,
}

impl FamilyName {
    fn of(self) -> Family {
        match self {
            FamilyName::Googlesql => Family::GoogleSql,
            FamilyName::Mark => Family::Mark,
            FamilyName::Constructor => Family::Constructor,
            FamilyName::Transform => Family::Transform,
            FamilyName::Composition => Family::Composition,
        }
    }
}

/// A page of the reference.
struct Page {
    /// `function`, `constructor`, `parameter`, `style`, or `syntax`.
    kind: &'static str,
    name: String,
    markdown: String,
}

/// Runs `docs`.
pub async fn run(options: &Options, output: &Output) -> Result<Status, String> {
    let library = Library::shared();
    if let Some(at) = &options.at {
        let at = At::resolve(at).await?;
        let Some(hover) = at.hover() else {
            eprintln!("nothing at the position has a hover");
            return Ok(Status::Failed);
        };
        let page = Page {
            kind: "hover",
            name: String::new(),
            markdown: hover.markdown,
        };
        print(&[page], output);
        return Ok(Status::Ok);
    }
    if options.all {
        let pages = all(library);
        print(&pages, output);
        return Ok(Status::Ok);
    }
    let filtered = options.kind.is_some()
        || options.family.is_some()
        || options.returns.is_some()
        || options.takes.is_some();
    let words = &options.words;
    let (search, query) = match words.split_first() {
        Some((first, rest)) if first == "search" => (true, rest.join(" ")),
        Some(_) => (filtered, words.join(" ")),
        None if filtered => (true, String::new()),
        None => {
            return Err(
                "name an entry, or `search WORDS`, `--at PATH:LINE:COLUMN`, or `--all`".into(),
            )
        }
    };
    if !search {
        // A path that does not walk fails as a name of no entry does.
        let pages = match pages(library, &query, options) {
            Ok(pages) => pages,
            Err(error) => {
                eprintln!("error: {error}");
                return Ok(Status::Failed);
            }
        };
        if !pages.is_empty() {
            print(&pages, output);
            return Ok(Status::Ok);
        }
        // A syntax form, from the sections of the spec.
        if let Some(section) = spec_section(&query) {
            print(&[section], output);
            return Ok(Status::Ok);
        }
    }
    // Filters alone list every entry that they keep.
    let results = match query.trim().is_empty() {
        true => (library.entries())
            .map(|entry| Match {
                entry,
                rank: 0,
                alias: None,
            })
            .collect(),
        false => library.search(&query),
    };
    let results = filter(results, options);
    if !search && !results.is_empty() && !output.json() {
        eprintln!("no entry `{query}`; the entries that match it:");
    }
    print_results(&results, output);
    Ok(match results.is_empty() {
        true => {
            if !output.json() {
                eprintln!("nothing matches `{query}`: try `avenger docs search WORDS`");
            }
            Status::Failed
        }
        false => Status::Ok,
    })
}

/// The pages of the name `name`: `style:` an entry of the style language, a function, a
/// constructor by its unique name or by the name that a call writes, or a dotted path to a
/// parameter. None for a name of no entry.
fn pages(library: &Library, name: &str, options: &Options) -> Result<Vec<Page>, String> {
    if let Some(style) = name.strip_prefix("style:") {
        return Ok(style_page(style).into_iter().collect());
    }
    let mut pages = Vec::new();
    if let Some(function) = library.get(name) {
        pages.push(function_page(function, &function.name, None, options));
    }
    // The constructors that calls write by the name, or the constructor of the unique name.
    let written = library.written_constructors(name);
    if !written.is_empty() {
        for (written, constructor) in written {
            let places = library.constructor_places(written).to_vec();
            pages.push(function_page(constructor, written, Some(&places), options));
        }
    } else if let Some(constructor) = library.constructor(name) {
        let places = unique_places(library, &constructor.name);
        let unique = &constructor.name;
        pages.push(function_page(constructor, unique, Some(&places), options));
    }
    if pages.is_empty() && name.contains('.') && !name.contains(' ') {
        return path_page(library, name, options).map(|page| vec![page]);
    }
    Ok(pages)
}

/// The places of the constructor with the unique name `unique`, under each name that calls write
/// for it.
fn unique_places(library: &Library, unique: &str) -> Vec<String> {
    let mut places = Vec::new();
    for entry in library.entries() {
        if let Entry::Constructor {
            written,
            constructor,
        } = entry
        {
            if constructor.name == unique {
                places.extend(library.constructor_places(written).iter().cloned());
            }
        }
    }
    places.dedup();
    places
}

/// The page of `function` under the name `name`, or of a constructor with its `places`, whole or
/// as `--brief` and `--examples` cut it.
fn function_page(
    function: &Function,
    name: &str,
    places: Option<&[String]>,
    options: &Options,
) -> Page {
    let markdown = match (options.brief, options.examples, places) {
        (true, _, _) => library::forms(function, name),
        (_, true, _) => match function.examples.is_empty() {
            true => format!("`{name}` has no examples.\n"),
            false => library::examples(function),
        },
        (_, _, Some(places)) => library::constructor_entry(function, name, places),
        (_, _, None) => library::entry(function),
    };
    Page {
        kind: match places {
            Some(_) => "constructor",
            None => "function",
        },
        name: name.to_string(),
        markdown,
    }
}

/// The page of the dotted path `path`: a function or a constructor by the name that a call
/// writes, then its parameters, each next name a parameter of the parameter's own constructor or
/// a constructor that the parameter takes.
fn path_page(library: &Library, path: &str, options: &Options) -> Result<Page, String> {
    enum Node<'a> {
        Function(&'a Function, String),
        /// A parameter, with the name that calls write for its function.
        Param(String, &'a Param),
    }
    let names: Vec<&str> = path.split('.').collect();
    let first = names[0];
    let start = (library
        .get(first)
        .map(|function| (function, function.name.clone())))
    .or_else(|| {
        let (name, constructor) = *library.written_constructors(first).first()?;
        Some((constructor, name.to_string()))
    })
    .ok_or_else(|| format!("`{path}`: no function or constructor `{first}`"))?;
    let mut node = Node::Function(start.0, start.1);
    for name in &names[1..] {
        node = match node {
            Node::Function(function, written) => match function.param_named(name) {
                Some(param) => Node::Param(written, param),
                None => {
                    let params: Vec<String> = function
                        .params
                        .iter()
                        .map(|p| format!("`{}`", p.name))
                        .collect();
                    return Err(format!(
                        "`{path}`: `{written}` has no parameter `{name}`; its parameters are {}",
                        params.join(", ")
                    ));
                }
            },
            Node::Param(_, param) => {
                let own = (param.takes.constructors.iter())
                    .find(|builds| builds.part.is_none())
                    .and_then(|builds| Some((builds, library.constructor(&builds.constructor)?)));
                match (
                    own.and_then(|(builds, own)| Some((builds, own.param_named(name)?))),
                    param.takes.find(name),
                ) {
                    (Some((builds, inner)), _) => Node::Param(builds.name.clone(), inner),
                    (None, Some(builds)) => match library.constructor(&builds.constructor) {
                        Some(constructor) => Node::Function(constructor, builds.name.clone()),
                        None => return Err(format!("`{path}`: no constructor `{name}`")),
                    },
                    (None, None) => {
                        return Err(format!(
                            "`{path}`: `{}` takes no `{name}`, as a parameter or a constructor",
                            param.name
                        ))
                    }
                }
            }
        };
    }
    Ok(match node {
        Node::Function(function, written) => {
            let places = library.constructor_places(&written).to_vec();
            let mut page = function_page(function, &written, Some(&places), options);
            page.name = path.to_string();
            page
        }
        Node::Param(owner, param) => {
            let forms = param.forms(None).map(|forms| forms.markdown());
            let markdown = match (options.brief, forms) {
                (true, Some(forms)) => format!("{forms}\n"),
                (true, None) => format!("`{path}`\n"),
                (false, forms) => {
                    let forms = forms
                        .map(|forms| format!("\n{forms}\n"))
                        .unwrap_or_default();
                    format!(
                        "## `{path}`\n\nA parameter of `{owner}(...)`.\n\n{}\n{forms}",
                        param.doc
                    )
                }
            };
            Page {
                kind: "parameter",
                name: path.to_string(),
                markdown,
            }
        }
    })
}

/// The page of `name` of the style language: a property, a shorthand, or an element.
fn style_page(name: &str) -> Option<Page> {
    let markdown = if let Some(property) = Property::from_name(name) {
        property_markdown(property)
    } else if let Some(shorthand) = Shorthand::from_name(name) {
        shorthand_markdown(shorthand)
    } else {
        element_markdown(Element::from_name(name)?)
    };
    Some(Page {
        kind: "style",
        name: format!("style:{name}"),
        markdown,
    })
}

fn property_markdown(property: Property) -> String {
    let elements: Vec<String> = (property.elements().iter())
        .map(|element| format!("`{}`", element.name()))
        .collect();
    format!(
        "## `{name}`\n\n```css\n{name}: {grammar}\n```\n\nInitial: `{initial}`. Inherited: {inherited}.\n\nApplies to {elements}.\n",
        name = property.name(),
        grammar = property.syntax().grammar(),
        initial = property.initial(),
        inherited = match property.is_inherited() {
            true => "yes",
            false => "no",
        },
        elements = elements.join(", "),
    )
}

fn shorthand_markdown(shorthand: Shorthand) -> String {
    let longhands: Vec<String> = (shorthand.longhands().iter())
        .map(|property| format!("`{}`", property.name()))
        .collect();
    format!(
        "## `{}`\n\nA shorthand of {}.\n",
        shorthand.name(),
        longhands.join(", ")
    )
}

fn element_markdown(element: Element) -> String {
    let parts: Vec<String> = (element.parts().iter())
        .map(|part| format!("`{}`", part.name()))
        .collect();
    let properties: Vec<String> = (Property::ALL.iter())
        .filter(|property| property.elements().contains(&element))
        .map(|property| format!("`{}`", property.name()))
        .collect();
    let mut markdown = format!("## `{}`\n\nAn element of the style tree.\n", element.name());
    if !parts.is_empty() {
        markdown.push_str(&format!("\nParts: {}.\n", parts.join(", ")));
    }
    if !properties.is_empty() {
        markdown.push_str(&format!("\nProperties: {}.\n", properties.join(", ")));
    }
    markdown
}

/// The sections of the spec, each its title and its text with its heading.
fn spec_sections() -> Vec<(&'static str, &'static str)> {
    let spec = avenger_sql::SPEC;
    let mut starts: Vec<usize> = spec.match_indices("\n## ").map(|(at, _)| at + 1).collect();
    starts.push(spec.len());
    (starts.windows(2))
        .map(|range| {
            let text = &spec[range[0]..range[1]];
            let title = text[3..].lines().next().unwrap_or_default();
            (title, text.trim_end())
        })
        .collect()
}

/// The section of the spec whose text holds `phrase` most often, without case.
fn spec_section(phrase: &str) -> Option<Page> {
    let phrase = phrase.to_ascii_lowercase();
    if phrase.trim().is_empty() {
        return None;
    }
    let (title, text) = (spec_sections().into_iter())
        .map(|(title, text)| {
            (
                title,
                text,
                text.to_ascii_lowercase().matches(&phrase).count(),
            )
        })
        .filter(|(_, _, count)| *count > 0)
        .max_by_key(|(_, _, count)| *count)
        .map(|(title, text, _)| (title, text))?;
    Some(Page {
        kind: "syntax",
        name: title.to_string(),
        markdown: format!("{text}\n"),
    })
}

/// The results of a search that the filters of `options` keep.
fn filter<'a>(results: Vec<Match<'a>>, options: &Options) -> Vec<Match<'a>> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    (results.into_iter())
        .filter(|m| {
            let function = m.entry.function();
            let constructor = matches!(m.entry, Entry::Constructor { .. });
            let family = match constructor {
                true => Family::Constructor,
                false => function.family,
            };
            // A constructor is a scalar function underneath, and its kind is its family's.
            (options.kind).is_none_or(|kind| !constructor && function.kind == kind.of())
                && (options.family).is_none_or(|name| family == name.of())
                && (options.returns.as_deref()).is_none_or(|ty| {
                    (function.forms.iter()).any(|form| same(&form.signature.result.to_string(), ty))
                })
                && (options.takes.as_deref()).is_none_or(|ty| {
                    (function.forms.iter()).any(|form| {
                        (form.signature.args.iter()).any(|arg| match &arg.accept {
                            Accept::Value(t) | Accept::Const(t) => same(&t.to_string(), ty),
                            Accept::OneOf(_) => same("STRING", ty),
                            Accept::Part(_) => false,
                        })
                    })
                })
        })
        .collect()
}

/// The name of an entry, as a search lists it: in lower case, as the style of avenger-sql writes
/// function names.
fn entry_name(entry: &Entry<'_>) -> String {
    entry.name().to_ascii_lowercase()
}

/// The kind of an entry, as a search lists it.
fn entry_kind(entry: &Entry<'_>) -> &'static str {
    if let Entry::Constructor { .. } = entry {
        return "constructor";
    }
    let function = entry.function();
    match function.family {
        Family::Mark => "mark",
        Family::Composition => "composition",
        Family::Transform => "transform",
        Family::Constructor => "constructor",
        Family::GoogleSql => match function.kind {
            FunctionKind::Scalar => "scalar",
            FunctionKind::Aggregate => "aggregate",
            FunctionKind::Window => "window",
            FunctionKind::Table => "table",
        },
    }
}

fn print_results(results: &[Match<'_>], output: &Output) {
    if output.json() {
        let results: Vec<serde_json::Value> = (results.iter())
            .map(|m| {
                json!({
                    "name": entry_name(&m.entry),
                    "kind": entry_kind(&m.entry),
                    "summary": m.entry.function().summary,
                    "alias": m.alias,
                })
            })
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("results".into(), json!(results));
        output.object(fields);
        return;
    }
    let width = (results.iter())
        .map(|m| entry_name(&m.entry).chars().count())
        .max()
        .unwrap_or(0);
    for m in results {
        let name = entry_name(&m.entry);
        let pad = " ".repeat(width - name.chars().count());
        let alias = (m.alias)
            .map(|alias| format!("  (for {alias})"))
            .unwrap_or_default();
        println!(
            "{name}{pad}  {:<11}  {}{alias}",
            entry_kind(&m.entry),
            m.entry.function().summary
        );
    }
}

fn print(pages: &[Page], output: &Output) {
    if output.json() {
        let entries: Vec<serde_json::Value> = (pages.iter())
            .map(|page| json!({"kind": page.kind, "name": page.name, "markdown": page.markdown}))
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("entries".into(), json!(entries));
        output.object(fields);
        return;
    }
    let text: Vec<&str> = pages.iter().map(|page| page.markdown.trim_end()).collect();
    println!("{}", text.join("\n\n"));
}

/// The whole reference: the functions, the constructors under the names that calls write, the
/// properties, the shorthands, and the elements of the style language, and the spec.
fn all(library: &Library) -> Vec<Page> {
    let mut pages = vec![Page {
        kind: "function",
        name: "functions".into(),
        markdown: library::reference(library),
    }];
    let mut constructors = String::from("# Constructors\n");
    for entry in library.entries() {
        let Entry::Constructor {
            written,
            constructor,
        } = entry
        else {
            continue;
        };
        let places = library.constructor_places(written);
        constructors.push('\n');
        constructors.push_str(&library::constructor_entry(constructor, written, places));
    }
    pages.push(Page {
        kind: "constructor",
        name: "constructors".into(),
        markdown: constructors,
    });
    let mut style = String::from("# Style\n");
    for property in Property::ALL {
        style.push('\n');
        style.push_str(&property_markdown(*property));
    }
    for shorthand in Shorthand::ALL {
        style.push('\n');
        style.push_str(&shorthand_markdown(*shorthand));
    }
    for element in Element::ALL.iter().filter(|element| !element.is_variant()) {
        style.push('\n');
        style.push_str(&element_markdown(*element));
    }
    pages.push(Page {
        kind: "style",
        name: "style".into(),
        markdown: style,
    });
    pages.push(Page {
        kind: "syntax",
        name: "spec".into(),
        markdown: avenger_sql::SPEC.to_string(),
    });
    pages
}
