//! The search of the library: its functions and its constructors, ranked for a query, with the
//! names of other dialects as aliases of avenger-sql's, and the closest name to a name that the
//! library lacks, for the suggestions of the analysis.

use std::collections::BTreeMap;

use super::{Function, Library};

/// The names of other dialects, and the functions that avenger-sql writes for them, in lower case
/// as the style of avenger-sql writes function names. They steer search and suggestions only: a page describes avenger-sql alone.
pub const ALIASES: &[(&str, &str)] = &[
    ("strftime", "format_timestamp"),
    ("to_char", "format_timestamp"),
    ("date_format", "format_date"),
    ("strptime", "parse_timestamp"),
    ("to_timestamp", "parse_timestamp"),
    ("to_date", "parse_date"),
    ("now", "current_timestamp"),
    ("getdate", "current_timestamp"),
    ("today", "current_date"),
    ("epoch", "unix_seconds"),
    ("to_unixtime", "unix_seconds"),
    ("from_unixtime", "timestamp_seconds"),
    ("group_concat", "string_agg"),
    ("listagg", "string_agg"),
    ("collect_list", "array_agg"),
    ("len", "length"),
    ("char_length", "length"),
    ("character_length", "length"),
    ("instr", "strpos"),
    ("position", "strpos"),
    ("charindex", "strpos"),
    ("locate", "strpos"),
    ("ucase", "upper"),
    ("lcase", "lower"),
    ("nvl", "ifnull"),
    ("isnull", "ifnull"),
    ("iif", "if"),
    ("cardinality", "array_length"),
    ("array_size", "array_length"),
    ("array_join", "array_to_string"),
    ("string_split", "split"),
    ("split_part", "split"),
    ("regexp_like", "regexp_contains"),
    ("std", "stddev"),
    ("var", "variance"),
    ("bool_and", "logical_and"),
    ("every", "logical_and"),
    ("bool_or", "logical_or"),
    ("arbitrary", "any_value"),
    ("truncate", "trunc"),
    ("point", "mark_symbol"),
    ("circle", "mark_symbol"),
    ("scatter", "mark_symbol"),
    ("line", "mark_line"),
    ("area", "mark_area"),
    ("bar", "mark_bar"),
    ("rect", "mark_rect"),
    ("rule", "mark_rule"),
    ("text", "mark_text"),
    ("geoshape", "mark_geo_shape"),
];

/// How a word of a query matched an entry, best first. An entry ranks by the sum of its words'
/// ranks.
#[derive(Clone, Copy)]
enum Rank {
    /// The name, without case.
    Name,
    /// A name of another dialect for the entry.
    Alias,
    /// The start of the name, or of a part of it between `_` and `.`.
    Prefix,
    /// A name a few edits away.
    Fuzzy,
    /// A part of the name, of four characters or more, that starts the word, as `trunc` starts
    /// `truncate`.
    Stem,
    /// A parameter's name, or the start of a word of the summary.
    Summary,
    /// The start of a word of the description, the notes, or the examples.
    Text,
}

impl Rank {
    fn weight(self) -> usize {
        self as usize + 1
    }
}

/// An entry of the library that a search found.
#[derive(Clone, Copy, Debug)]
pub enum Entry<'a> {
    Function(&'a Function),
    /// A constructor, by the name that a call writes, such as `log`, and the constructor, whose
    /// name is its unique name, such as `log_scale`.
    Constructor {
        written: &'a str,
        constructor: &'a Function,
    },
}

impl<'a> Entry<'a> {
    /// The name that a call writes.
    pub fn name(&self) -> &'a str {
        match self {
            Entry::Function(function) => &function.name,
            Entry::Constructor { written, .. } => written,
        }
    }

    pub fn function(&self) -> &'a Function {
        match self {
            Entry::Function(function) => function,
            Entry::Constructor { constructor, .. } => constructor,
        }
    }
}

/// An entry that a search found, its rank, the sum of its words' ranks, and the name of another
/// dialect that it matched, which tells why a name unlike the query's is a result.
#[derive(Clone, Debug)]
pub struct Match<'a> {
    pub entry: Entry<'a>,
    pub rank: usize,
    pub alias: Option<&'static str>,
}

impl Library {
    /// The constructors by the names that calls write, by those names in lower case: each name
    /// as a call writes it with the unique name of its constructor, in the order of the unique
    /// names. [`Library::new`] finds them once.
    pub(super) fn find_written(&self) -> BTreeMap<String, Vec<(String, String)>> {
        let mut written: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for function in self.functions().chain(self.constructors()) {
            for param in &function.params {
                for builds in &param.takes.constructors {
                    if self.constructor(&builds.constructor).is_none() {
                        continue;
                    }
                    let list = written.entry(builds.name.to_ascii_lowercase()).or_default();
                    if !list.iter().any(|(_, unique)| *unique == builds.constructor) {
                        list.push((builds.name.clone(), builds.constructor.clone()));
                    }
                }
            }
        }
        for list in written.values_mut() {
            list.sort_by(|a, b| a.1.cmp(&b.1));
        }
        written
    }

    /// The constructors that calls write by the name `name`, without case, each with the name as
    /// a call writes it.
    pub fn written_constructors(&self, name: &str) -> Vec<(&str, &Function)> {
        let list = self.written.get(&name.to_ascii_lowercase());
        (list.into_iter().flatten())
            .filter_map(|(written, unique)| Some((written.as_str(), self.constructor(unique)?)))
            .collect()
    }

    /// The entries of the library: its functions, then its constructors by the names that calls
    /// write, in the order of those names.
    pub fn entries(&self) -> impl Iterator<Item = Entry<'_>> {
        let constructors = (self.written.values().flatten()).filter_map(|(written, unique)| {
            Some(Entry::Constructor {
                written,
                constructor: self.constructor(unique)?,
            })
        });
        self.functions().map(Entry::Function).chain(constructors)
    }

    /// The functions and the constructors that match every word of `query`, best first: by the
    /// sum of the ranks of their words, then by name.
    pub fn search(&self, query: &str) -> Vec<Match<'_>> {
        let words: Vec<String> = (query.split_whitespace())
            .map(|word| word.to_ascii_lowercase())
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut matches: Vec<Match<'_>> = (self.entries())
            .filter_map(|entry| {
                let mut rank = 0;
                let mut alias = None;
                for word in &words {
                    let word_rank = rank_word(&entry, word)?;
                    rank += word_rank.weight();
                    if let Rank::Alias = word_rank {
                        alias = (ALIASES.iter())
                            .find(|(name, _)| name == word)
                            .map(|(name, _)| *name);
                    }
                }
                Some(Match { entry, rank, alias })
            })
            .collect();
        matches.sort_by(|a, b| {
            (a.rank, a.entry.name().to_ascii_lowercase())
                .cmp(&(b.rank, b.entry.name().to_ascii_lowercase()))
        });
        matches
    }

    /// The name to suggest for `name`, a function that the library lacks: the function of its
    /// alias, or the closest name of the library, in lower case as the style of avenger-sql writes
    /// function names, or of `others`, such as a script's temporary functions, in the case that
    /// its definition writes it, within a few edits.
    pub fn closest(&self, name: &str, others: &[&str]) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        if let Some((_, target)) = ALIASES.iter().find(|(alias, _)| *alias == lower) {
            return Some(target.to_string());
        }
        let library: Vec<String> = (self.functions())
            .map(|function| function.name.to_ascii_lowercase())
            .collect();
        let names = (library.iter().map(String::as_str)).chain(others.iter().copied());
        closest_name(name, names).map(str::to_string)
    }
}

/// `message` with the suggestion of the name that it meant, when there is one.
pub fn did_you_mean(message: String, suggestion: Option<impl std::fmt::Display>) -> String {
    match suggestion {
        Some(suggestion) => format!("{message}. Did you mean {suggestion}?"),
        None => message,
    }
}

/// The name of `names` closest to `name` without case, of those that are [`close`] to it.
pub fn closest_name<'a>(name: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    (names.into_iter())
        .filter(|candidate| !candidate.eq_ignore_ascii_case(name))
        .filter_map(|candidate| Some((close(&lower, &candidate.to_ascii_lowercase())?, candidate)))
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
        .map(|(_, candidate)| candidate)
}

/// The number of edits from `name` to `candidate`, both in lower case, when it is within a
/// quarter of the length of `name`, at least one edit. A name of one or two characters is close
/// to too many names, and is close to none.
fn close(name: &str, candidate: &str) -> Option<usize> {
    let length = name.chars().count();
    if length < 3 {
        return None;
    }
    let edits = distance(name, candidate);
    (edits <= (length / 4).max(1)).then_some(edits)
}

/// How `word`, in lower case, matches `entry`, or `None`.
fn rank_word(entry: &Entry<'_>, word: &str) -> Option<Rank> {
    let name = entry.name().to_ascii_lowercase();
    if name == word {
        return Some(Rank::Name);
    }
    if (ALIASES.iter()).any(|(alias, target)| *alias == word && target.eq_ignore_ascii_case(&name))
    {
        return Some(Rank::Alias);
    }
    let parts = || name.split(['_', '.']).filter(|part| !part.is_empty());
    if name.starts_with(word) || parts().any(|part| part.starts_with(word)) {
        return Some(Rank::Prefix);
    }
    if close(word, &name).is_some() {
        return Some(Rank::Fuzzy);
    }
    if parts().any(|part| part.len() >= 4 && word.starts_with(part)) {
        return Some(Rank::Stem);
    }
    let function = entry.function();
    let param = (function.params.iter()).any(|param| param.name.eq_ignore_ascii_case(word));
    if param || starts_a_word(&function.summary, word) {
        return Some(Rank::Summary);
    }
    let examples = function
        .examples
        .iter()
        .map(|example| example.query.as_str());
    let text = std::iter::once(function.description.as_str())
        .chain(function.notes.iter().map(String::as_str))
        .chain(examples)
        .chain(function.params.iter().map(|param| param.doc.as_str()));
    for text in text {
        if starts_a_word(text, word) {
            return Some(Rank::Text);
        }
    }
    None
}

/// Whether a word of `text` starts with `word`, in lower case.
fn starts_a_word(text: &str, word: &str) -> bool {
    (text.split(|c: char| !c.is_alphanumeric() && c != '_'))
        .any(|token| token.to_ascii_lowercase().starts_with(word))
}

/// The number of edits between `a` and `b`, by characters, where an edit inserts, removes, or
/// replaces a character, or swaps two neighbors, as a typing slip does.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut rows = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in rows.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[a.len()][b.len()]
}
