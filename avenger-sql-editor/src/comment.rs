//! Comments the lines of a selection out with `--`, or back in.

use std::ops::Range;

/// An edit of one line: `len` characters inserted at `at`, or removed from `at` when `insert` is
/// unset.
struct Edit {
    at: usize,
    len: usize,
    insert: bool,
}

/// Toggles the comments of the lines that `selection`, a range of characters, covers: when every
/// line that is not blank starts with `--` after its indentation, it removes the `--` and one space
/// after it, and otherwise it puts `-- ` at the smallest indentation of those lines. A selection
/// that ends at the start of a line leaves that line out, and blank lines stay as they are, except
/// that the line of an empty selection on a blank line takes the comment. Returns the text and the
/// selection over the same characters.
pub fn toggle(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let (start, end) = (
        selection.start.min(chars.len()),
        selection.end.min(chars.len()),
    );
    // The start of each line, by character.
    let mut starts = vec![0];
    starts.extend(
        (0..chars.len())
            .filter(|&i| chars[i] == '\n')
            .map(|i| i + 1),
    );
    let line_of = |index: usize| starts.partition_point(|&start| start <= index) - 1;
    let first = line_of(start);
    let mut last = line_of(end);
    if end > start && last > first && starts[last] == end {
        last -= 1;
    }
    let line = |line: usize| {
        let from = starts[line];
        let to = starts.get(line + 1).map_or(chars.len(), |next| next - 1);
        &chars[from..to]
    };
    let indent = |chars: &[char]| {
        chars
            .iter()
            .take_while(|c| **c == ' ' || **c == '\t')
            .count()
    };
    let blank = |chars: &[char]| indent(chars) == chars.len();
    let mut lines: Vec<usize> = (first..=last).filter(|&l| !blank(line(l))).collect();
    if lines.is_empty() {
        lines.push(first);
    }
    let commented = (lines.iter()).all(|&l| line(l)[indent(line(l))..].starts_with(&['-', '-']));
    let edits: Vec<Edit> = match commented {
        true => (lines.iter())
            .map(|&l| {
                let rest = &line(l)[indent(line(l))..];
                let space = rest.get(2) == Some(&' ');
                Edit {
                    at: starts[l] + indent(line(l)),
                    len: 2 + usize::from(space),
                    insert: false,
                }
            })
            .collect(),
        false => {
            let column = (lines.iter()).map(|&l| indent(line(l))).min().unwrap_or(0);
            (lines.iter())
                .map(|&l| Edit {
                    at: starts[l] + column,
                    len: 3,
                    insert: true,
                })
                .collect()
        }
    };
    let mut out = String::with_capacity(text.len() + 3 * edits.len());
    let mut next = 0;
    for edit in &edits {
        out.extend(&chars[next..edit.at]);
        match edit.insert {
            true => {
                out.push_str("-- ");
                next = edit.at;
            }
            false => next = edit.at + edit.len,
        }
    }
    out.extend(&chars[next..]);
    let map = |index: usize| {
        let mut shift: isize = 0;
        for edit in &edits {
            match edit.insert {
                true if index >= edit.at => shift += edit.len as isize,
                false if index >= edit.at + edit.len => shift -= edit.len as isize,
                false if index > edit.at => shift -= (index - edit.at) as isize,
                _ => {}
            }
        }
        (index as isize + shift) as usize
    };
    (
        out,
        map(selection.start.min(chars.len()))..map(selection.end.min(chars.len())),
    )
}
