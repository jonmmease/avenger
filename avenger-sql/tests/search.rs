//! The search of the library: the ranks of a query's words, the aliases of other dialects, and
//! the closest name that the analysis suggests.

use avenger_sql::library::{
    search::{closest_name, Entry, ALIASES},
    Library,
};

/// The names of the first `n` results of `query`.
fn first(query: &str, n: usize) -> Vec<String> {
    let library = Library::new();
    let results = library.search(query);
    results
        .iter()
        .take(n)
        .map(|m| m.entry.name().to_string())
        .collect()
}

#[test]
fn a_search_ranks_names_then_aliases_then_prefixes_then_text() {
    assert_eq!(first("date_trunc", 1), ["DATE_TRUNC"]);
    assert_eq!(first("strftime", 1), ["FORMAT_TIMESTAMP"]);
    assert_eq!(first("date_trunk", 1), ["DATE_TRUNC"]);
    assert_eq!(first("truncate date", 1), ["DATE_TRUNC"]);
    // A prefix of a part of the name.
    assert!(first("trunc", 5).contains(&"TIMESTAMP_TRUNC".to_string()));
    // A word of the summary or the text alone still finds an entry.
    let library = Library::new();
    let found = library.search("regular");
    assert!(
        found.iter().any(|m| m.entry.name() == "REGEXP_CONTAINS"),
        "{found:?}"
    );
    // Every word must match.
    assert!(library.search("date zzzzzz").is_empty());
    // An alias's result names the alias, and another result names none.
    let alias = &library.search("strftime")[0];
    assert_eq!(alias.alias, Some("strftime"));
    assert_eq!(library.search("date_trunc")[0].alias, None);
}

#[test]
fn a_search_finds_constructors_by_the_names_that_calls_write() {
    let library = Library::new();
    let found = library.search("log");
    let constructor = found.iter().find_map(|m| match m.entry {
        Entry::Constructor {
            written,
            constructor,
        } => Some((written, constructor.name.as_str())),
        Entry::Function(_) => None,
    });
    assert_eq!(constructor, Some(("log", "log_scale")));
    // A written name of several constructors finds each.
    let titles: Vec<&str> = (library.search("title").iter())
        .filter_map(|m| match m.entry {
            Entry::Constructor {
                written: "title",
                constructor,
            } => Some(constructor.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    assert!(
        titles.contains(&"title") && titles.contains(&"tooltip_title"),
        "{titles:?}"
    );
}

#[test]
fn every_alias_names_a_function_of_the_library() {
    let library = Library::new();
    for (alias, target) in ALIASES {
        assert!(library.get(target).is_some(), "{alias} -> {target}");
        assert!(
            library.get(alias).is_none(),
            "{alias} is a function of the library"
        );
    }
}

#[test]
fn the_closest_name_is_a_few_edits_away_or_none() {
    let library = Library::new();
    assert_eq!(
        library.closest("date_trunk", &[]).as_deref(),
        Some("date_trunc")
    );
    assert_eq!(
        library.closest("strftime", &[]).as_deref(),
        Some("format_timestamp")
    );
    assert_eq!(
        library.closest("doubel", &["double"]).as_deref(),
        Some("double")
    );
    for name in ["foo", "xyz", "t", "ab", "completely_unknown"] {
        assert_eq!(library.closest(name, &[]), None, "{name}");
    }
    assert_eq!(
        closest_name("specie", ["species", "island"]),
        Some("species")
    );
    assert_eq!(closest_name("y", ["s", "x"]), None);
}
