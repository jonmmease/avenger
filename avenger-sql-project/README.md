# avenger-sql-project

Projects of avenger-sql: a directory whose root holds the manifest `avenger.toml`, with data scripts that register the project's tables, chart scripts whose last queries hold charts, modules that `IMPORT MODULE` reads, and stylesheets that style every chart. The crate reads the manifest, gives each file of the project its role, loads the entry stylesheet, loads the data scripts into a DataFusion session, runs the chart scripts against it, and reloads the data scripts and the modules that change. It owns the I/O of a project, through an object store, so that a project reads the same from a directory on disk and from files that a web build embeds.

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

An unknown section or key, a value of the wrong type, an empty `name`, a `format` other than 1, an invalid pattern, and a path or a pattern that is not relative to the root are a `ManifestError`, with the range of the key or the value in the text and its line. A path or a pattern is relative to the root when it does not start with `/` and has no `..` name and no `\`, and it reads without its `.` names and its empty names, so `./charts//*.sql` is `charts/*.sql`, and a pattern that names no file, such as `""`, is an error. A pattern is a glob of paths relative to the root, in which `*` and `?` match within a name and `**` as a whole name matches any number of directories, none included, so `data/**/*.sql` matches `data/sources.sql`. A pattern matches with case, so `charts/*.sql` does not match `Charts/a.sql`, and `*`, `?`, and `**` match no name that starts with `.`. A `Pattern` keeps the range of its string and its line, so an editor places an error of a pattern at the pattern, and so does `StyleSection` for the entry stylesheet that the manifest names.

## The file source

A `FileSource` is an object store and the URL of the project's root in it. `FileSource::directory` reads a directory on disk at its `file:` URL through a `LocalFileSystem` whose keys are absolute paths, and is not in a web build. `FileSource::embedded` holds files in an `InMemory` store at `memory://{name}/`, on every target, and `FileSource::new` takes any store and URL. The key of a file is the path of the root's URL and the file's path, as DataFusion reads it from the file's URL, so `register` puts the store in a `SessionContext` at the scheme and the host of the root's URL, and `avenger_sql::data::file_table` resolves the URIs of data scripts against `base`. `read`, `read_text`, and `list` read a file and list the files under a directory, by paths relative to the root with `/` between their names, which follow the manifest's rules of paths. A listing leaves out hidden files and directories, whose names start with `.`, such as `.git` and `.DS_Store`, and does not descend into a hidden directory, so a project inside a git checkout never walks `.git`. A `Listing` holds the paths of the files and the directories that it skipped: a directory under the listed one that the store fails to list, such as one that can't be read, is a `Skipped` with its path and the store's error, and the listing goes on without its files, while a failure to list the listed directory itself is the error. A `LocalFileSystem` leaves out a file whose name ends with `#` and digits, such as `data#1`, and a link that leads nowhere, and a name that a store's path can't hold, such as macOS's `Icon\r` with its carriage return, fails the listing of its directory. In a source of a directory on disk, a directory whose canonical path the listing already reads is skipped, with the error that it links to a folder that the listing already reads, so a link to a directory above it, such as `charts/up` to `..`, or to a directory of the project at another path, adds no files, and the listing reads each directory once, at its shortest path. A link to a directory outside the project is followed.

## Roles

`ProjectFiles::open` reads the manifest at the root of a source, lists the project's files, and gives each its role by `Manifest::role`, and `skipped` gives the directories that the listing skipped, for a host to show. A hidden file, one with a name on its path that starts with `.`, has no role. A `.sql` file that a pattern of `[data]` matches, and no pattern of its `exclude`, is a data script, and a `.sql` file that a pattern of `[charts]` matches is a chart script. A file that both match is a `RoleConflict`, which names a pattern of each, with its line in `avenger.toml` or as a default, and a project with one does not open. A `.sqlm` file under the modules root is a module, whose module path comes from its path: `modules/a/b.sqlm` is `a.b`. A module path ignores case, as `IMPORT MODULE` does, so two modules whose paths differ only in case, such as `modules/Geo.sqlm` and `modules/geo.sqlm`, are a `ModuleConflict`, and a project with one does not open. A `.css` file is a stylesheet, unless a pattern of `exclude` of `[data]` matches it, as `data/files/report.css`, which is data. A file of no role, such as a data file, is in `files` alone. Each list is in the order of the paths.

On disk, `find_root` finds the root of the project of any path under it, the nearest directory from the path up that holds `avenger.toml`, and `ProjectFiles::find` opens that project. A path that does not exist yet, such as that of a file to make, starts from its nearest directory that exists.

## The entry stylesheet

`ProjectFiles::load_stylesheet` loads the entry stylesheet, the file at the path of `stylesheet` of `[style]`, with `avenger_style::Sheet::load`, and its imports through the file source. An import is a path relative to the stylesheet that holds it, of a stylesheet of the project, or a standard stylesheet such as `std:vegalite.css`. An import that does not load, that leaves the project, or that imports itself through other stylesheets is a diagnostic at the span of its URL. The source id of each stylesheet is the position of its file in `ProjectFiles::stylesheets`, and `ProjectStylesheet::path` gives the file of a diagnostic. The load reads the entry stylesheet and the stylesheets that it reaches through its imports, and no other. A project without a file at the default path of the entry stylesheet, `styles/main.css`, has no stylesheet of its own, and an entry stylesheet that the manifest names and the project lacks is a `ManifestError` at its value, the error of `load_stylesheet`. `unreached` lists the stylesheets that the entry stylesheet does not reach, which apply to no chart and which the load does not read, for an editor to warn about.

## Loading

`LoadedProject::load` loads a project into a `SessionContext`: a new session, or a session of the host whose tables the scripts read as they read the project's. It registers avenger-sql's `avenger_sql::register` and the store of the file source in the session, reads the texts of the data scripts and the modules, and loads the data scripts in the order of their `avenger_sql::project::Graph`, each after the scripts whose tables, views, and schemas it reads. For each data script, the loader builds the table of each file read of `file_reads` with `file_table`, analyzes the script with `analyze_data` and a `ProjectCatalog` of the session's tables, the definitions of the scripts that it reads, the tables of its files, and the modules that it imports, directly or through other modules, and registers its tables with `lower`. A table of a script replaces a table of the session of the same name without regard to case, such as a table of the host, and when the script's table leaves the session, at a reload, the host's table comes back. A path of one name is a table of the session's default schema, so `orders` and `public.orders` are one table when the default schema is `public`. A `TEMP` table registers nothing.

`scripts` gives the `ScriptLoad` of each data script, in the order of the load, with its `ScriptStatus` and its diagnostics, each a `ProjectDiagnostic` at a range of the script's text:

- `Loaded`: the script's analysis and what it registered.
- `Failed`: the parse, the analysis, or the lowering of the script has errors, such as a URI that matches no file, which is an error at the URIs of its statement, or a query that fails to run, which is an error at its statement. A script that fails registers nothing. A name that two data scripts define, an error of the graph at the name in each, fails both, so that neither replaces the other's table.
- `NotLoaded`: the script reads a name of a script that did not load, and does not run. Its error is at its first read of a name of each such script, and names the script and the scripts upstream that failed.
- `Blocked`: the script is in a cycle of scripts that read each other's names, and has the graph's error at its read of the next script, or it reads a script of a cycle, and has an error at that read.

The scripts that do not depend on a failed script load as usual. `graph` gives the errors of the graph, which are also the diagnostics of the scripts at their places. A load fails as a whole, with a `ProjectError`, only when the session's query planner can't be replaced or a file of the project can't be read.

The `files` of a `ScriptLoad` hold the table of each set of files that the script read, or the error of reading it, and `file_tables` gives those of every script, which an editor's analysis of the scripts takes, so that it reads the schemas of the files that the load read. A script that did not run read no files.

## Running a chart

`LoadedProject::run_chart` runs a chart script's text against the session: it analyzes the chart with a catalog of the session's tables that it reads, its own tables of files, and the project's modules that it imports, and gives the `DataFrame` of its last query. A chart's own tables of files, its `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE` statements, read URIs relative to the project's root, and `avenger_sql::data::ScriptTables` builds their tables, which the chart's plans read and no session registers, so no other chart and no data script sees them. The project keeps a chart's tables by the chart's path, which `run_chart` takes with the text: a run whose statements of files are those of the last run reuses them, so a run that changes only the query reads no files again, and a `LOAD DATA` reads its files once. The rows of a chart's `LOAD DATA` stay in memory while the project is open, as those of a data script do. A statement of files whose table fails to build is an error at the statement, an `avenger_sql::ScriptError`. A read of files that fails, such as of a file that does not exist yet, is not kept, so the next run reads the files again. `reload_all` drops every chart's tables. When the run fails and the chart reads a table of a data script that did not load, or a table in the schema of one, the error is `ChartError::Unloaded`, with an `UnloadedRead` at each such read that names the data script and why it did not load, and the error of the run, which can hold other errors of the chart: its message is the error of each read, and then the run's. A chart can read a table of its own of the same name, such as a `TEMP` table, so the loader looks for these reads only when the run fails, and a read of a name of a pipe `WITH` reads the `WITH` query, and is not such a read. `LoadedProject::lift` gives the lifted form of a chart script at its path, with the project's modules and its own tables of files, as `run_chart` analyzes it, and the text of its statements of files as written.

## Reloading

`LoadedProject::reload` loads a data script or a module again with a new text. The scripts that reload are the script, or the scripts that import the module, directly or through other modules, and the scripts downstream of them, in the graph before the change and after it, and the scripts that define a name that one of them also defines. What they registered before leaves the session, including the tables and the schemas that a script no longer defines, and the scripts load again in the order of the new graph. A reload returns a `TableChange` for each table that a reloaded script registered before or after, as `Registered`, `Replaced`, or `Removed`, so that a host runs again the charts that read them, which `avenger_sql::catalog::references` of a chart gives. A table that a script registers again is `Replaced`, since its rows and its files are those of the reload.

`reload_all` opens the project again from its file source, with its manifest, its files, and the texts of its data scripts and modules, and loads every data script again in the same session. The tables of the host stay, as a reload gives back those that the scripts' tables replaced.

`scripts_reading` gives the scripts that read a file or a directory, by its path relative to the root, as a `FileReaders`: the data scripts whose last loads read it, in load order, and the chart scripts whose own tables of files read it in their last runs. A script reads a file when a URI names the file, or a directory or a pattern that holds it, as `avenger_sql::data::file_urls` resolves the URIs and the listing of a table matches its files, and it reads a directory that holds such a file, directory, or pattern, or a directory under the prefix of a Hive layout, since a file watcher reports a moved or removed directory as one change. The match reads the URIs and not the store, so a removed file matches as it did, and a new file matches a directory or a pattern. It compares the spellings of paths, so a URI that names a file with other cases of its letters, or through a symbolic link, does not match the file's path. `reload_files` loads the scripts that read the files or directories at a list of paths again, after they change on disk, with the scripts downstream of them, each once, and returns a `FilesReload`: the changes of the tables as a reload does, and the charts whose own tables read them. Each such chart drops the tables of files that its last run read and built, so a host runs it again, and its next run builds them.

A `LoadedProject` is `Send` and `Sync`, and the futures of `load`, `reload`, `reload_all`, `reload_files`, `run_chart`, and `lift` are not `Send`: they hold the syntax trees of scripts, which are not `Send`, across their awaits, and `lower` takes a catalog as a `&dyn` reference that is not `Sync`. A host awaits them on one thread, as a web build does.
