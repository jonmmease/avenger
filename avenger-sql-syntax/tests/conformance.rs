//! Conformance with GoogleSQL's parser: an ignored test that parses a corpus with
//! `execute_query --mode=parse` and with [`avenger_sql_syntax::parse`], and compares whether each
//! accepts each input and where each finds its first error.
//!
//! The corpus is the inputs of GoogleSQL's parser tests in the subset, which
//! `tests/conformance/extract.py` writes to `scratch/tools/googlesql/parser-corpus.txt`, and the
//! inputs of this crate's tests. A difference that `tests/conformance/known.txt` lists is
//! explained; the test fails on any other, and writes its report to
//! `tests/output/conformance.md`. Without the binary or the corpus, it skips with a message.
//!
//! ```bash
//! python3 avenger-sql-syntax/tests/conformance/extract.py
//! cargo test --release -p avenger-sql-syntax --test conformance -- --ignored
//! ```

mod common;

use std::{fmt::Write as _, path::PathBuf, process::Command};

use avenger_sql_syntax::DiagnosticCode;

/// The root of the repository.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// What a parser says of an input: it accepts it, it rejects it with its first error at a line
/// and a column, from 1, or, for avenger, it reports a construct outside its subset there first.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Accepts,
    Rejects { line: usize, column: usize },
    Unsupported { line: usize, column: usize },
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Accepts => f.write_str("accepts"),
            Verdict::Rejects { line, column } => write!(f, "rejects at {line}:{column}"),
            Verdict::Unsupported { line, column } => {
                write!(f, "reports an unsupported construct at {line}:{column}")
            }
        }
    }
}

/// An input of the corpus, and its label.
struct Input {
    label: String,
    text: String,
}

/// The inputs of the corpus file `text`: a line `## <label>` before each.
fn corpus(text: &str) -> Vec<Input> {
    let mut inputs: Vec<Input> = Vec::new();
    for line in text.lines() {
        if let Some(label) = line.strip_prefix("## ") {
            inputs.push(Input {
                label: label.to_string(),
                text: String::new(),
            });
        } else if let Some(input) = inputs.last_mut() {
            if !input.text.is_empty() {
                input.text.push('\n');
            }
            input.text.push_str(line);
        }
    }
    inputs
}

/// The inputs of this crate's tests.
fn crate_inputs() -> Vec<Input> {
    let input = |label: String, text: &str| Input {
        label,
        text: text.trim().to_string(),
    };
    let mut inputs = vec![
        input("crate:OPERATORS".into(), common::OPERATORS),
        input("crate:STATEMENTS".into(), common::STATEMENTS),
        input("crate:SOURCES".into(), common::SOURCES),
    ];
    for (i, text) in common::QUERIES.iter().enumerate() {
        inputs.push(input(format!("crate:QUERIES:{i}"), text));
    }
    for (i, text) in common::EXPRESSIONS.iter().enumerate() {
        inputs.push(input(
            format!("crate:EXPRESSIONS:{i}"),
            &format!("FROM t |> WHERE {text}"),
        ));
    }
    for (i, text) in common::TRAILING_COMMAS.iter().enumerate() {
        inputs.push(input(format!("crate:TRAILING_COMMAS:{i}"), text));
    }
    for (i, (text, _, _)) in common::REJECTED.iter().enumerate() {
        inputs.push(input(format!("crate:REJECTED:{i}"), text));
    }
    inputs
}

/// The line and the column, from 1, of the byte offset `offset` of `text`.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
    (line, column)
}

/// Our verdict of `text`.
fn ours(text: &str) -> Verdict {
    let parse = avenger_sql_syntax::parse(text);
    match parse.diagnostics().iter().min_by_key(|d| d.range.start()) {
        None => Verdict::Accepts,
        Some(diagnostic) => {
            let (line, column) = position(text, usize::from(diagnostic.range.start()));
            match diagnostic.code {
                DiagnosticCode::Unsupported | DiagnosticCode::NotPipeSyntax => {
                    Verdict::Unsupported { line, column }
                }
                _ => Verdict::Rejects { line, column },
            }
        }
    }
}

/// The verdict of GoogleSQL's `binary` on `text`, which it parses as a script with every
/// language feature.
fn theirs(binary: &PathBuf, text: &str) -> Verdict {
    let output = Command::new(binary)
        .args([
            "--mode=parse",
            "--sql_mode=script",
            "--enabled_language_features=MAXIMUM",
            text,
        ])
        .output()
        .expect("execute_query runs");
    let out = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    let at = out.find("[at ").and_then(|start| {
        let rest = &out[start + 4..];
        let (place, _) = rest.split_once(']')?;
        let (line, column) = place.split_once(':')?;
        Some(Verdict::Rejects {
            line: line.trim().parse().ok()?,
            column: column.trim().parse().ok()?,
        })
    });
    match at {
        Some(verdict) => verdict,
        None if out.contains("ERROR") || out.contains("Syntax error") => {
            Verdict::Rejects { line: 0, column: 0 }
        }
        None => Verdict::Accepts,
    }
}

/// A line of `known.txt`: a label, or a prefix of labels ending in `*`, and the reason of its
/// difference.
struct Known {
    pattern: String,
    reason: String,
}

fn known() -> Vec<Known> {
    let text = include_str!("conformance/known.txt");
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (pattern, reason) = line.split_once('\t').expect("a label, a tab, and a reason");
            Known {
                pattern: pattern.trim().to_string(),
                reason: reason.trim().to_string(),
            }
        })
        .collect()
}

impl Known {
    fn matches(&self, label: &str) -> bool {
        match self.pattern.strip_suffix('*') {
            Some(prefix) => label.starts_with(prefix),
            None => label == self.pattern,
        }
    }
}

/// The result of an input whose verdicts differ.
struct Difference {
    label: String,
    text: String,
    ours: Verdict,
    theirs: Verdict,
}

#[test]
#[ignore = "runs GoogleSQL's execute_query; see the module's docs"]
fn conformance() {
    let binary = root().join("scratch/tools/googlesql/execute_query_macos");
    let corpus_path = root().join("scratch/tools/googlesql/parser-corpus.txt");
    if !binary.exists() || !corpus_path.exists() {
        eprintln!(
            "skipped: the conformance check needs {} and {}, which tests/conformance/extract.py writes",
            binary.display(),
            corpus_path.display()
        );
        return;
    }
    let mut inputs = corpus(&std::fs::read_to_string(&corpus_path).unwrap());
    inputs.extend(crate_inputs());

    // The binary's verdicts, a process for each input, on a thread for each core.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = inputs.len().div_ceil(threads);
    let verdicts: Vec<Verdict> = std::thread::scope(|scope| {
        let handles: Vec<_> = (inputs.chunks(chunk))
            .map(|inputs| {
                let binary = &binary;
                scope.spawn(move || {
                    (inputs.iter())
                        .map(|input| theirs(binary, &input.text))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    let known = known();
    let mut same = 0;
    let mut unsupported = 0;
    let mut rejected_unsupported = 0;
    let mut explained: Vec<(&str, Difference)> = Vec::new();
    let mut unexplained: Vec<Difference> = Vec::new();
    let mut used = vec![false; known.len()];
    for (input, theirs) in inputs.iter().zip(verdicts) {
        let ours = ours(&input.text);
        // A construct outside the subset agrees with GoogleSQL's acceptance, and where GoogleSQL
        // rejects the input, avenger reports an error too, so neither is a difference.
        match (&ours, &theirs) {
            (Verdict::Unsupported { .. }, Verdict::Accepts) => {
                unsupported += 1;
                continue;
            }
            (Verdict::Unsupported { .. }, Verdict::Rejects { .. }) => {
                rejected_unsupported += 1;
                continue;
            }
            _ if ours == theirs => {
                same += 1;
                continue;
            }
            _ => {}
        }
        let difference = Difference {
            label: input.label.clone(),
            text: input.text.clone(),
            ours,
            theirs,
        };
        match known.iter().position(|known| known.matches(&input.label)) {
            Some(index) => {
                used[index] = true;
                explained.push((&known[index].reason, difference));
            }
            None => unexplained.push(difference),
        }
    }
    // The entries of `known.txt` that explain no difference, which a change made obsolete.
    let stale: Vec<&Known> = (known.iter().zip(&used))
        .filter(|(_, used)| !**used)
        .map(|(known, _)| known)
        .collect();

    let mut report = String::from("# Conformance with GoogleSQL's parser\n\n");
    let _ = writeln!(
        report,
        "{} inputs: {same} with the same verdict, {unsupported} that GoogleSQL accepts and avenger reports as unsupported, {rejected_unsupported} that GoogleSQL rejects and avenger reports as unsupported, {} explained differences, {} unexplained.\n",
        inputs.len(),
        explained.len(),
        unexplained.len()
    );
    let input = |report: &mut String, d: &Difference| {
        let _ = writeln!(
            report,
            "### `{}`\n\navenger {}, GoogleSQL {}.\n\n```sql\n{}\n```\n",
            d.label, d.ours, d.theirs, d.text
        );
    };
    if !unexplained.is_empty() {
        report.push_str("## Unexplained differences\n\n");
        for difference in &unexplained {
            input(&mut report, difference);
        }
    }
    if !stale.is_empty() {
        report.push_str("## Known entries without a difference\n\n");
        for known in &stale {
            let _ = writeln!(report, "- `{}`: {}", known.pattern, known.reason);
        }
        report.push('\n');
    }
    report.push_str("## Explained differences\n\n");
    let mut reasons: Vec<&str> = explained.iter().map(|(reason, _)| *reason).collect();
    reasons.sort();
    reasons.dedup();
    for reason in reasons {
        let labels: Vec<&str> = (explained.iter())
            .filter(|(r, _)| *r == reason)
            .map(|(_, d)| d.label.as_str())
            .collect();
        let _ = writeln!(
            report,
            "- {reason} ({}): {}",
            labels.len(),
            labels.join(", ")
        );
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("conformance.md"), &report).unwrap();
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{} unexplained differences and {} known entries without one; the report is in tests/output/conformance.md",
        unexplained.len(),
        stale.len()
    );
}
