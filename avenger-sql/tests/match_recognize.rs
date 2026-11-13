//! `MATCH_RECOGNIZE` through the pass against GoogleSQL: a corpus of seeded random cases, whose
//! results GoogleSQL's `execute_query` recorded in `data/googlesql_match_recognize.txt`.
//!
//! A case is a table of at most 12 rows in at most 3 partitions, with a unique order key and a
//! value that can be `NULL`, a random pattern of up to three variables whose conditions read the
//! row and its neighbors through `PREV` and `NEXT`, a skip mode, and `use_longest_match`. Its query
//! writes every match as one string: the partition, the match number, `COUNT(*)`, each row's
//! variable, and each row's `MATCH_ROW_NUMBER()`, so the string shows the whole match.
//!
//! The ignored test `record_the_corpus` runs `execute_query` on the cases and writes their
//! results to `tests/output/googlesql_match_recognize.txt`; a copy into `tests/data/` accepts
//! them. Without the binary, it skips with a message.
//!
//! ```bash
//! cargo test --release -p avenger-sql --test match_recognize -- --ignored
//! ```

mod pass;

use datafusion::arrow::{
    array::{Array, AsArray},
    compute::cast,
    datatypes::DataType,
};

/// A xorshift generator of the corpus.
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

    /// A random pattern of the variables `x`, `y` and `z`, in GoogleSQL's syntax.
    fn pattern(&mut self, depth: u32) -> String {
        if depth == 0 || self.below(10) < 3 {
            return match self.below(20) {
                0 | 1 => "()".to_string(),
                2 => "^".to_string(),
                3 => "$".to_string(),
                n => ["x", "y", "z"][n as usize % 3].to_string(),
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

/// The conditions of the variables, which read the row and its neighbors.
const CONDITIONS: [&str; 10] = [
    "v > PREV(v)",
    "v < PREV(v)",
    "v = 0",
    "v >= NEXT(v)",
    "v IS NULL",
    "TRUE",
    "MOD(id, 2) = 0",
    "v > PREV(v, 2)",
    "v <= NEXT(v, 0) AND v > 1",
    "PREV(v) IS NULL",
];

/// A case of the corpus.
struct Case {
    /// The rows: the partition, the order key, and the value.
    rows: Vec<(Option<&'static str>, i64, Option<i64>)>,
    partitioned: bool,
    pattern: String,
    /// The condition of `x`, `y` and `z`, as indices of [`CONDITIONS`].
    conditions: [usize; 3],
    skip_to_next_row: bool,
    longest: bool,
}

impl Case {
    fn random(random: &mut Random) -> Case {
        let partitions = random.below(4) as usize;
        let keys = [Some("a"), Some("b"), None];
        let count = random.below(13) as usize;
        // The order keys, unique and shuffled.
        let mut ids: Vec<i64> = (1..=count as i64).collect();
        for i in (1..ids.len()).rev() {
            ids.swap(i, random.below(i as u64 + 1) as usize);
        }
        let rows = (ids.into_iter())
            .map(|id| {
                let key = match partitions {
                    0 => Some("a"),
                    n => keys[random.below(n as u64) as usize],
                };
                let value = match random.below(10) {
                    0 => None,
                    _ => Some(random.below(4) as i64),
                };
                (key, id, value)
            })
            .collect();
        let pattern = random.pattern(3);
        let conditions = [
            random.below(CONDITIONS.len() as u64) as usize,
            random.below(CONDITIONS.len() as u64) as usize,
            random.below(CONDITIONS.len() as u64) as usize,
        ];
        Case {
            rows,
            partitioned: partitions > 0,
            pattern,
            conditions,
            skip_to_next_row: random.below(2) == 0,
            longest: random.below(10) < 3,
        }
    }

    /// The query, whose one cell is every match, in the order of the partitions and the match
    /// numbers, as `partition/number:count:variables:row numbers`.
    fn query(&self) -> String {
        let data = match self.rows.is_empty() {
            true => "FROM UNNEST(ARRAY<STRUCT<p STRING, id INT64, v INT64>>[])".to_string(),
            false => format!(
                "FROM UNNEST([{}])",
                (self.rows.iter())
                    .map(|(key, id, value)| {
                        let key = key
                            .map_or("CAST(NULL AS STRING)".to_string(), |key| format!("'{key}'"));
                        let value = value
                            .map_or("CAST(NULL AS INT64)".to_string(), |value| value.to_string());
                        format!("STRUCT({key} AS p, {id} AS id, {value} AS v)")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let (partition, key) = match self.partitioned {
            true => ("PARTITION BY p ", "IFNULL(p, '-')"),
            false => ("", "'-'"),
        };
        let skip = match self.skip_to_next_row {
            true => "TO NEXT ROW",
            false => "PAST LAST ROW",
        };
        let [x, y, z] = self.conditions.map(|index| CONDITIONS[index]);
        let p = if self.partitioned { "p, " } else { "" };
        format!(
            "{data} |> SELECT * |> MATCH_RECOGNIZE({partition}ORDER BY id MEASURES MATCH_NUMBER() AS m, COUNT(*) AS n, ARRAY_TO_STRING(ARRAY_AGG(CLASSIFIER()), '') AS c, ARRAY_TO_STRING(ARRAY_AGG(CAST(MATCH_ROW_NUMBER() AS STRING)), ',') AS r AFTER MATCH SKIP {skip} PATTERN (({}) x{{0}} y{{0}} z{{0}}) DEFINE x AS {x}, y AS {y}, z AS {z} OPTIONS (use_longest_match = {})) |> AGGREGATE STRING_AGG(CONCAT({key}, '/', CAST(m AS STRING), ':', CAST(n AS STRING), ':', IFNULL(c, ''), ':', IFNULL(r, '')), ' ' ORDER BY {p}m) AS j",
            self.pattern,
            self.longest
        )
    }
}

/// The cases of the corpus, from one seed.
fn corpus() -> Vec<Case> {
    let mut random = Random(0x6d72_5f70_6173_7321);
    (0..500).map(|_| Case::random(&mut random)).collect()
}

#[tokio::test]
async fn the_pass_finds_the_corpus_matches_of_googlesql() {
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
        let query = case.query();
        let (recorded_query, expected) = line.split_once('\t').unwrap_or((line, ""));
        assert_eq!(
            recorded_query, query,
            "the corpus is recorded for its cases"
        );
        let found = match pass::run(&query).await {
            Ok(batches) => {
                let column = cast(batches[0].column(0), &DataType::Utf8).unwrap();
                let column = column.as_string::<i32>();
                match column.is_valid(0) {
                    true => column.value(0).to_string(),
                    false => String::new(),
                }
            }
            Err(errors) => format!("error: {errors:?}"),
        };
        if found != expected {
            failures.push(format!(
                "{query}\n  avenger   {found}\n  GoogleSQL {expected}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
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
        let query = case.query();
        let output = std::process::Command::new(&binary)
            .args([
                "--mode=execute",
                "--enabled_language_features=MAXIMUM",
                &query,
            ])
            .output()
            .expect("execute_query runs");
        let out = String::from_utf8_lossy(&output.stdout);
        let cell = (out.lines())
            .filter(|line| line.starts_with('│'))
            .nth(1)
            .map(|line| line.trim_matches(|c| c == '│' || c == ' ').to_string())
            .unwrap_or_else(|| {
                panic!(
                    "a result of {query}: {out}{}",
                    String::from_utf8_lossy(&output.stderr)
                )
            });
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
        .map(|(case, result)| format!("{}\t{result}\n", case.query()))
        .collect();
    let output = root.join("tests/output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("googlesql_match_recognize.txt"), text).unwrap();
}
