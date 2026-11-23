//! `string-arguments.json`: the arguments of avenger-sql's chart functions and constructors whose
//! strings are in a language of their own, Typst markup or a format pattern, in the terms that a
//! Tree-sitter injection can match, from which `tree-sitter-avenger-sql` generates its injection
//! patterns. The test builds the file from the library, writes it to `tests/output/`, and compares
//! it with the committed file, which a change accepts by copying.
//!
//! An argument is a call's argument by its name, or the first by position, the shorthand of a
//! constructor, with `array` for the strings of an array. A call is a function by its name, or a
//! constructor by each name that a parameter gives it. A name whose constructors disagree on their
//! arguments, such as `title`, has an entry for each meaning, with `within`, the enclosing calls
//! and arguments that take it. `unless` is the argument, `TEXT_SYNTAX`, whose values make the
//! strings of markup plain, every value of its set but `typst`, on the call itself or on its place
//! in an enclosing call. The formats of
//! GoogleSQL's functions stay out: the grammar names them by hand.

use std::collections::BTreeSet;
use std::path::Path;

use avenger_sql::library::{
    search::Entry, Accept, Family, Function, Library, Param, Ty, TEXT_SYNTAX,
};
use serde_json::{json, Value};

const TYPST: &str = "avenger-typst";
const FORMAT: &str = "avenger-format";

/// The language of the strings of `param`, if they have one of their own.
fn language(param: &Param) -> Option<&'static str> {
    match (param.takes.markup, param.takes.format_pattern) {
        (true, _) => Some(TYPST),
        (false, true) => Some(FORMAT),
        (false, false) => None,
    }
}

/// The arguments of `function` in a language of their own: the first by position when it is
/// one, then each by name, in the order of the parameters.
fn arguments(function: &Function) -> Vec<Value> {
    let first = function.positional_param(0);
    let mut arguments = Vec::new();
    for param in &function.params {
        let Some(language) = language(param) else {
            continue;
        };
        if first.as_deref() == Some(param.name.as_str()) {
            arguments.insert(0, json!({"position": 0, "language": language}));
        }
        let array = (function.forms.iter())
            .flat_map(|form| form.signature.args.iter())
            .any(|arg| {
                arg.param == param.name && matches!(arg.accept, Accept::Value(Ty::Array(_)))
            });
        let mut argument = json!({"name": param.name, "language": language});
        if array {
            argument["array"] = json!(true);
        }
        arguments.push(argument);
    }
    arguments
}

/// The values of the `TEXT_SYNTAX` of the constructor `function` that make its markup plain,
/// every value of its set but `typst`, or `None` for a call without one.
fn plain_values(function: &Function) -> Option<Vec<String>> {
    if function.family != Family::Constructor || function.param_named(TEXT_SYNTAX.param).is_none() {
        return None;
    }
    let values = (function.values_of(TEXT_SYNTAX.param).into_iter())
        .filter(|value| !value.eq_ignore_ascii_case(TEXT_SYNTAX.typst))
        .map(str::to_string)
        .collect();
    Some(values)
}

/// The `unless` of the markup of a call whose `TEXT_SYNTAX` makes its markup plain with `values`.
/// The values are listed, since the predicates of Tree-sitter's queries have no lookahead to test
/// for a value that is not `typst`.
fn unless(values: &[String]) -> Value {
    json!({"name": TEXT_SYNTAX.param, "values": values, "language": TYPST})
}

/// The places of the constructor `constructor` under the name `written`: each enclosing call, by
/// its name, and its argument, with the values of the call's `TEXT_SYNTAX` that make its markup
/// plain, if it has one.
fn places(
    library: &Library,
    written: &str,
    constructor: &str,
) -> BTreeSet<(String, String, Option<Vec<String>>)> {
    let mut places = BTreeSet::new();
    for entry in library.entries() {
        let enclosing = entry.function();
        for param in &enclosing.params {
            let takes = (param.takes.constructors.iter()).any(|builds| {
                builds.name.eq_ignore_ascii_case(written) && builds.constructor == constructor
            });
            if takes {
                let name = entry.name().to_ascii_lowercase();
                places.insert((name, param.name.clone(), plain_values(enclosing)));
            }
        }
    }
    places
}

/// The file, from the library.
fn string_arguments(library: &Library) -> Value {
    let mut calls = Vec::new();
    for function in library
        .functions()
        .filter(|f| f.family != Family::GoogleSql)
    {
        let arguments = arguments(function);
        if !arguments.is_empty() {
            calls.push(
                json!({"function": function.name.to_ascii_lowercase(), "arguments": arguments}),
            );
        }
    }
    // The constructors, by the names that calls write.
    let mut written: Vec<(String, Vec<&Function>)> = Vec::new();
    for entry in library.entries() {
        let Entry::Constructor {
            written: name,
            constructor,
        } = entry
        else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        match written.iter_mut().find(|(other, _)| *other == name) {
            Some((_, constructors)) => constructors.push(constructor),
            None => written.push((name, vec![constructor])),
        }
    }
    for (name, constructors) in written {
        let meanings: Vec<(&Function, Vec<Value>)> = (constructors.iter())
            .map(|constructor| (*constructor, arguments(constructor)))
            .collect();
        let disagree = (meanings.iter()).any(|(_, arguments)| *arguments != meanings[0].1);
        for (constructor, arguments) in meanings {
            if arguments.is_empty() {
                continue;
            }
            let markup = arguments
                .iter()
                .any(|argument| argument["language"] == TYPST);
            let places = places(library, &name, &constructor.name);
            let unless_places = markup && places.iter().any(|(_, _, plain)| plain.is_some());
            let mut call = json!({"function": name, "arguments": arguments});
            if let (true, Some(values)) = (markup, plain_values(constructor)) {
                call["unless"] = unless(&values);
            }
            if disagree || unless_places {
                let within: Vec<Value> = (places.iter())
                    .map(|(function, argument, plain)| {
                        let mut place = json!({"function": function, "argument": argument});
                        if let (true, Some(values)) = (markup, plain) {
                            place["unless"] = unless(values);
                        }
                        place
                    })
                    .collect();
                call["within"] = json!(within);
            }
            calls.push(call);
        }
    }
    calls.sort_by_key(|call| {
        (
            call["function"].as_str().unwrap_or_default().to_string(),
            call.to_string(),
        )
    });
    json!({"version": 1, "calls": calls})
}

/// The entries of the file whose call is `function`.
fn calls<'a>(file: &'a Value, function: &str) -> Vec<&'a Value> {
    (file["calls"].as_array().unwrap().iter())
        .filter(|call| call["function"] == function)
        .collect()
}

/// Whether `call` has the argument `argument`, a name or `0`, in `language`.
fn has(call: &Value, argument: &str, language: &str) -> bool {
    (call["arguments"].as_array().unwrap().iter()).any(|candidate| {
        let named = candidate["name"] == argument;
        let positional = argument == "0" && candidate["position"] == 0;
        (named || positional) && candidate["language"] == language
    })
}

/// The one entry of `function` with `argument` in `language`.
fn entry<'a>(file: &'a Value, function: &str, argument: &str, language: &str) -> &'a Value {
    let found: Vec<&Value> = (calls(file, function).into_iter())
        .filter(|call| has(call, argument, language))
        .collect();
    assert_eq!(found.len(), 1, "{function}({argument}) in {found:#?}");
    found[0]
}

#[test]
fn the_file_matches_the_library() {
    let library = Library::new();
    let file = string_arguments(&library);
    let text = format!("{}\n", serde_json::to_string_pretty(&file).unwrap());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = root.join("tests/output/string-arguments.json");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    std::fs::write(&output, &text).unwrap();
    let committed = root.join("string-arguments.json");
    let expected = std::fs::read_to_string(&committed).unwrap_or_default();
    assert!(
        expected == text,
        "the string arguments changed; to accept {}, copy it to {}",
        output.display(),
        committed.display()
    );
}

#[test]
fn the_file_holds_the_arguments_of_markup_and_of_formats() {
    let file = string_arguments(&Library::new());
    // Markup.
    let title = entry(&file, "title", "0", TYPST);
    assert!(has(title, "subtitle", TYPST), "{title:#?}");
    entry(&file, "subtitle", "0", TYPST);
    entry(&file, "mark_bar", "title", TYPST);
    entry(&file, "axis", "title", TYPST);
    entry(&file, "labels", "markup", TYPST);
    let attribution = entry(&file, "attribution", "credits", TYPST);
    let credits = (attribution["arguments"].as_array().unwrap().iter())
        .find(|argument| argument["name"] == "credits")
        .unwrap();
    assert_eq!(credits["array"], true);
    entry(&file, "attribution", "separator", TYPST);
    entry(&file, "field", "markup", TYPST);
    let tooltip_title = entry(&file, "title", "markup", TYPST);
    // Formats.
    entry(&file, "labels", "format", FORMAT);
    entry(&file, "mark_text", "format", FORMAT);
    entry(&file, "field", "format", FORMAT);
    assert!(has(tooltip_title, "format", FORMAT), "{tooltip_title:#?}");
    // The channel functions hold no markup: a channel's title is plain text.
    for channel in ["x", "y", "color"] {
        assert_eq!(calls(&file, channel), Vec::<&Value>::new(), "{channel}");
    }
    // The formats of GoogleSQL's functions are the grammar's own.
    assert_eq!(calls(&file, "format_date"), Vec::<&Value>::new());
}

#[test]
fn a_name_of_constructors_that_disagree_has_an_entry_for_each_meaning() {
    let file = string_arguments(&Library::new());
    let titles = calls(&file, "title");
    assert_eq!(titles.len(), 2, "{titles:#?}");
    let places = |call: &Value| -> Vec<(String, String)> {
        (call["within"].as_array().unwrap().iter())
            .map(|place| {
                let text = |key: &str| place[key].as_str().unwrap().to_string();
                (text("function"), text("argument"))
            })
            .collect()
    };
    let plot = entry(&file, "title", "0", TYPST);
    let tooltip = entry(&file, "title", "markup", TYPST);
    let tooltip_place = ("tooltip".to_string(), "title".to_string());
    assert!(places(tooltip).contains(&tooltip_place), "{tooltip:#?}");
    assert!(!places(plot).contains(&tooltip_place), "{plot:#?}");
    assert!(
        places(plot).contains(&("mark_bar".to_string(), "title".to_string())),
        "{plot:#?}"
    );
    assert!(
        places(plot).contains(&("axis".to_string(), "title".to_string())),
        "{plot:#?}"
    );
}

#[test]
fn a_syntax_other_than_typst_is_the_only_condition() {
    let library = Library::new();
    let file = string_arguments(&library);
    // The rule is the strings of the title's `syntax`.
    let title = library.constructor("title").unwrap();
    // The values are every value of the title's `syntax` but `typst`.
    let syntax = title.values_of(TEXT_SYNTAX.param);
    assert!(syntax.contains(&TEXT_SYNTAX.typst), "{syntax:?}");
    let plain: Vec<&str> = (syntax.iter().copied())
        .filter(|value| *value != TEXT_SYNTAX.typst)
        .collect();
    assert!(
        plain.contains(&"plain") && plain.contains(&"plain_lines"),
        "{plain:?}"
    );
    let expected = json!({"name": "syntax", "values": plain, "language": TYPST});
    assert_eq!(entry(&file, "title", "0", TYPST)["unless"], expected);
    // The subtitle's place in a title.
    let subtitle = entry(&file, "subtitle", "0", TYPST);
    let place = (subtitle["within"].as_array().unwrap().iter())
        .find(|place| place["function"] == "title" && place["argument"] == "subtitle")
        .unwrap();
    assert_eq!(place["unless"], expected);
    // No other entry has one.
    for call in file["calls"].as_array().unwrap() {
        let own = call.get("unless").is_some();
        let within = (call["within"].as_array().into_iter().flatten())
            .any(|place| place.get("unless").is_some());
        let function = call["function"].as_str().unwrap();
        if own || within {
            assert!(function == "title" || function == "subtitle", "{call:#?}");
        }
    }
}
