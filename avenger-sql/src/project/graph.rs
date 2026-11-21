//! The order of a project's data scripts, from the names that each one defines and reads.
//!
//! A data script depends on another when it reads a table, a view, or a schema that the other
//! defines without `TEMP`. A statement that defines a table in a schema, such as
//! `finance.orders`, reads the schema `finance`, and depends on the script that creates it. A
//! read of `finance.orders` in a query reads the table alone: the script that defines the table
//! depends on the script of the schema already. The graph reads these names from the parses of
//! the scripts, with [`script_names`], since the analysis of a script needs the order that the
//! graph gives.
//!
//! A read that no data script defines adds no dependency: it reads a table of the host, or a
//! table that doesn't exist, which the analysis of the script reports. A read of a name of a
//! pipe `WITH`, after the `WITH` in its query, reads the `WITH` query, not a table, so it adds no
//! dependency either. The reads in the functions of an imported module are not in the text of
//! the script, so they add no dependency.

use std::collections::{BTreeSet, VecDeque};

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};

use crate::{
    analysis::script_targets,
    catalog::{references, same_path},
};

/// What a name of a data script names: a table or a view, or a schema. A table and a schema of
/// the same name are different names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    Table,
    Schema,
}

/// A name that a data script defines or reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// The path of the name, without backticks.
    pub names: Vec<String>,
    pub kind: NameKind,
    /// The range of the name in the text of its script. For the schema of a table that a
    /// statement defines, the range of the schema's name.
    pub range: TextRange,
}

/// The names that a data script defines for other scripts, and the names that it reads from
/// them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptNames {
    /// The tables, views, and schemas that the script defines without `TEMP` and that no later
    /// statement of the script replaces, in the order of the text: the names of
    /// [`crate::data::DataAnalysis::exported`].
    pub definitions: Vec<Name>,
    /// The tables and the schemas that the script reads from other scripts or from the host, in
    /// the order of the text: the tables of its queries, and the schema of each table in a schema
    /// that it defines.
    pub reads: Vec<Name>,
}

/// The names that the data script `root`, which [`avenger_sql_syntax::parse_data`] reads,
/// defines for other scripts and reads from them, as its analysis reads them.
///
/// A read of a table that an earlier statement of the script defines, `TEMP` or not, reads that
/// statement's table, and a read of a table that a later statement defines is an error of the
/// analysis, so neither is a read from another script. A read in the statement that defines the
/// same name, when no earlier statement defines it, reads a table of another script or of the
/// host, as `FROM sales` does in `CREATE TEMP VIEW sales AS (FROM sales)`. A read of a name of a
/// pipe `WITH`, after the `WITH` in the query that holds it, reads the `WITH` query.
pub fn script_names(root: &SyntaxNode) -> ScriptNames {
    let statements: Vec<ast::Stmt> = (ast::Script::cast(root.clone()).into_iter())
        .flat_map(|script| script.stmts())
        .collect();
    let targets = script_targets(&statements);
    let creates_schema = |names: &[String]| {
        (targets.iter().flatten())
            .any(|defined| defined.target.schema && same_path(&defined.target.names, names))
    };
    let mut definitions = Vec::new();
    let mut reads = Vec::new();
    for defined in targets.iter().flatten() {
        let target = &defined.target;
        // An earlier statement that creates the schema holds it, and a later one is an error of
        // the analysis.
        if let Some((names, range)) = target.read_schema() {
            if !creates_schema(&names) {
                reads.push(Name {
                    names,
                    kind: NameKind::Schema,
                    range,
                });
            }
        }
        if defined.exported() {
            definitions.push(Name {
                names: target.names.clone(),
                kind: match target.schema {
                    true => NameKind::Schema,
                    false => NameKind::Table,
                },
                range: target.path.syntax().text_range(),
            });
        }
    }
    let scopes = with_scopes(root);
    // The paths of the tables in `FROM` and `JOIN`, which the names of a pipe `WITH` can name.
    let sources: Vec<TextRange> = (root.descendants())
        .filter_map(ast::TableSource::cast)
        .filter_map(|source| Some(source.path()?.range()))
        .collect();
    for reference in references(root).tables {
        let later = (statements.iter().zip(&targets)).any(|(statement, defined)| {
            statement.syntax().text_range().start() > reference.range.start()
                && defined.as_ref().is_some_and(|defined| {
                    !defined.target.schema && same_path(&defined.target.names, &reference.names)
                })
        });
        // Names match without regard to case, as in GoogleSQL.
        let with = match reference.names.as_slice() {
            [name] => {
                sources.contains(&reference.range)
                    && (scopes.iter()).any(|(with, scope)| {
                        scope.contains_range(reference.range) && with.eq_ignore_ascii_case(name)
                    })
            }
            _ => false,
        };
        if later || with {
            continue;
        }
        reads.push(Name {
            names: reference.names,
            kind: NameKind::Table,
            range: reference.range,
        });
    }
    reads.sort_by_key(|read| read.range.start());
    ScriptNames { definitions, reads }
}

/// The name of each query of a pipe `WITH` in `root`, with the range of its scope: from the end
/// of its query to the end of the query that holds the `WITH`, which holds the later queries of
/// the `WITH` and the pipe operators after it.
fn with_scopes(root: &SyntaxNode) -> Vec<(String, TextRange)> {
    let mut scopes = Vec::new();
    for with in root.descendants().filter_map(ast::PipeWith::cast) {
        let Some(query) = with.syntax().ancestors().find_map(ast::Query::cast) else {
            continue;
        };
        let end = query.syntax().text_range().end();
        for named in with.named_queries() {
            let Some(name) = named.name().and_then(|name| name.text()) else {
                continue;
            };
            let start = named.syntax().text_range().end();
            scopes.push((name, TextRange::new(start, end)));
        }
    }
    scopes
}

/// A read of one data script of a name that another defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The path of the script that reads.
    pub script: String,
    /// The path of the script that defines what it reads.
    pub dependency: String,
    /// The name that `script` reads, at its range in the text of `script`.
    pub read: Name,
    /// The range of the definition's name in the text of `dependency`.
    pub definition: TextRange,
}

/// A range in the text of a data script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub range: TextRange,
}

/// An error of the dependencies of a project's data scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// A name that two data scripts define without `TEMP`, at the name in each, `first` in the
    /// script whose path comes first. A name that three scripts define has an error for each
    /// pair.
    Duplicate {
        kind: NameKind,
        names: Vec<String>,
        first: Place,
        second: Place,
    },
    /// Data scripts that read each other's names in a cycle: in each script of the cycle, its
    /// first read of a name of the next, so the `dependency` of the last read is the `script` of
    /// the first. The first cycle starts at the first script by path that is in a cycle, and each
    /// later cycle at the first script by path that is in a cycle and in no earlier cycle. A
    /// script can be in more than one cycle, and has an error in one of them at least.
    Cycle { reads: Vec<Edge> },
}

impl GraphError {
    /// The message of the error, which each of its places shows.
    pub fn message(&self) -> String {
        match self {
            GraphError::Duplicate {
                kind,
                names,
                first,
                second,
            } => {
                let name = names.join(".");
                let (first, second) = (&first.path, &second.path);
                match kind {
                    NameKind::Table => format!(
                        "Table {name} is defined by both {first} and {second}: a table of a project is defined by one data script, or is TEMP in each"
                    ),
                    NameKind::Schema => format!(
                        "Schema {name}, which BigQuery calls a dataset, is created by both {first} and {second}: a schema of a project is created by one data script"
                    ),
                }
            }
            GraphError::Cycle { reads } => {
                let steps: Vec<String> = (reads.iter())
                    .map(|edge| {
                        let what = match edge.read.kind {
                            NameKind::Table => "",
                            NameKind::Schema => "schema ",
                        };
                        format!(
                            "{} reads {what}{} of {}",
                            edge.script,
                            edge.read.names.join("."),
                            edge.dependency
                        )
                    })
                    .collect();
                format!(
                    "Data scripts read each other's tables in a cycle: {}. A data script runs after the scripts that it reads, so none of these can run",
                    steps.join(", ")
                )
            }
        }
    }

    /// The places of the error: the two definitions of a duplicate name, or the read in each
    /// script of a cycle.
    pub fn places(&self) -> Vec<Place> {
        match self {
            GraphError::Duplicate { first, second, .. } => vec![first.clone(), second.clone()],
            GraphError::Cycle { reads } => (reads.iter())
                .map(|edge| Place {
                    path: edge.script.clone(),
                    range: edge.read.range,
                })
                .collect(),
        }
    }
}

/// The error at a read of the name `names` of the kind `kind`, which the data script at `script`
/// defines and which has no table, since `why`, a clause such as `which failed to load`, as a
/// loader and an editor's analysis of the project write it.
pub fn unavailable_read(kind: NameKind, names: &[String], script: &str, why: &str) -> String {
    let name = names.join(".");
    match kind {
        NameKind::Table => format!("Table {name} is defined by the data script {script}, {why}"),
        NameKind::Schema => format!("Schema {name} is created by the data script {script}, {why}"),
    }
}

/// The dependencies of a project's data scripts, their errors, and the order that they run in.
///
/// [`Graph::order`] holds the scripts that can run, each after the scripts that it depends on,
/// and among the scripts that are ready at the same time, by path. A script in a cycle, or that
/// depends on one, directly or through others, can't run: it is in [`Graph::blocked`] and not in
/// the order. Each script in a cycle is in the error of a cycle, so a blocked script that no
/// error names depends on a cycle. A name that two scripts define is an error, and blocks
/// neither: a script that reads the name depends on both, and a loader decides what each
/// registers.
#[derive(Clone, Debug)]
pub struct Graph {
    /// The paths of the scripts, sorted, and the names of each.
    scripts: Vec<(String, ScriptNames)>,
    edges: Vec<Edge>,
    /// For each script, the indices of the scripts that it depends on.
    dependencies: Vec<BTreeSet<usize>>,
    /// For each script, the indices of the scripts that depend on it.
    dependents: Vec<BTreeSet<usize>>,
    errors: Vec<GraphError>,
    order: Vec<usize>,
    blocked: Vec<usize>,
}

impl Graph {
    /// The graph of the data scripts `scripts`: the path of each, as its host names it, and its
    /// parse by [`avenger_sql_syntax::parse_data`]. The paths are distinct.
    pub fn new<P: Into<String>>(scripts: impl IntoIterator<Item = (P, SyntaxNode)>) -> Graph {
        let mut scripts: Vec<(String, ScriptNames)> = (scripts.into_iter())
            .map(|(path, root)| (path.into(), script_names(&root)))
            .collect();
        scripts.sort_by(|(a, _), (b, _)| a.cmp(b));
        let count = scripts.len();
        let mut edges = Vec::new();
        let mut pairs = Vec::new();
        let mut dependencies = vec![BTreeSet::new(); count];
        let mut dependents = vec![BTreeSet::new(); count];
        for (script, (path, names)) in scripts.iter().enumerate() {
            for read in &names.reads {
                for (dependency, (other, other_names)) in scripts.iter().enumerate() {
                    if dependency == script {
                        continue;
                    }
                    for definition in &other_names.definitions {
                        if definition.kind == read.kind && same_path(&definition.names, &read.names)
                        {
                            edges.push(Edge {
                                script: path.clone(),
                                dependency: other.clone(),
                                read: read.clone(),
                                definition: definition.range,
                            });
                            pairs.push((script, dependency));
                            dependencies[script].insert(dependency);
                            dependents[dependency].insert(script);
                        }
                    }
                }
            }
        }
        let mut errors = duplicates(&scripts);
        let (order, blocked) = order(&dependencies, &dependents);
        for cycle in cycles(&dependencies, &blocked) {
            let reads = (cycle.iter().enumerate())
                .map(|(at, &script)| {
                    let next = cycle[(at + 1) % cycle.len()];
                    let edge = (pairs.iter())
                        .position(|&pair| pair == (script, next))
                        .expect("a cycle follows the edges of the graph");
                    edges[edge].clone()
                })
                .collect();
            errors.push(GraphError::Cycle { reads });
        }
        Graph {
            scripts,
            edges,
            dependencies,
            dependents,
            errors,
            order,
            blocked,
        }
    }

    /// The paths of the scripts, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.scripts.iter().map(|(path, _)| path.as_str())
    }

    /// The names that the script at `path` defines and reads, or `None` when the graph has no
    /// script at `path`.
    pub fn names(&self, path: &str) -> Option<&ScriptNames> {
        let index = self.index(path)?;
        Some(&self.scripts[index].1)
    }

    /// Each read of a name that another script defines, by the scripts in the order of their
    /// paths and the reads of each in the order of its text.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The duplicate names, by the scripts in the order of their paths, and then the cycles.
    pub fn errors(&self) -> &[GraphError] {
        &self.errors
    }

    /// The scripts that can run, in the order that they run.
    pub fn order(&self) -> Vec<&str> {
        self.order.iter().map(|&index| self.path(index)).collect()
    }

    /// The scripts that can't run, since they are in a cycle or depend on one, by path.
    pub fn blocked(&self) -> Vec<&str> {
        self.blocked.iter().map(|&index| self.path(index)).collect()
    }

    /// The scripts that the script at `path` reads a name of, by path.
    pub fn dependencies(&self, path: &str) -> Vec<&str> {
        let Some(index) = self.index(path) else {
            return Vec::new();
        };
        (self.dependencies[index].iter())
            .map(|&index| self.path(index))
            .collect()
    }

    /// The scripts that depend on the script at `path`, directly or through others, which a
    /// change of the script reloads: those that can run in the order of [`Graph::order`], and
    /// then those that are blocked, by path.
    pub fn downstream(&self, path: &str) -> Vec<&str> {
        let Some(index) = self.index(path) else {
            return Vec::new();
        };
        let mut reached = BTreeSet::new();
        let mut pending = vec![index];
        while let Some(script) = pending.pop() {
            for &dependent in &self.dependents[script] {
                if dependent != index && reached.insert(dependent) {
                    pending.push(dependent);
                }
            }
        }
        (self.order.iter().chain(&self.blocked))
            .filter(|script| reached.contains(script))
            .map(|&script| self.path(script))
            .collect()
    }

    fn index(&self, path: &str) -> Option<usize> {
        (self.scripts)
            .binary_search_by(|(script, _)| script.as_str().cmp(path))
            .ok()
    }

    fn path(&self, index: usize) -> &str {
        &self.scripts[index].0
    }
}

/// The errors of the names that two scripts define, by pair of scripts in the order of their
/// paths.
fn duplicates(scripts: &[(String, ScriptNames)]) -> Vec<GraphError> {
    let mut errors = Vec::new();
    for (index, (first_path, first)) in scripts.iter().enumerate() {
        for (second_path, second) in &scripts[index + 1..] {
            for name in &first.definitions {
                let other = (second.definitions.iter())
                    .find(|other| other.kind == name.kind && same_path(&other.names, &name.names));
                if let Some(other) = other {
                    errors.push(GraphError::Duplicate {
                        kind: name.kind,
                        names: name.names.clone(),
                        first: Place {
                            path: first_path.clone(),
                            range: name.range,
                        },
                        second: Place {
                            path: second_path.clone(),
                            range: other.range,
                        },
                    });
                }
            }
        }
    }
    errors
}

/// The scripts in the order that they run, by Kahn's algorithm with the ready scripts taken in
/// the order of their indices, which is the order of their paths, and the scripts that never
/// become ready, by index.
fn order(
    dependencies: &[BTreeSet<usize>],
    dependents: &[BTreeSet<usize>],
) -> (Vec<usize>, Vec<usize>) {
    let mut waiting: Vec<usize> = dependencies.iter().map(BTreeSet::len).collect();
    let mut ready: BTreeSet<usize> = (0..waiting.len())
        .filter(|&index| waiting[index] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(script) = ready.pop_first() {
        order.push(script);
        for &dependent in &dependents[script] {
            waiting[dependent] -= 1;
            if waiting[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }
    let blocked = (0..waiting.len())
        .filter(|&index| waiting[index] > 0)
        .collect();
    (order, blocked)
}

/// The cycles of the blocked scripts, so that each script in a cycle is in one of them: for each
/// script by index that is in a cycle and in none of the cycles before, the shortest cycle from
/// it through its strongly connected component. The other blocked scripts depend on a cycle,
/// and are in none.
fn cycles(dependencies: &[BTreeSet<usize>], blocked: &[usize]) -> Vec<Vec<usize>> {
    let reach: Vec<BTreeSet<usize>> = (0..dependencies.len())
        .map(|start| {
            let mut reached = BTreeSet::new();
            let mut pending = vec![start];
            while let Some(script) = pending.pop() {
                for &dependency in &dependencies[script] {
                    if reached.insert(dependency) {
                        pending.push(dependency);
                    }
                }
            }
            reached
        })
        .collect();
    // The scripts on a cycle that has an error.
    let mut covered = BTreeSet::new();
    let mut cycles = Vec::new();
    for &start in blocked {
        if covered.contains(&start) || !reach[start].contains(&start) {
            continue;
        }
        let component: BTreeSet<usize> = (reach[start].iter().copied())
            .filter(|&other| reach[other].contains(&start))
            .collect();
        let cycle = shortest_cycle(dependencies, &component, start);
        covered.extend(cycle.iter().copied());
        cycles.push(cycle);
    }
    cycles
}

/// The shortest cycle from `start` back to `start` within `component`, by a breadth-first search
/// that takes the scripts in the order of their indices.
fn shortest_cycle(
    dependencies: &[BTreeSet<usize>],
    component: &BTreeSet<usize>,
    start: usize,
) -> Vec<usize> {
    let mut parent = vec![None; dependencies.len()];
    let mut queue = VecDeque::from([start]);
    let mut last = None;
    'search: while let Some(script) = queue.pop_front() {
        for &next in &dependencies[script] {
            if next == start {
                last = Some(script);
                break 'search;
            }
            if component.contains(&next) && parent[next].is_none() {
                parent[next] = Some(script);
                queue.push_back(next);
            }
        }
    }
    let mut cycle = Vec::new();
    let mut at = last;
    while let Some(script) = at {
        cycle.push(script);
        at = parent[script];
    }
    cycle.reverse();
    cycle
}
