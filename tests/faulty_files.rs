use std::process::{Command, Output};

fn run_osmshrink(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_osmshrink"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_failed(output: &Output, context: &str) -> String {
    assert!(
        !output.status.success(),
        "{context} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_validate_spec_reports_malformed_yaml_file() {
    let file = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
    std::fs::write(file.path(), "filter:\n  types: [node\n").unwrap();

    let output = run_osmshrink(&[
        "--quiet",
        "validate-spec",
        "--spec",
        file.path().to_str().unwrap(),
    ]);

    let stderr = assert_failed(&output, "malformed YAML spec validation");
    assert!(stderr.contains("unable to parse"));
    assert!(stderr.contains("as YAML"));
    assert!(stderr.contains(file.path().to_str().unwrap()));
}

#[test]
fn cli_validate_spec_reports_missing_spec_file() {
    let output = run_osmshrink(&[
        "--quiet",
        "validate-spec",
        "--spec",
        "missing-filter-spec.yaml",
    ]);

    let stderr = assert_failed(&output, "missing spec validation");
    assert!(stderr.contains("unable to read"));
    assert!(stderr.contains("missing-filter-spec.yaml"));
}

#[test]
fn cli_filter_rejects_unsupported_input_extension() {
    let temp_dir = tempfile::tempdir().unwrap();
    let input = temp_dir.path().join("extract.txt");
    let output_path = temp_dir.path().join("schools.ndjson");
    std::fs::write(&input, b"not pbf").unwrap();

    let output = run_osmshrink(&[
        "--quiet",
        "filter",
        "amenity=school",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);

    let stderr = assert_failed(&output, "filter with unsupported input extension");
    assert!(stderr.contains("unsupported input file"));
    assert!(stderr.contains("expected an .osm.pbf file"));
    assert!(!output_path.exists());
}

#[test]
fn cli_filter_reports_corrupt_pbf_file() {
    let input = tempfile::NamedTempFile::with_suffix(".osm.pbf").unwrap();
    let output_path = tempfile::NamedTempFile::with_suffix(".ndjson").unwrap();
    std::fs::write(input.path(), b"not a valid osm pbf").unwrap();

    let output = run_osmshrink(&[
        "--quiet",
        "filter",
        "amenity=school",
        "--input",
        input.path().to_str().unwrap(),
        "--output",
        output_path.path().to_str().unwrap(),
    ]);

    let stderr = assert_failed(&output, "filter with corrupt PBF");
    assert!(stderr.contains("OSM PBF parsing failed"));
    assert!(stderr.contains(input.path().to_str().unwrap()));
}
