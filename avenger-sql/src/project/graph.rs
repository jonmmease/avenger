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
//! dependency either, as [`table_reads`] leaves it out. A table that a function of an imported
//! module reads, directly or through the modules that it imports, is a read of the script at the
//! path of its `IMPORT MODULE`, since the script reads the table where it calls the function. A
//! path of a table of one name is in the session's default schema, so a read of `public.orders`
//! depends on the script that defines `orders` when the default schema is `public`.
//!
//! The graph also holds the modules that each script imports, through its `IMPORT MODULE`s and
//! the imports of those modules in turn, so that a change of a module reloads the scripts that
//! import it, which [`Graph::importers`] gives. A module is not a script of the graph: it has
//! no place in the order, and its imports add no dependency between scripts.

use std::collections::{BTreeSet, VecDeque};

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};

use crate::{
    analysis::script_targets,
    catalog::{imported_modules, references, same_path, same_table, Reference, References},
};

/// What a name of a data script names: a table or a view, or a schema. A table and a schema of
/// the same name are different names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    Table,
    Schema,
}

impl NameKind {
    /// Whether the paths `a` and `b` of names of this kind name the same table or schema, without
    /// regard to case, with a path of a table of one name in the schema `default_schema`, as
    /// [`same_table`] compares tables.
    pub fn same(self, a: &[String], b: &[String], default_schema: &str) -> bool {
        match self {
            NameKind::Table => same_table(a, b, default_schema),
            NameKind::Schema => same_path(a, b),
        }
    }
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
    /// that it defines. In a [`Graph`], also the tables that the functions of its imported
    /// modules read, at the path of the `IMPORT MODULE`.
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
    for reference in table_reads(root) {
        let later = (statements.iter().zip(&targets)).any(|(statement, defined)| {
            statement.syntax().text_range().start() > reference.range.start()
                && defined.as_ref().is_some_and(|defined| {
                    !defined.target.schema && same_path(&defined.target.names, &reference.names)
                })
        });
        if later {
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

/// The reads of tables of `root`, a script, as [`references`] gives them, without the reads of a
/// name of a pipe `WITH`, after the `WITH` in the query that holds it, which read the `WITH`
/// query and not a table. Names match without regard to case, as in GoogleSQL.
pub fn table_reads(root: &SyntaxNode) -> Vec<Reference> {
    let scopes = with_scopes(root);
    // The paths of the tables in `FROM` and `JOIN`, which the names of a pipe `WITH` can name.
    let sources: Vec<TextRange> = (root.descendants())
        .filter_map(ast::TableSource::cast)
        .filter_map(|source| Some(source.path()?.range()))
        .collect();
    let mut reads = references(root).tables;
    reads.retain(|reference| match reference.names.as_slice() {
        [name] => {
            !sources.contains(&reference.range)
                || !(scopes.iter()).any(|(with, scope)| {
                    scope.contains_range(reference.range) && with.eq_ignore_ascii_case(name)
                })
        }
        _ => true,
    });
    reads
}

/// The paths of the tables that `root` reads in `FROM`, joins, and `TABLE` arguments, in the
/// order of the text, as a reader of the text finds them: with the reads of a table that an
/// earlier statement of a script defines, which [`references`] leaves out, and without the reads
/// of a name of a pipe `WITH`, after the `WITH` in the query that holds it. An `IMPORT MODULE`
/// and a call of a function are not reads of tables.
pub fn table_paths(root: &SyntaxNode) -> Vec<Reference> {
    let scopes = with_scopes(root);
    let mut paths = Vec::new();
    for node in root.descendants() {
        let (path, source) = if let Some(source) = ast::TableSource::cast(node.clone()) {
            (source.path(), true)
        } else if let Some(arg) = ast::TableArg::cast(node.clone()) {
            (arg.path(), false)
        } else if let Some(arg) = ast::NamedTableArg::cast(node.clone()) {
            (arg.path(), false)
        } else {
            continue;
        };
        let Some((path, names)) = path.and_then(|path| Some((path.range(), path.names()?))) else {
            continue;
        };
        let with = match names.as_slice() {
            [name] => {
                source
                    && (scopes.iter()).any(|(with, scope)| {
                        scope.contains_range(path) && with.eq_ignore_ascii_case(name)
                    })
            }
            _ => false,
        };
        if !with {
            paths.push(Reference { names, range: path });
        }
    }
    paths
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
            } => duplicate_message(*kind, names, &[&first.path, &second.path]),
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

/// The error of the name `names` of the kind `kind` that each of the data scripts `scripts`, two
/// or more, defines, by path.
pub fn duplicate_message(kind: NameKind, names: &[String], scripts: &[&str]) -> String {
    let name = names.join(".");
    let scripts = match scripts {
        [first, second] => format!("both {first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
        [] => String::new(),
    };
    match kind {
        NameKind::Table => format!(
            "Table {name} is defined by {scripts}: a table of a project is defined by one data script, or is TEMP in each"
        ),
        NameKind::Schema => format!(
            "Schema {name}, which BigQuery calls a dataset, is created by {scripts}: a schema of a project is created by one data script"
        ),
    }
}

/// Why the tables of a data script are unavailable to the scripts that read them, as a loader
/// and an editor's analysis of a project tell it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unloaded {
    /// The script has errors, or defines a name that another data script defines too.
    Failed,
    /// The script is in a cycle of data scripts that read each other's tables, or reads one.
    Blocked,
    /// The script reads, directly or through others, scripts that failed or are blocked: those
    /// scripts, by path, each with whether it is blocked.
    NotLoaded { causes: Vec<(String, bool)> },
}

impl Unloaded {
    /// The clause that ends the error of a read, as `which failed to load`, which
    /// [`unavailable_read`] takes.
    pub fn why(&self) -> String {
        match self {
            Unloaded::Failed => "which failed to load".to_string(),
            Unloaded::Blocked => {
                "which can't run, since it is in a cycle of data scripts that read each other's tables, or reads one".to_string()
            }
            Unloaded::NotLoaded { causes } => {
                let causes: Vec<String> = (causes.iter())
                    .map(|(cause, blocked)| match blocked {
                        true => format!("{cause} is in a cycle"),
                        false => format!("{cause} failed to load"),
                    })
                    .collect();
                format!("which did not load, since {}", causes.join(" and "))
            }
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
///
/// [`Graph::imports`] gives the modules that a script imports, and [`Graph::importers`] the
/// scripts that import a module.
#[derive(Clone, Debug)]
pub struct Graph {
    /// The paths of the scripts, sorted, and the names of each.
    scripts: Vec<(String, ScriptNames)>,
    /// For each script, the paths of the modules that it imports, directly or through other
    /// modules.
    imports: Vec<Vec<Vec<String>>>,
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
    /// parse by [`avenger_sql_syntax::parse_data`]. The paths are distinct. `modules` gives the
    /// text of the module at a path, or `None` when the project has none, as
    /// [`imported_modules`] reads the modules that the scripts import. A path of a table of one
    /// name is in the schema `default_schema`: the default schema of the session that the
    /// scripts load into, or DataFusion's, `public`, without one.
    pub fn new<P: Into<String>>(
        scripts: impl IntoIterator<Item = (P, SyntaxNode)>,
        modules: &dyn Fn(&[String]) -> Option<String>,
        default_schema: &str,
    ) -> Graph {
        let mut roots: Vec<(String, SyntaxNode)> = (scripts.into_iter())
            .map(|(path, root)| (path.into(), root))
            .collect();
        roots.sort_by(|(a, _), (b, _)| a.cmp(b));
        let imports: Vec<Vec<Vec<String>>> = (roots.iter())
            .map(|(_, root)| {
                (imported_modules(&references(root), modules).into_iter())
                    .map(|(names, _)| names)
                    .collect()
            })
            .collect();
        let scripts: Vec<(String, ScriptNames)> = (roots.into_iter())
            .map(|(path, root)| {
                let mut names = script_names(&root);
                names.reads.extend(module_reads(&root, modules));
                names.reads.sort_by_key(|read| read.range.start());
                (path, names)
            })
            .collect();
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
                        if definition.kind == read.kind
                            && (read.kind).same(&definition.names, &read.names, default_schema)
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
        let mut errors = duplicates(&scripts, default_schema);
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
            imports,
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

    /// The paths of the modules that the script at `path` imports, through its `IMPORT MODULE`s
    /// and the imports of those modules in turn, each once, of the modules that the project has.
    pub fn imports(&self, path: &str) -> &[Vec<String>] {
        match self.index(path) {
            Some(index) => &self.imports[index],
            None => &[],
        }
    }

    /// The scripts that import the module at the path `module`, directly or through other
    /// modules, which a change of the module reloads, by path. Module paths match without regard
    /// to case.
    pub fn importers(&self, module: &[String]) -> Vec<&str> {
        (self.imports.iter().enumerate())
            .filter(|(_, imports)| (imports.iter()).any(|names| same_path(names, module)))
            .map(|(index, _)| self.path(index))
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

/// The tables that the functions of the modules that the script `root` imports read, directly or
/// through the modules that those import, each once, at the path of the script's first `IMPORT
/// MODULE` that imports it.
fn module_reads(root: &SyntaxNode, modules: &dyn Fn(&[String]) -> Option<String>) -> Vec<Name> {
    let mut reads: Vec<Name> = Vec::new();
    for import in references(root).modules {
        let imported = References {
            tables: Vec::new(),
            modules: vec![import.clone()],
        };
        for (_, text) in imported_modules(&imported, modules) {
            let module = avenger_sql_syntax::parse_module(&text).syntax();
            for read in table_reads(&module) {
                if !(reads.iter()).any(|found| same_path(&found.names, &read.names)) {
                    reads.push(Name {
                        names: read.names,
                        kind: NameKind::Table,
                        range: import.range,
                    });
                }
            }
        }
    }
    reads
}

/// The errors of the names that two scripts define, by pair of scripts in the order of their
/// paths, with a path of a table of one name in the schema `default_schema`.
fn duplicates(scripts: &[(String, ScriptNames)], default_schema: &str) -> Vec<GraphError> {
    let mut errors = Vec::new();
    for (index, (first_path, first)) in scripts.iter().enumerate() {
        for (second_path, second) in &scripts[index + 1..] {
            for name in &first.definitions {
                let other = (second.definitions.iter()).find(|other| {
                    other.kind == name.kind
                        && (name.kind).same(&other.names, &name.names, default_schema)
                });
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
