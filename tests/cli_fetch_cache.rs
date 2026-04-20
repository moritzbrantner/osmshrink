mod support;

use std::process::Command;

use support::TestHttpServer;

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
