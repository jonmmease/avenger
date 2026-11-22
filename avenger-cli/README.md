# avenger-cli

`avenger`, the command-line program of avenger-sql projects, for people, scripts, CI, and agents. Each command starts, reads the files, answers, and exits, so a command runs the same way in a terminal, in CI, and in an agent's loop.

```sh
avenger check              # the errors and warnings of the project of the current directory
avenger check --run charts # also run the queries of the charts under charts/
avenger fmt                # format the project's scripts and modules in place
avenger fmt --check        # list the files that need formatting, and change none
```

## Files and projects

A command takes files and directories, or the current directory without one. A path in a project, under a directory whose `avenger.toml` is the nearest one above it, selects the project's files under the path, by the roles that the manifest gives them: data scripts, chart scripts, modules, and stylesheets. A project loads once, whatever paths select it, as an editor loads it: its data scripts run, so the tables that they create have their schemas when the other files are checked.

Scripts end with `.av.sql` and modules with `.av.sqlm`. A script or a module outside every project, or without a role in its project's manifest, is a loose file: it is checked alone, as a chart script or a module, so it reads no tables of a project, and a read of a table has a help line that says so.

## Commands

`avenger check [PATH ...]` reports the diagnostics of the selected files: the errors of their syntax and of their analysis, the project's errors, such as a name that two data scripts define, a cycle of data scripts, or a read of a table whose data script would not load, the loader's errors, and the errors of the load of the entry stylesheet in each stylesheet. With `--run`, each chart script without errors also runs its query, which finds errors that appear only when the query runs, such as a cast of a value that fails.

`avenger fmt [PATH ...]` formats the selected scripts and modules in the style of `avenger-sql/STYLE.md`, by the role of each file, and prints the files that it changed. A statement with a syntax error stays as written. With `--check`, it changes no file, prints the files that formatting would change, and fails when there is one.

## Output

Results go to standard output, and the counts of errors and warnings and the errors that keep a command from running go to standard error. Text output has colors on a terminal, and `NO_COLOR` turns them off. A diagnostic prints as `path:line:column: severity: message`, with the line of the text and a marker under its range, and its help after `= help:`. Paths are relative to the current directory when they are under it. Lines and columns start at 1, and a column counts characters. An error of a whole file, such as the failure of a chart's query, has no line and column.

`--json` prints one JSON object, whose shape changes only with its `version` field. `check` prints `{"version": 1, "diagnostics": [...]}`, where each diagnostic has `path`, `range` (with `start` and `end` lines and columns and the `bytes` of the range in the file, or `null`), `severity` (`error` or `warning`), `message`, `help`, and `source` (`syntax`, `analysis`, `project`, `loader`, `style`, or `run`). `fmt` prints `{"version": 1, "formatted": [...]}`, or with `--check`, `{"version": 1, "unformatted": [...]}`.

## Exit status

| Status | When |
|---|---|
| 0 | The command succeeded: no errors, or no file that needs formatting. Warnings do not change the status. |
| 1 | `check` found errors, or `fmt --check` found a file that needs formatting. |
| 2 | The command could not run: a usage error, a path that does not exist, a file that is not a script, a module, or a stylesheet, or a project whose manifest cannot be read. |

No command prompts.
