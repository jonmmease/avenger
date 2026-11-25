//! `avenger quarto2-init` with a fake `q2` on `PATH`, a script that records its arguments: the
//! files of a new project, an existing `_quarto.yml`, `--force`, an installed extension newer than
//! avenger's, `--dry-run`, `--example`, a second run, and a missing `q2`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A folder with a fake `q2`, which appends its arguments to `q2.log` beside it.
fn fake_q2() -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("q2.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$1\" in\n  --version) echo 'q2 (quarto 2) 0.48.0-test' ;;\n  add) echo 'Extension installation complete.' ;;\nesac\n",
        log.display()
    );
    let q2 = bin.path().join("q2");
    std::fs::write(&q2, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&q2, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// The exit status and the standard output of `avenger quarto2-init` with `args` in `dir`, with
/// `bin` and the system's folders on `PATH`.
fn init(dir: &Path, bin: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .arg("quarto2-init")
        .args(args)
        .current_dir(dir)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env_remove("QUARTO_NODE")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// The lines that the fake `q2` in `bin` recorded.
fn calls(bin: &Path) -> Vec<String> {
    let log = std::fs::read_to_string(bin.join("q2.log")).unwrap_or_default();
    log.lines().map(str::to_string).collect()
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_new_folder_takes_the_extension_and_the_files_of_a_project() {
    let (bin, project) = (fake_q2(), tempfile::tempdir().unwrap());
    let dir = project.path();
    let (status, stdout, stderr) = init(dir, bin.path(), &[]);
    assert_eq!(status, 0, "{stdout}{stderr}");
    assert!(stdout.contains("extension: installed"), "{stdout}");
    assert_eq!(
        calls(bin.path()),
        ["--version", "add .quarto/avenger-extension --no-prompt"]
    );
    // The embedded extension, with avenger's version.
    let source = dir.join(".quarto/avenger-extension/_extensions/avenger");
    let manifest = read(source.join("_extension.yml"));
    let version = format!("version: {}\n", env!("CARGO_PKG_VERSION"));
    assert!(manifest.contains(&version), "{manifest}");
    assert!(manifest.contains("avenger-sql:"), "{manifest}");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("quarto2/_extensions/avenger/dist/avenger-engine.js");
    assert_eq!(read(source.join("dist/avenger-engine.js")), read(bundle));
    // The grammar of avenger-sql, which q2 highlights the cells' code with.
    assert!(
        stdout.contains("grammar: written to _quarto/grammars/avenger-sql/"),
        "{stdout}"
    );
    let vendored = Path::new(env!("CARGO_MANIFEST_DIR")).join("quarto2/grammars/avenger-sql");
    for file in ["avenger-sql.wasm", "highlights.scm"] {
        let written = std::fs::read(dir.join("_quarto/grammars/avenger-sql").join(file)).unwrap();
        assert_eq!(
            written,
            std::fs::read(vendored.join(file)).unwrap(),
            "{file}"
        );
    }
    // A Quarto 2 project, whose renders go to `_output/` and whose preview runs the engine on every
    // edit, with q2's working folder, the renders, and the figures beside each document ignored.
    let quarto_yml = read(dir.join("_quarto.yml"));
    assert!(
        quarto_yml.starts_with("quarto-required: \"^2\"\n"),
        "{quarto_yml}"
    );
    assert!(
        quarto_yml.contains(
            "project:\n  type: default\n  output-dir: _output\n  resources:\n    - \"_avenger/**\"\n"
        ),
        "{quarto_yml}"
    );
    assert!(
        quarto_yml.contains("preview:\n  engine: auto\n"),
        "{quarto_yml}"
    );
    assert_eq!(
        read(dir.join(".gitignore")),
        "/.quarto/\n/_output/\n/_avenger/\n# The figures that q2 leaves beside each document\n*_files/\n"
    );
    // A second run changes no file of the project, and takes the other forms of the ignores.
    let ignores = "target\n.quarto/\n_output/\n_avenger/\n*_files\n";
    std::fs::write(dir.join(".gitignore"), ignores).unwrap();
    let (status, stdout, _) = init(dir, bin.path(), &[]);
    assert_eq!(status, 0);
    assert!(
        stdout.contains(
            ".gitignore: ignores `/.quarto/`, `/_output/`, `/_avenger/`, and `*_files/` already"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("grammar: already current"), "{stdout}");
    assert_eq!(read(dir.join(".gitignore")), ignores);
}

#[test]
fn an_existing_quarto_yml_stays_and_the_lines_to_add_print() {
    let (bin, project) = (fake_q2(), tempfile::tempdir().unwrap());
    let dir = project.path();
    // A comment that names the runtime's folder is not its resource.
    let quarto_yml = "# _avenger/ holds the runtime\nproject:\n  type: website\n";
    std::fs::write(dir.join("_quarto.yml"), quarto_yml).unwrap();
    std::fs::write(dir.join(".gitignore"), "target").unwrap();
    let (status, stdout, _) = init(dir, bin.path(), &["--force"]);
    assert_eq!(status, 0, "{stdout}");
    assert_eq!(read(dir.join("_quarto.yml")), quarto_yml);
    assert!(
        stdout.contains("add\n  preview:\n    engine: auto"),
        "{stdout}"
    );
    assert!(
        stdout.contains("add\n  project:\n    resources:\n      - \"_avenger/**\""),
        "{stdout}"
    );
    // The ignores follow the file's own lines, which keep no final newline.
    assert_eq!(
        read(dir.join(".gitignore")),
        "target\n/.quarto/\n/_output/\n/_avenger/\n# The figures that q2 leaves beside each document\n*_files/\n"
    );
    assert!(
        stdout.contains(".gitignore: `/.quarto/`, `/_output/`, `/_avenger/`, `*_files/` added"),
        "{stdout}"
    );
    assert_eq!(
        calls(bin.path()),
        [
            "--version",
            "add .quarto/avenger-extension --no-prompt --force"
        ]
    );
}

#[test]
fn an_extension_newer_than_avenger_s_stays_unless_forced() {
    let (bin, project) = (fake_q2(), tempfile::tempdir().unwrap());
    let dir = project.path();
    std::fs::create_dir_all(dir.join("_extensions/avenger")).unwrap();
    std::fs::write(
        dir.join("_extensions/avenger/_extension.yml"),
        "title: Avenger\nversion: 99.0.0\n",
    )
    .unwrap();
    let (status, stdout, _) = init(dir, bin.path(), &[]);
    assert_eq!(status, 0, "{stdout}");
    assert!(
        stdout.contains("newer than this avenger's") && stdout.contains("upgrade avenger"),
        "{stdout}"
    );
    assert_eq!(calls(bin.path()), ["--version"]);
    assert!(
        stdout.contains("grammar: kept, with the newer extension"),
        "{stdout}"
    );
    assert!(!dir.join("_quarto").exists());
    let (_, stdout, _) = init(dir, bin.path(), &["--force"]);
    assert!(dir
        .join("_quarto/grammars/avenger-sql/avenger-sql.wasm")
        .exists());
    assert!(stdout.contains("extension: installed"), "{stdout}");
    assert!(calls(bin.path())
        .contains(&"add .quarto/avenger-extension --no-prompt --force".to_string()));
}

#[test]
fn a_dry_run_writes_nothing_and_runs_no_q2_add() {
    let (bin, project) = (fake_q2(), tempfile::tempdir().unwrap());
    let dir = project.path();
    let (status, stdout, _) = init(dir, bin.path(), &["--dry-run", "--example"]);
    assert_eq!(status, 0, "{stdout}");
    assert!(
        stdout.contains("would write .quarto/avenger-extension/"),
        "{stdout}"
    );
    assert!(stdout.contains("would: index.av.qmd: written"), "{stdout}");
    assert!(stdout.contains("would: grammar: written"), "{stdout}");
    assert_eq!(calls(bin.path()), ["--version"]);
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
}

#[test]
fn an_example_is_written_once() {
    let (bin, project) = (fake_q2(), tempfile::tempdir().unwrap());
    let dir = project.path();
    let (status, stdout, _) = init(dir, bin.path(), &["--example"]);
    assert_eq!(status, 0, "{stdout}");
    let example = read(dir.join("index.av.qmd"));
    assert!(example.contains("#| label: fig-cars"), "{example}");
    assert!(
        stdout.ends_with("preview: q2 preview index.av.qmd\n"),
        "{stdout}"
    );
    std::fs::write(dir.join("index.av.qmd"), "# Mine\n").unwrap();
    let (_, stdout, _) = init(dir, bin.path(), &["--example"]);
    assert!(stdout.contains("index.av.qmd: kept"), "{stdout}");
    assert_eq!(read(dir.join("index.av.qmd")), "# Mine\n");
}

#[test]
fn a_missing_q2_fails_with_how_to_install_it() {
    let (bin, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (status, _, stderr) = init(project.path(), bin.path(), &[]);
    assert_eq!(status, 2);
    assert!(
        stderr.contains("q2 is not on PATH: install Quarto 2"),
        "{stderr}"
    );
}
