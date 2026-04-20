mod support;

use osmshrink::fetch::{FetchOptions, download_source};
use support::TestHttpServer;

#[tokio::test]
async fn download_source_reuses_existing_output_file() {
    let server = TestHttpServer::start(b"osm pbf bytes".to_vec());
    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join("extract.osm.pbf");
    let options = FetchOptions {
        show_progress: false,
        force: false,
    };

    let first = download_source(&server.url("/extract.osm.pbf"), &output, &options)
        .await
        .unwrap();
    let second = download_source(&server.url("/extract.osm.pbf"), &output, &options)
        .await
        .unwrap();

    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(second.bytes_written, 13);
    assert_eq!(tokio::fs::read(&output).await.unwrap(), b"osm pbf bytes");
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn force_download_bypasses_existing_output_file() {
    let server = TestHttpServer::start(b"fresh bytes".to_vec());
    let temp_dir = tempfile::tempdir().unwrap();
    let output = temp_dir.path().join("extract.osm.pbf");

    download_source(
        &server.url("/extract.osm.pbf"),
        &output,
        &FetchOptions {
            show_progress: false,
            force: false,
        },
    )
    .await
    .unwrap();
    let forced = download_source(
        &server.url("/extract.osm.pbf"),
        &output,
        &FetchOptions {
            show_progress: false,
            force: true,
        },
    )
    .await
    .unwrap();

    assert!(!forced.cached);
    assert_eq!(forced.bytes_written, 11);
    assert_eq!(server.request_count(), 2);
}
