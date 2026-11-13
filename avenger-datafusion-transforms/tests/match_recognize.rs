//! The matcher of `MATCH_RECOGNIZE` against GoogleSQL: the cases that `execute_query` ran, with
//! their rows' variables as booleans, and a corpus of seeded random patterns and rows whose
//! matches `execute_query` recorded in `data/googlesql_match_recognize.txt`.
//!
//! The ignored test `record_the_corpus` runs `execute_query` on the corpus's cases and writes
//! their matches to `tests/output/googlesql_match_recognize.txt`; a copy into `tests/data/`
//! accepts them. Without the binary, it skips with a message.
//!
//! ```bash
//! cargo test --release -p avenger-datafusion-transforms --test match_recognize -- --ignored
//! ```

use avenger_datafusion_transforms::match_recognize::{
    Anchor, Pattern, Program, Skip, Tail, matches, tail_matches,
};

// --- A parser of the tests' patterns ---

/// The pattern of `text`, in GoogleSQL's syntax, whose variables are single letters, numbered by
/// `variables`.
fn pattern(text: &str, variables: &str) -> Pattern {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut at = 0;
    let parsed = alternation(&chars, &mut at, variables);
    assert_eq!(at, chars.len(), "the pattern {text} parses to its end");
    parsed
}

fn alternation(chars: &[char], at: &mut usize, variables: &str) -> Pattern {
    let mut parts = vec![sequence(chars, at, variables)];
    while *at < chars.len() && chars[*at] == '|' {
        *at += 1;
        // `||` is `| () |`.
        if *at < chars.len() && chars[*at] == '|' {
            *at += 1;
            parts.push(Pattern::Empty);
        }
        parts.push(sequence(chars, at, variables));
    }
    match parts.len() {
        1 => parts.pop().unwrap(),
        _ => Pattern::Alternation(parts),
    }
}

fn sequence(chars: &[char], at: &mut usize, variables: &str) -> Pattern {
    let mut parts = Vec::new();
    while *at < chars.len() && !matches!(chars[*at], '|' | ')') {
        let term = match chars[*at] {
            '^' => {
                *at += 1;
                parts.push(Pattern::Anchor(Anchor::Start));
                continue;
            }
            '$' => {
                *at += 1;
                parts.push(Pattern::Anchor(Anchor::End));
                continue;
            }
            '(' => {
                *at += 1;
                let inner = alternation(chars, at, variables);
                assert_eq!(chars[*at], ')');
                *at += 1;
                inner
            }
            c => {
                *at += 1;
                Pattern::Variable(variables.find(c).expect("a variable"))
            }
        };
        parts.push(quantified(chars, at, term));
    }
    match parts.len() {
        0 => Pattern::Empty,
        1 => parts.pop().unwrap(),
        _ => Pattern::Concat(parts),
    }
}

fn quantified(chars: &[char], at: &mut usize, term: Pattern) -> Pattern {
    let number = |at: &mut usize| -> Option<u32> {
        let start = *at;
        while chars[*at].is_ascii_digit() {
            *at += 1;
        }
        (start < *at).then(|| {
            chars[start..*at]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap()
        })
    };
    let (min, max, exact) = match chars.get(*at) {
        Some('?') => (0, Some(1), false),
        Some('*') => (0, None, false),
        Some('+') => (1, None, false),
        Some('{') => {
            *at += 1;
            let min = number(at);
            let (max, exact) = match chars[*at] {
                ',' => {
                    *at += 1;
                    (number(at), false)
                }
                _ => (min, true),
            };
            assert_eq!(chars[*at], '}');
            (min.unwrap_or(0), max, exact)
        }
        _ => return term,
    };
    *at += 1;
    let reluctant = !exact && chars.get(*at) == Some(&'?');
    if reluctant {
        *at += 1;
    }
    Pattern::Repeat {
        body: Box::new(term),
        min,
        max,
        greedy: !reluctant,
    }
}

fn vm(
    p: &Pattern,
    rows: usize,
    meets: &dyn Fn(usize, usize) -> bool,
    skip: Skip,
    longest: bool,
) -> Vec<(usize, Vec<usize>)> {
    let program = Program::compile(p).expect("a program");
    let found = matches(&program, rows, meets, skip, longest);
    for (index, m) in found.iter().enumerate() {
        assert_eq!(m.number, index as u64 + 1, "matches are numbered in order");
    }
    found.into_iter().map(|m| (m.start, m.variables)).collect()
}

/// `matches` written as `start:variables`, rows from 1, as the cases write them.
fn written(found: &[(usize, Vec<usize>)], variables: &str) -> String {
    let names: Vec<char> = variables.chars().collect();
    (found.iter())
        .map(|(start, path)| {
            let vars: String = path.iter().map(|v| names[*v]).collect();
            format!("{}:{vars}", start + 1)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- The cases of execute_query ---

/// The rows of GoogleSQL's cases: `f` is 1 1 0 1 1 1 0. The variables: `u` where f = 1, `d`
/// where f = 0, `x` and `q` always, and `p` where f = 1.
const FLAGS: [bool; 7] = [true, true, false, true, true, true, false];
const FLAG_VARIABLES: &str = "udxqp";

fn flags(row: usize, variable: usize) -> bool {
    match FLAG_VARIABLES.as_bytes()[variable] {
        b'u' | b'p' => FLAGS[row],
        b'd' => !FLAGS[row],
        _ => true,
    }
}

/// The rows of the corner cases of iterations that match no rows: `f` is 1 1 0 1, `a` where
/// f = 1 and `b` where f = 0.
const SHORT: [bool; 4] = [true, true, false, true];

fn short(row: usize, variable: usize) -> bool {
    match variable {
        0 => SHORT[row],
        _ => !SHORT[row],
    }
}

#[test]
fn the_matcher_finds_googlesqls_matches() {
    // Each case: the pattern, its skip mode, `use_longest_match`, and the matches that
    // `execute_query` returned, as `start:variables` (`scratch/projects/match-recognize/`).
    let past = Skip::PastLastRow;
    let next = Skip::ToNextRow;
    let flag_cases: &[(&str, Skip, bool, &str)] = &[
        ("u* x", past, false, "1:uux 4:uuux"),
        ("u*? x", past, false, "1:x 2:x 3:x 4:x 5:x 6:x 7:x"),
        ("u+ u", past, false, "1:uu 4:uuu"),
        ("u+?", past, false, "1:u 2:u 4:u 5:u 6:u"),
        ("u? d", past, false, "2:ud 6:ud"),
        ("u?? x", past, false, "1:x 2:x 3:x 4:x 5:x 6:x 7:x"),
        ("u{2}", past, false, "1:uu 4:uu"),
        ("u{1,2}", past, false, "1:uu 4:uu 6:u"),
        ("u{1,2}?", past, false, "1:u 2:u 4:u 5:u 6:u"),
        ("u{3,}", past, false, "4:uuu"),
        ("u{,2} d", past, false, "1:uud 5:uud"),
        ("u{0}", past, false, "1: 2: 3: 4: 5: 6: 7:"),
        ("u{,}", past, false, "1:uu 3: 4:uuu 7:"),
        ("u{,2}?", past, false, "1: 2: 3: 4: 5: 6: 7:"),
        ("(u?)?", past, false, "1:u 2:u 3: 4:u 5:u 6:u 7:"),
        ("(u u)+", past, false, "1:uu 4:uu"),
        ("q* x*", past, false, "1:qqqqqqq"),
        ("q*? x*", past, false, "1:xxxxxxx"),
        ("p | q", past, false, "1:p 2:p 3:q 4:p 5:p 6:p 7:q"),
        ("u | u u", past, false, "1:u 2:u 4:u 5:u 6:u"),
        ("u | u u", past, true, "1:uu 4:uu 6:u"),
        ("u+?", past, true, "1:uu 4:uuu"),
        ("d || d", past, false, "1: 2: 3:d 4: 5: 6: 7:d"),
        ("d*", past, false, "1: 2: 3:d 4: 5: 6: 7:d"),
        ("u*", past, false, "1:uu 3: 4:uuu 7:"),
        ("u u", past, false, "1:uu 4:uu"),
        ("u u", next, false, "1:uu 4:uu 5:uu"),
        ("u+", past, false, "1:uu 4:uuu"),
        ("u+", next, false, "1:uu 2:u 4:uuu 5:uu 6:u"),
    ];
    let short_cases: &[(&str, &str)] = &[
        ("(() | a)*", "1:aa 3: 4:a"),
        ("(a | ())*", "1:aa 3: 4:a"),
        ("(a??)*", "1:aa 3: 4:a"),
        ("(a*)*", "1:aa 3: 4:a"),
        ("(a?){2,3}", "1:aa 3: 4:a"),
        ("(a??){2}", "1: 2: 3: 4:"),
        ("(()|a){1,2}", "1: 2: 3: 4:"),
        ("(() | a)+", "1: 2: 3: 4:"),
        ("(()|a)*?", "1: 2: 3: 4:"),
        ("a{0}", "1: 2: 3: 4:"),
        ("^ a", "1:a"),
        ("a $", "4:a"),
        ("(^ | a) a", "1:a"),
        ("(a | ^)+", "1:aa 4:a"),
        ("(a* | b)*", "1:aaba"),
        ("(() | a)+ b", "1:aab"),
        ("(() | a){0,3}", "1: 2: 3: 4:"),
        ("(() | a){0,3} b", "1:aab"),
        ("(() | a){0,3} a", "1:a 2:a 4:a"),
        ("(a??){1,3}", "1: 2: 3: 4:"),
        ("(() | a){2,}", "1: 2: 3: 4:"),
        ("(()|a)? a", "1:a 2:a 4:a"),
        ("((() | a) b?)*", "1:aaba"),
        ("(a | ()){0,3}", "1:aa 3: 4:a"),
        ("(a?)?", "1:a 2:a 3: 4:a"),
        ("(a??)?", "1: 2: 3: 4:"),
        ("(a??){0,}", "1:aa 3: 4:a"),
        ("(a | ())+ b", "1:aab"),
        ("(()|a)* b", "1:aab"),
        ("(a?)* b", "1:aab"),
        ("(a | a a)* b", "1:aab"),
        ("a* | b", "1:aa 3: 4:a"),
        ("((() a)*?){0,}", "1:a 2:a 3: 4:a"),
    ];
    let mut failures = Vec::new();
    for (text, skip, longest, expected) in flag_cases {
        let p = pattern(text, FLAG_VARIABLES);
        let found = written(
            &vm(&p, FLAGS.len(), &flags, *skip, *longest),
            FLAG_VARIABLES,
        );
        if found != *expected {
            failures.push(format!(
                "{text} {skip:?} longest={longest}: {found}, GoogleSQL {expected}"
            ));
        }
    }
    for (text, expected) in short_cases {
        let p = pattern(text, "ab");
        let found = written(&vm(&p, SHORT.len(), &short, Skip::PastLastRow, false), "ab");
        if found != *expected {
            failures.push(format!("{text}: {found}, GoogleSQL {expected}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn no_rows_have_no_matches() {
    let p = pattern("a*", "a");
    assert!(vm(&p, 0, &|_, _| true, Skip::PastLastRow, false).is_empty());
}

// --- The corpus of random cases ---

/// A seeded xorshift generator, so the corpus reproduces.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A random pattern of the variables `a`, `b` and `c`, in GoogleSQL's syntax.
    fn pattern(&mut self, depth: u32) -> String {
        if depth == 0 || self.below(10) < 3 {
            return match self.below(20) {
                0 | 1 => "()".to_string(),
                2 => "^".to_string(),
                3 => "$".to_string(),
                n => ["a", "b", "c"][n as usize % 3].to_string(),
            };
        }
        let count = 2 + self.below(2);
        let parts = |random: &mut Random| -> Vec<String> {
            (0..count).map(|_| random.pattern(depth - 1)).collect()
        };
        match self.below(3) {
            0 => parts(self).join(" "),
            1 => format!("({})", parts(self).join(" | ")),
            _ => {
                let body = self.pattern(depth - 1);
                const QUANTIFIERS: [&str; 10] = [
                    "*", "+", "?", "{0,1}", "{1,2}", "{2}", "{0,}", "{1,}", "{2,}", "{,2}",
                ];
                let quantifier = QUANTIFIERS[self.below(QUANTIFIERS.len() as u64) as usize];
                let reluctant = quantifier != "{2}" && self.below(5) < 2;
                format!("({body}){quantifier}{}", if reluctant { "?" } else { "" })
            }
        }
    }
}

/// A case of the corpus: the pattern, the rows (whether each meets `a`, `b` and `c`), the skip
/// mode and `use_longest_match`.
struct Case {
    pattern: String,
    rows: Vec<[bool; 3]>,
    skip: Skip,
    longest: bool,
}

impl Case {
    fn random(random: &mut Random) -> Case {
        let pattern = random.pattern(3);
        let rows = (0..random.below(8))
            .map(|_| {
                [
                    random.below(2) == 0,
                    random.below(2) == 0,
                    random.below(2) == 0,
                ]
            })
            .collect();
        let skip = match random.below(2) {
            0 => Skip::PastLastRow,
            _ => Skip::ToNextRow,
        };
        Case {
            pattern,
            rows,
            skip,
            longest: random.below(10) < 3,
        }
    }

    /// The rows as the corpus writes them: `1` where a row meets `a`, `b` and `c`.
    fn rows_text(&self) -> String {
        (self.rows.iter())
            .map(|row| {
                row.iter()
                    .map(|b| if *b { '1' } else { '0' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn skip_text(&self) -> &str {
        match self.skip {
            Skip::PastLastRow => "past",
            Skip::ToNextRow => "next",
        }
    }

    /// The pattern with every variable used, as GoogleSQL requires.
    fn full_pattern(&self) -> String {
        format!("({}) a{{0}} b{{0}} c{{0}}", self.pattern)
    }

    /// The query of `execute_query`, whose one cell is the matches as `start:variables` in
    /// their order, `-` for the start of an empty match.
    fn query(&self) -> String {
        let data = match self.rows.is_empty() {
            true => "FROM UNNEST(ARRAY<STRUCT<id INT64, fa BOOL, fb BOOL, fc BOOL>>[])".to_string(),
            false => format!(
                "FROM UNNEST([{}])",
                (self.rows.iter().enumerate())
                    .map(|(i, [a, b, c])| {
                        format!("STRUCT({} AS id, {a} AS fa, {b} AS fb, {c} AS fc)", i + 1)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let skip = match self.skip {
            Skip::PastLastRow => "PAST LAST ROW",
            Skip::ToNextRow => "TO NEXT ROW",
        };
        format!(
            "{data} |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MATCH_NUMBER() AS m, MIN(id) AS s, ARRAY_TO_STRING(ARRAY_AGG(CLASSIFIER()), '') AS c AFTER MATCH SKIP {skip} PATTERN ({}) DEFINE a AS fa, b AS fb, c AS fc OPTIONS (use_longest_match = {})) |> AGGREGATE STRING_AGG(CONCAT(IFNULL(CAST(s AS STRING), '-'), ':', IFNULL(c, '')), ' ' ORDER BY m) AS j",
            self.full_pattern(),
            self.longest
        )
    }

    /// The matches of the matcher, as the corpus writes them.
    fn ours(&self) -> String {
        let p = pattern(&self.full_pattern(), "abc");
        let meets = |row: usize, variable: usize| self.rows[row][variable];
        let found = vm(&p, self.rows.len(), &meets, self.skip, self.longest);
        (found.iter())
            .map(|(start, path)| {
                let vars: String = path.iter().map(|v| ['a', 'b', 'c'][*v]).collect();
                let start = match path.is_empty() {
                    true => "-".to_string(),
                    false => (start + 1).to_string(),
                };
                format!("{start}:{vars}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The cases of the corpus, from one seed.
fn corpus() -> Vec<Case> {
    let mut random = Random(0x6d61_7463_685f_7265);
    (0..1000).map(|_| Case::random(&mut random)).collect()
}

#[test]
fn the_matcher_finds_the_corpus_matches_of_googlesql() {
    let recorded: Vec<&str> = include_str!("data/googlesql_match_recognize.txt")
        .lines()
        .collect();
    let cases = corpus();
    assert_eq!(
        recorded.len(),
        cases.len(),
        "the corpus is recorded for its cases"
    );
    let mut failures = Vec::new();
    for (case, line) in cases.iter().zip(recorded) {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields[0], case.pattern,
            "the corpus is recorded for its cases"
        );
        let expected = fields.get(4).copied().unwrap_or("");
        let ours = case.ours();
        if ours != expected {
            failures.push(format!(
                "{} over {} {} longest={}\n  avenger   {ours}\n  GoogleSQL {expected}",
                case.pattern,
                case.rows_text(),
                case.skip_text(),
                case.longest
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
#[ignore = "runs GoogleSQL's execute_query; see the module's docs"]
fn record_the_corpus() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let binary = root.join("../scratch/tools/googlesql/execute_query_macos");
    if !binary.exists() {
        eprintln!("skipped: the corpus needs {}", binary.display());
        return;
    }
    let cases = corpus();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = cases.len().div_ceil(threads);
    let run = |case: &Case| -> String {
        let output = std::process::Command::new(&binary)
            .args([
                "--mode=execute",
                "--enabled_language_features=MAXIMUM",
                &case.query(),
            ])
            .output()
            .expect("execute_query runs");
        let out = String::from_utf8_lossy(&output.stdout);
        let cell = (out.lines())
            .filter(|line| line.starts_with('│'))
            .nth(1)
            .map(|line| line.trim_matches(|c| c == '│' || c == ' ').to_string())
            .unwrap_or_else(|| panic!("a result of {}: {out}", case.query()));
        match cell.as_str() {
            "NULL" => String::new(),
            _ => cell,
        }
    };
    let results: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (cases.chunks(chunk))
            .map(|cases| scope.spawn(|| cases.iter().map(run).collect::<Vec<_>>()))
            .collect();
        (handles.into_iter())
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    let text: String = (cases.iter().zip(results))
        .map(|(case, result)| {
            format!(
                "{}\t{}\t{}\t{}\t{result}\n",
                case.pattern,
                case.rows_text(),
                case.skip_text(),
                case.longest
            )
        })
        .collect();
    let output = root.join("tests/output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("googlesql_match_recognize.txt"), text).unwrap();
}

/// The matches of a streaming matcher at each prefix of `case`'s rows: the final matches before
/// each frontier, kept, and the tail's matches from the frontier, found again for each new row.
fn streamed(program: &Program, case: &Case, prefix: usize) -> Vec<(usize, Vec<usize>)> {
    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut frontier = 0;
    let mut current = Vec::new();
    for rows in 1..=prefix {
        let meets = |row: usize, variable: usize| case.rows[row][variable];
        let tail = Tail {
            start: frontier,
            partition_start: true,
            firm: rows,
        };
        let (found, settled) = tail_matches(program, rows, meets, case.skip, case.longest, tail);
        current = kept.clone();
        for m in found {
            let entry = (m.start, m.variables);
            if m.start < settled {
                kept.push(entry.clone());
            }
            current.push(entry);
        }
        current.sort();
        current.dedup();
        frontier = settled;
    }
    current
}

#[test]
fn a_streaming_matcher_finds_the_matches_of_each_prefix() {
    let mut failures = Vec::new();
    for case in corpus() {
        let p = pattern(&case.full_pattern(), "abc");
        let program = Program::compile(&p).unwrap();
        for prefix in 1..=case.rows.len() {
            let meets = |row: usize, variable: usize| case.rows[row][variable];
            let mut batch: Vec<(usize, Vec<usize>)> =
                matches(&program, prefix, meets, case.skip, case.longest)
                    .into_iter()
                    .map(|m| (m.start, m.variables))
                    .collect();
            batch.sort();
            let streaming = streamed(&program, &case, prefix);
            if streaming != batch {
                failures.push(format!(
                    "{} over {} {} longest={} prefix={prefix}\n  streaming {streaming:?}\n  batch     {batch:?}",
                    case.pattern,
                    case.rows_text(),
                    case.skip_text(),
                    case.longest
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn a_streaming_matcher_takes_the_last_rows_conditions_as_provisional() {
    // As a condition reads `NEXT`, the last row of a prefix has conditions that change when the
    // next row arrives: here, the inverse of its final conditions.
    let view = |case: &Case, prefix: usize, row: usize, variable: usize| match row + 1 == prefix {
        true => !case.rows[row][variable],
        false => case.rows[row][variable],
    };
    let mut failures = Vec::new();
    for case in corpus() {
        let p = pattern(&case.full_pattern(), "abc");
        let program = Program::compile(&p).unwrap();
        let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut frontier = 0;
        for prefix in 1..=case.rows.len() {
            let meets = |row: usize, variable: usize| view(&case, prefix, row, variable);
            let mut batch: Vec<(usize, Vec<usize>)> =
                matches(&program, prefix, meets, case.skip, case.longest)
                    .into_iter()
                    .map(|m| (m.start, m.variables))
                    .collect();
            batch.sort();
            let tail = Tail {
                start: frontier,
                partition_start: true,
                firm: prefix - 1,
            };
            let (found, settled) =
                tail_matches(&program, prefix, meets, case.skip, case.longest, tail);
            let mut streaming = kept.clone();
            for m in found {
                if m.start < settled {
                    kept.push((m.start, m.variables.clone()));
                }
                streaming.push((m.start, m.variables));
            }
            streaming.sort();
            frontier = settled;
            if streaming != batch {
                failures.push(format!(
                    "{} over {} {} longest={} prefix={prefix}\n  streaming {streaming:?}\n  batch     {batch:?}",
                    case.pattern,
                    case.rows_text(),
                    case.skip_text(),
                    case.longest
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
