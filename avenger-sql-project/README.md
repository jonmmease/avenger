# avenger-sql-project

Projects of avenger-sql: a directory whose root holds the manifest `avenger.toml`, with data scripts that register the project's tables, chart scripts whose last queries hold charts, modules that `IMPORT MODULE` reads, and stylesheets that style every chart. The crate reads the manifest, gives each file of the project its role, loads the entry stylesheet, loads the data scripts into a DataFusion session, runs the chart scripts against it, and reloads the data scripts that change. It owns the I/O of a project, through an object store, so that a project reads the same from a directory on disk and from files that a web build embeds.

```rust
use avenger_sql_project::{FileSource, LoadedProject, ProjectFiles};
use datafusion::prelude::SessionContext;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = FileSource::embedded(
        "sales",
        [
            ("avenger.toml", "[project]\nname = \"sales\"\n"),
            ("data/sources.sql", "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['data/files/sales.csv']);"),
            ("data/files/sales.csv", "region,revenue\neast,10\n"),
            ("charts/revenue.sql", "FROM sales |> AGGREGATE mark_bar(x => region, y => revenue) AS chart"),
            ("styles/main.css", "@import 'std:base.css';"),
        ],
    )
    .await?;
    let project = ProjectFiles::open(source).await?;
    assert_eq!(project.data_scripts(), ["data/sources.sql"]);
    assert_eq!(project.chart_scripts(), ["charts/revenue.sql"]);

    let style = project.load_stylesheet().await?;
    assert!(style.sheet.is_some() && style.diagnostics.is_empty());

    let loaded = LoadedProject::load(project, SessionContext::new()).await?;
    assert!(loaded.scripts().all(|script| script.loaded().is_some()));
    let chart = loaded.files().source().read_text("charts/revenue.sql").await?;
    let rows = loaded.run_chart("charts/revenue.sql", &chart).await?;
    let rows = rows.collect().await?;
    assert_eq!(rows[0].num_rows(), 1);
    Ok(())
}
```

## The layout

The layout of a project, which is also the default of a manifest without paths:

```text
sales/
  avenger.toml
  data/                 data scripts
    sources.sql
    files/              the data that the data scripts read, by convention only
      sales.csv
  charts/               chart scripts, in folders of any depth
    revenue.sql
    regions/
      by-month.sql
  styles/
    main.css            the entry stylesheet, which imports the others
    brand.css
  modules/              modules that IMPORT MODULE reads
    stats.sqlm
```

`data/files/` is a convention, not a role: a data script can read any file of the project. The default patterns of the data scripts leave it out, so a `.sql` file among the data, such as a dump, is not a data script.

## The manifest

`Manifest::parse` reads `avenger.toml`. Every section but `[project]` and every field but `name` is optional, with these defaults, which `Manifest::new` also gives:

```toml
[project]
name = "sales"
# The version of the project format. This crate reads format 1 only.
format = 1

[data]
# The data scripts, as patterns relative to the root.
scripts = ["data/**/*.sql"]
# The patterns that no data script matches.
exclude = ["data/files/**"]

[charts]
scripts = ["charts/**/*.sql"]

[style]
# The entry stylesheet, whose @import rules load the project's other stylesheets in cascade order.
stylesheet = "styles/main.css"

[modules]
# The directory where IMPORT MODULE a.b finds a/b.sqlm.
root = "modules"
```

`Manifest::smallest` writes the text of the smallest manifest of a name, the `[project]` section with the name alone, as an editor writes it for a new project.

An unknown section or key, a value of the wrong type, a `format` other than 1, an invalid pattern, and a path that leaves the project are a `ManifestError`, with the range of the key or the value in the text and its line. A pattern is a glob of paths relative to the root, in which `*` and `?` match within a name and `**` as a whole name matches any number of directories, none included, so `data/**/*.sql` matches `data/sources.sql`. A `Pattern` keeps the range of its string, so an editor places an error of a pattern at the pattern.

## The file source

A `FileSource` is an object store and the URL of the project's root in it. `FileSource::directory` reads a directory on disk at its `file:` URL through a `LocalFileSystem` whose keys are absolute paths, and is not in a web build. `FileSource::embedded` holds files in an `InMemory` store at `memory://{name}/`, on every target, and `FileSource::new` takes any store and URL. The key of a file is the path of the root's URL and the file's path, as DataFusion reads it from the file's URL, so `register` puts the store in a `SessionContext` at the scheme and the host of the root's URL, and `avenger_sql::data::file_table` resolves the URIs of data scripts against `base`. `read`, `read_text`, and `list` read a file and list the files under a directory, by paths relative to the root with `/` between their names. A listing leaves out hidden files and directories, whose names start with `.`, such as `.git` and `.DS_Store`, and does not descend into a hidden directory, so a project inside a git checkout never walks `.git`, and a hidden file has no role.

## Roles

`ProjectFiles::open` reads the manifest at the root of a source, lists the project's files, and gives each its role by `Manifest::role`. A `.sql` file that a pattern of `[data]` matches, and no pattern of its `exclude`, is a data script, and a `.sql` file that a pattern of `[charts]` matches is a chart script. A file that both match is a `RoleConflict`, which names a pattern of each, and a project with one does not open. A `.sqlm` file under the modules root is a module, whose module path comes from its path: `modules/a/b.sqlm` is `a.b`. A `.css` file is a stylesheet. A file of no role, such as a data file, is in `files` alone. Each list is in the order of the paths.

On disk, `find_root` finds the root of the project of any path under it, the nearest directory from the path up that holds `avenger.toml`, and `ProjectFiles::find` opens that project.

## The entry stylesheet

`ProjectFiles::load_stylesheet` loads the entry stylesheet with `avenger_style::Sheet::load` and its imports through the file source. An import is a path relative to the stylesheet that holds it, of a stylesheet of the project, or a standard stylesheet such as `std:vegalite.css`. An import that does not load, that leaves the project, or that imports itself through other stylesheets is a diagnostic at the span of its URL. The source id of each stylesheet is the position of its file in `ProjectFiles::stylesheets`, and `ProjectStylesheet::path` gives the file of a diagnostic. A project without a file at the path of its entry stylesheet has no stylesheet of its own. `unreached` lists the `.css` files that the entry stylesheet does not reach, which apply to no chart, for an editor to warn about.

## Loading

`LoadedProject::load` loads a project into a `SessionContext`: a new session, or a session of the host whose tables the scripts read as they read the project's. It registers avenger-sql's `avenger_sql::register` and the store of the file source in the session, reads the texts of the data scripts and the modules, and loads the data scripts in the order of their `avenger_sql::project::Graph`, each after the scripts whose tables, views, and schemas it reads. For each data script, the loader builds the table of each file read of `file_reads` with `file_table`, analyzes the script with `analyze_data` and a `ProjectCatalog` of the session's tables, the definitions of the scripts that it reads, the tables of its files, and the project's modules, and registers its tables with `lower`. A `TEMP` table registers nothing.

`scripts` gives the `ScriptLoad` of each data script, in the order of the load, with its `ScriptStatus` and its diagnostics, each a `ProjectDiagnostic` at a range of the script's text:

- `Loaded`: the script's analysis and what it registered.
- `Failed`: the parse, the analysis, or the lowering of the script has errors, such as a URI that matches no file, which is an error at the URIs of its statement, or a query that fails to run, which is an error at its statement. A script that fails registers nothing. A name that two data scripts define, an error of the graph at the name in each, fails both, so that neither replaces the other's table.
- `NotLoaded`: the script reads a name of a script that did not load, and does not run. Its error is at its first read of a name of each such script, and names the script and the scripts upstream that failed.
- `Blocked`: the script is in a cycle of scripts that read each other's names, and has the graph's error at its read of the next script, or it reads a script of a cycle, and has an error at that read.

The scripts that do not depend on a failed script load as usual. `graph` gives the errors of the graph, which are also the diagnostics of the scripts at their places. A load fails as a whole, with a `ProjectError`, only when the session's query planner can't be replaced or a file of the project can't be read.

The `files` of a `ScriptLoad` hold the table of each set of files that the script read, or the error of reading it, and `file_tables` gives those of every script, and then those of the charts' own tables of files in their last runs, which an editor's analysis of the scripts takes, so that it reads the schemas of the files that the load read. A script that did not run read no files.

## Running a chart

`LoadedProject::run_chart` runs a chart script's text against the session, with the project's modules, as `avenger_sql::sql_with_modules` does, and gives the `DataFrame` of its last query. A chart's own tables of files, its `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE` statements, read URIs relative to the project's root, and `avenger_sql::data::ScriptTables` builds their tables, which the chart's plans read and no session registers, so no other chart and no data script sees them. The project keeps a chart's tables by the chart's path, which `run_chart` takes with the text: a run whose statements of files are those of the last run reuses them, so a run that changes only the query reads no files again, and a `LOAD DATA` reads its files once. The rows of a chart's `LOAD DATA` stay in memory while the project is open, as those of a data script do. A statement of files whose table fails to build is an error at the statement, an `avenger_sql::ScriptError`, and `reload_all` drops every chart's tables. When the run fails and the chart reads a table of a data script that did not load, or a table in the schema of one, the error is `ChartError::Unloaded`, with an `UnloadedRead` at each such read that names the data script and why it did not load. A chart can read a table of its own of the same name, such as a `TEMP` table, so the loader looks for these reads only when the run fails. `LoadedProject::lift` gives the lifted form of a chart script at its path, with the project's modules and its own tables of files, as `avenger_sql::lift_with_modules` prints it, and the text of its statements of files as written.

## Reloading

`LoadedProject::reload` loads a data script again with a new text. The scripts that reload are the script and the scripts downstream of it, in the graph before the change and after it, and the scripts that define a name that one of them also defines. What they registered before leaves the session, including the tables and the schemas that a script no longer defines, and the scripts load again in the order of the new graph. A reload returns a `TableChange` for each table that a reloaded script registered before or after, as `Registered`, `Replaced`, or `Removed`, so that a host runs again the charts that read them, which `avenger_sql::catalog::references` of a chart gives. A table that a script registers again is `Replaced`, since its rows and its files are those of the reload.

`reload_all` opens the project again from its file source, with its manifest, its files, and the texts of its data scripts and modules, and loads every data script again in the same session. The tables of the host stay.

`scripts_reading` gives the data scripts whose last loads read a file, by its path relative to the root, and then the chart scripts whose own tables of files read it in their last runs: those with a URI that names the file, or a directory or a pattern that holds it, as `avenger_sql::data::file_urls` resolves the URIs and the listing of a table matches its files. The match reads the URIs and not the store, so a removed file matches as it did, and a new file matches a directory or a pattern. `reload_files` loads the scripts that read the files at a list of paths again, after the files change on disk, with the scripts downstream of them, each once, and returns a `FilesReload`: the changes of the tables as a reload does, and the charts whose own tables read the files. Each such chart reads its files again and drops the tables that its last run built, so a host runs it again, and its next run builds them.

A `LoadedProject` is `Send` and `Sync`, and the futures of `load`, `reload`, `reload_all`, `reload_files`, `run_chart`, and `lift` are not `Send`: they hold the syntax trees of scripts, which are not `Send`, across their awaits, and `lower` and `sql_with_modules` take a catalog and the modules as `&dyn` references that are not `Sync`. A host awaits them on one thread, as a web build does.
