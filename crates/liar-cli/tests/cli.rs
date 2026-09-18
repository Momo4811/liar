//! End-to-end tests for the `liar` binary.

use assert_cmd::Command;
use std::fs;

fn liar() -> Command {
    Command::cargo_bin("liar").expect("binary should build")
}

#[test]
fn a_clean_file_exits_zero_and_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clean.py");
    fs::write(&file, "def f():\n    pass\n").unwrap();

    liar().arg("check").arg(&file).assert().success().stdout("");
}

#[test]
fn a_directory_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    fs::write(dir.path().join("b.py"), "y = 2\n").unwrap();

    liar().arg("check").arg(dir.path()).assert().success();
}

#[test]
fn a_syntax_error_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("broken.py");
    fs::write(&file, "def (:\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicates::str::contains("broken.py"));
}

#[test]
fn a_missing_path_exits_two_with_a_message() {
    liar()
        .arg("check")
        .arg("definitely-not-here.py")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("definitely-not-here.py"));
}

#[test]
fn an_invalid_tone_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    fs::write(&file, "pass\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .args(["--tone", "sarcastic"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("sarcastic"));
}

#[test]
fn a_valid_tone_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    fs::write(&file, "pass\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .args(["--tone", "brutal"])
        .assert()
        .success();
}

#[test]
fn an_invalid_config_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    fs::write(&file, "pass\n").unwrap();
    let config = dir.path().join("liar.toml");
    fs::write(&config, "toen = \"dry\"\n").unwrap();

    liar()
        .arg("check")
        .arg(&file)
        .args(["--config", config.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("toen"));
}

#[test]
fn a_config_exclude_is_honoured() {
    // The excluded file does not parse. If it were analysed the run would
    // exit 2, so a clean exit proves the exclusion took effect.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("good.py"), "x = 1\n").unwrap();
    fs::create_dir_all(dir.path().join("vendor")).unwrap();
    fs::write(dir.path().join("vendor/bad.py"), "def (:\n").unwrap();

    let config = dir.path().join("liar.toml");
    fs::write(&config, "exclude = [\"vendor/**\"]\n").unwrap();

    liar()
        .arg("check")
        .arg(dir.path())
        .args(["--config", config.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn check_requires_at_least_one_path() {
    liar().arg("check").assert().failure();
}

#[test]
fn the_version_flag_works() {
    liar().arg("--version").assert().success();
}
