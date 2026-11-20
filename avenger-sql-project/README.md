# avenger-sql-project

Projects of avenger-sql: a directory whose root holds the manifest `avenger.toml`, with data scripts that register the project's tables, chart scripts whose last queries hold charts, modules that `IMPORT MODULE` reads, and stylesheets that style every chart. The crate reads the manifest, gives each file of the project its role, and loads the entry stylesheet. It owns the I/O of a project, through an object store, so that a project reads the same from a directory on disk and from files that a web build embeds.

```rust
use avenger_sql_project::{FileSource, ProjectFiles};

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

`ProjectFiles::open` reads the manifest at the root of a source, lists the project's files, and gives each its role by `Manifest::role`, and `skipped` gives the directories that the listing skipped, for a host to show. A hidden file, one with a name on its path that starts with `.`, has no role. A `.sql` file that a pattern of `[data]` matches, and no pattern of its `exclude`, is a data script, and a `.sql` file that a pattern of `[charts]` matches is a chart script. A file that both match is a `RoleConflict`, which names a pattern of each, with its line in `avenger.toml` or as a default, and a project with one does not open. A `.sqlm` file under the modules root is a module, whose module path comes from its path: `modules/a/b.sqlm` is `a.b`. A `.css` file is a stylesheet, unless a pattern of `exclude` of `[data]` matches it, as `data/files/report.css`, which is data. A file of no role, such as a data file, is in `files` alone. Each list is in the order of the paths.

On disk, `find_root` finds the root of the project of any path under it, the nearest directory from the path up that holds `avenger.toml`, and `ProjectFiles::find` opens that project. A path that does not exist yet, such as that of a file to make, starts from its nearest directory that exists.

## The entry stylesheet

The entry stylesheet is the file at the path of `stylesheet` of `[style]`, `styles/main.css` by default. A project can lack the file of the default path, and an entry stylesheet that the manifest names and the project lacks is a `ManifestError` at its value, the error of `load_stylesheet`. The load reads the entry stylesheet and the stylesheets that it reaches through its imports, and no other, so a stylesheet of `unreached` is never read.

`ProjectFiles::load_stylesheet` loads the entry stylesheet with `avenger_style::Sheet::load` and its imports through the file source. An import is a path relative to the stylesheet that holds it, of a stylesheet of the project, or a standard stylesheet such as `std:vegalite.css`. An import that does not load, that leaves the project, or that imports itself through other stylesheets is a diagnostic at the span of its URL. The source id of each stylesheet is the position of its file in `ProjectFiles::stylesheets`, and `ProjectStylesheet::path` gives the file of a diagnostic. A project without a file at the path of its entry stylesheet has no stylesheet of its own. `unreached` lists the `.css` files that the entry stylesheet does not reach, which apply to no chart, for an editor to warn about.
