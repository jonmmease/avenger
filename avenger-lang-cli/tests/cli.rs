use std::{fs, process::Command};

fn avenger() -> Command {
    Command::new(env!("CARGO_BIN_EXE_avenger"))
}

#[test]
fn cli_help_documents_only_the_watch_surface() {
    let output = avenger().arg("--help").output().expect("run avenger help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("watch"));
    for deferred in ["render", "check", "fmt", "inspect", "serve"] {
        assert!(
            !stdout
                .lines()
                .any(|line| line.trim_start().starts_with(deferred)),
            "deferred command leaked into initial help: {deferred}\n{stdout}"
        );
    }
}

#[test]
fn initial_compile_failure_prints_one_stdout_batch_and_exits_one() {
    let temporary = tempfile::tempdir().expect("create invalid chart project");
    let chart = temporary.path().join("chart.avenger");
    fs::write(&chart, "avenger 1; chart cartesian as broken {")
        .expect("write invalid chart source");

    let output = avenger()
        .arg("watch")
        .arg(&chart)
        .output()
        .expect("run invalid chart");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).expect("diagnostics are UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("error summary is UTF-8");
    assert_eq!(stdout.matches("compilation failed").count(), 1, "{stdout}");
    assert!(stdout.contains("chart.avenger"), "{stdout}");
    assert!(stdout.contains("chart cartesian as broken"), "{stdout}");
    assert!(
        !stdout.contains(&temporary.path().to_string_lossy().into_owned()),
        "diagnostics must use project-relative paths: {stdout}"
    );
    assert_eq!(
        stderr
            .matches("avenger: initial chart compilation failed")
            .count(),
        1,
        "{stderr}"
    );
}

#[test]
fn invalid_chart_argument_exits_one_without_compiler_diagnostics() {
    let output = avenger()
        .args(["watch", "chart.txt"])
        .output()
        .expect("run invalid chart argument");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("argument error is UTF-8");
    assert!(stderr.contains("must end in .avenger"), "{stderr}");
}

/// Run manually on a machine with a desktop session to exercise creation of
/// the persistent native window. Close the window to let the test finish.
#[test]
#[ignore = "requires a human-visible desktop session and manual window close"]
fn headful_watch_smoke() {
    let chart = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/watch_project/chart.avenger");
    let status = avenger()
        .arg("watch")
        .arg(chart)
        .status()
        .expect("launch native watch window");
    assert!(status.success());
}
