mod support;

use std::process::Command;

use support::TestHttpServer;

const SAARLAND_EXAMPLE_URL: &str =
    "https://download.geofabrik.de/europe/germany/saarland-latest.osm.pbf";

#[test]
fn cli_fetch_reuses_cached_output_file() {
    let server = TestHttpServer::start(b"cli pbf bytes".to_vec());
    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join("extract.osm.pbf");
    let bin = env!("CARGO_BIN_EXE_osmshrink");

    let first = Command::new(bin)
        .args([
            "--quiet",
            "fetch",
            &server.url("/extract.osm.pbf"),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "first fetch failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let second = Command::new(bin)
        .args([
            "--quiet",
            "fetch",
            &server.url("/extract.osm.pbf"),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "second fetch failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    assert_eq!(std::fs::read(&output).unwrap(), b"cli pbf bytes");
    assert_eq!(server.request_count(), 1);
}

#[test]
fn cli_fetch_reports_cached_saarland_geofabrik_url() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join("saarland.osm.pbf");
    std::fs::write(&output, b"cached saarland pbf").unwrap();
    let bin = env!("CARGO_BIN_EXE_osmshrink");

    let result = Command::new(bin)
        .args(["fetch", SAARLAND_EXAMPLE_URL, "--output"])
        .arg(&output)
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "fetch failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("Using cached 19 bytes"));
    assert!(stderr.contains(SAARLAND_EXAMPLE_URL));
    assert!(stderr.contains(output.to_str().unwrap()));
    assert_eq!(std::fs::read(&output).unwrap(), b"cached saarland pbf");
}
