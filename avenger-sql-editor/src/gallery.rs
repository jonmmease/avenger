//! The example queries of the gallery, from `examples.md`: a `##` heading starts a section, a
//! `###` heading starts an example and is its title, the prose under the heading describes it, and
//! the first `sql` block after the heading is its query. The introduction, the prose under a `##`
//! heading, other code blocks, and the prose after an example's query belong to the document only.

use std::sync::LazyLock;

/// An example query of the gallery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    /// The `##` heading of its section.
    pub section: String,
    /// Its `###` heading.
    pub title: String,
    /// Its prose, with a blank line between paragraphs.
    pub description: String,
    /// The text of its `sql` block.
    pub sql: String,
}

/// The examples of the editor's `examples.md`, in the order of the file.
pub fn examples() -> &'static [Example] {
    static EXAMPLES: LazyLock<Vec<Example>> =
        LazyLock::new(|| parse(include_str!("../examples.md")));
    &EXAMPLES
}

/// The examples of the markdown `text`. A `###` heading without a `sql` block is not an example.
pub fn parse(text: &str) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut section = String::new();
    // The example that the lines are in: its title, its paragraphs, and its query.
    let mut current: Option<(String, Vec<String>, Option<String>)> = None;
    // The text of the open code block, and whether it is the query of the current example.
    let mut block: Option<(String, bool)> = None;
    let mut paragraph = String::new();
    let mut finish = |current: &mut Option<(String, Vec<String>, Option<String>)>,
                      section: &str| {
        if let Some((title, paragraphs, Some(sql))) = current.take() {
            examples.push(Example {
                section: section.to_string(),
                title,
                description: paragraphs.join("\n\n"),
                sql,
            });
        }
    };
    for line in text.lines() {
        if let Some((code, query)) = &mut block {
            if line.trim_end() == "```" {
                if *query {
                    if let Some((_, _, sql)) = &mut current {
                        *sql = Some(code.trim_end().to_string());
                    }
                }
                block = None;
            } else {
                code.push_str(line);
                code.push('\n');
            }
            continue;
        }
        let describing = matches!(&current, Some((_, _, None)));
        if !line.trim().is_empty() && !line.starts_with('#') && !line.starts_with("```") {
            if describing {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(line.trim());
            }
            continue;
        }
        if !paragraph.is_empty() {
            if let Some((_, paragraphs, _)) = &mut current {
                paragraphs.push(std::mem::take(&mut paragraph));
            }
        }
        if let Some(fence) = line.strip_prefix("```") {
            block = Some((String::new(), describing && fence.trim() == "sql"));
        } else if let Some(title) = line.strip_prefix("### ") {
            finish(&mut current, &section);
            current = Some((title.trim().to_string(), Vec::new(), None));
        } else if let Some(heading) = line.strip_prefix("## ") {
            finish(&mut current, &section);
            section = heading.trim().to_string();
        }
    }
    finish(&mut current, &section);
    examples
}
