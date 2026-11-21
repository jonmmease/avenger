//! The gallery of a project: a card for each of its chart scripts, by folder. A chart's path
//! names it, and its text describes it:
//!
//! - The folder of the chart, under the directory of the `[charts]` pattern that matches it, is
//!   its section, as `penguins` for `charts/penguins/donut-of-species.sql`, and a chart in that
//!   directory itself is in the section `Charts`. The sections are in the order of their folders'
//!   paths.
//! - A file stem reads as a title: each `-` becomes a space, and the first letter is a capital,
//!   so that `donut-of-species` is "Donut of species". A leading number and `-`, as in
//!   `01-donut-of-species`, orders the file among the others of its folder and is not part of its
//!   title. A folder's name reads the same way, so that `01-penguins` is the section "Penguins".
//! - The comment lines at the start of the chart's text are its description: each line's text
//!   after `--` and a space, with the lines of a paragraph joined by spaces, and a line `--`
//!   alone between two paragraphs.

use avenger_sql_project::Manifest;

/// A chart script of the gallery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The path of the chart script, relative to the project root.
    pub path: String,
    /// The title of its folder, the heading of its section.
    pub section: String,
    /// The title of its file stem, without a leading number and `-`.
    pub title: String,
    /// The comment lines at the start of its text, with a blank line between paragraphs.
    pub description: String,
    /// The text of the script, which the card runs.
    pub text: String,
}

impl Entry {
    /// The entry of the chart script at `path`, whose text is `text`, of a project of `manifest`.
    pub fn new(manifest: &Manifest, path: &str, text: String) -> Entry {
        let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
        let stem = name.strip_suffix(".sql").unwrap_or(name);
        Entry {
            path: path.to_string(),
            section: section(manifest, folder),
            title: heading(stem),
            description: description(&text),
            text,
        }
    }

    /// Whether the entry holds the lowercase text `search` in its title, its description, its
    /// section, or its text, ignoring case.
    pub fn matches(&self, search: &str) -> bool {
        [&self.title, &self.description, &self.section, &self.text]
            .iter()
            .any(|text| text.to_lowercase().contains(search))
    }
}

/// The entries of the chart scripts `charts`, each a path and its text, of a project of
/// `manifest`, by section in the order of their folders, and in the order of their paths in a
/// section.
pub fn entries(
    manifest: &Manifest,
    charts: impl IntoIterator<Item = (String, String)>,
) -> Vec<Entry> {
    let mut entries: Vec<(String, Entry)> = (charts.into_iter())
        .map(|(path, text)| {
            let folder = path
                .rsplit_once('/')
                .map_or("", |(folder, _)| folder)
                .to_string();
            (folder, Entry::new(manifest, &path, text))
        })
        .collect();
    entries.sort_by(|(a, x), (b, y)| (a, &x.path).cmp(&(b, &y.path)));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

/// The heading of the charts in the folder `folder`: the title of each of its names under the
/// directory of the `[charts]` pattern that matches its charts, with ` / ` between them, or
/// `Charts` for that directory itself.
fn section(manifest: &Manifest, folder: &str) -> String {
    let base = (manifest.charts.scripts.iter())
        .map(|pattern| pattern.directory())
        .filter(|base| {
            base.is_empty() || folder == *base || folder.starts_with(&format!("{base}/"))
        })
        .max_by_key(|base| base.len())
        .unwrap_or_default();
    let names = folder[base.len()..].trim_start_matches('/');
    match names.is_empty() {
        true => "Charts".to_string(),
        false => (names.split('/'))
            .map(heading)
            .collect::<Vec<_>>()
            .join(" / "),
    }
}

/// The heading of a folder's name or the title of a file stem: the title of its text after a
/// leading number and `-`, which orders the name among the others and is not part of its title.
pub fn heading(name: &str) -> String {
    let unordered = name.trim_start_matches(|c: char| c.is_ascii_digit());
    match unordered.strip_prefix('-') {
        Some(rest) if unordered.len() < name.len() && !rest.is_empty() => title(rest),
        _ => title(name),
    }
}

/// The title of a file stem or a name: its text with a space for each `-`, and its first letter
/// a capital.
pub fn title(name: &str) -> String {
    let mut chars = name.chars().map(|c| if c == '-' { ' ' } else { c });
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The description of a chart script's text `text`: the comment lines at its start, each line's
/// text after `--` and a space, with the lines of a paragraph joined by spaces, and a blank line
/// between paragraphs, which a line `--` alone separates.
pub fn description(text: &str) -> String {
    let mut paragraphs: Vec<String> = vec![String::new()];
    for line in text.lines() {
        let Some(comment) = line.trim_end().strip_prefix("--") else {
            break;
        };
        let comment = comment.strip_prefix(' ').unwrap_or(comment).trim();
        let paragraph = paragraphs.last_mut().expect("a paragraph");
        match (comment.is_empty(), paragraph.is_empty()) {
            (true, true) => {}
            (true, false) => paragraphs.push(String::new()),
            (false, empty) => {
                if !empty {
                    paragraph.push(' ');
                }
                paragraph.push_str(comment);
            }
        }
    }
    paragraphs.retain(|paragraph| !paragraph.is_empty());
    paragraphs.join("\n\n")
}
