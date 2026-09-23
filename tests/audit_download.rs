#![cfg(feature = "cli")]
mod support;
use osmshrink::fetch::{FetchOptions, download_url};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use support::TestHttpServer;

#[tokio::test]
async fn truncated_http_body_preserves_previous_output_and_cleans_staging() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/extract.pbf", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\npartial",
            )
            .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("old.pbf");
    fs::write(&output, b"old complete data").unwrap();
    let result = download_url(
        &url,
        &output,
        &FetchOptions {
            force: true,
            show_progress: false,
        },
    )
    .await;
    server.join().unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read(output).unwrap(), b"old complete data");
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "error paths clean their own temporary files"
    );
}
#[tokio::test]
async fn concurrent_forced_downloads_publish_whole_files_not_mixed_bytes() {
    let a = vec![b'a'; 2 * 1024 * 1024];
    let b = vec![b'b'; a.len() + 17];
    let first = TestHttpServer::start(a.clone());
    let second = TestHttpServer::start(b.clone());
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("shared.pbf");
    let options = FetchOptions {
        force: true,
        show_progress: false,
    };
    let first_url = first.url("/a.pbf");
    let second_url = second.url("/b.pbf");
    let (one, two) = tokio::join!(
        download_url(&first_url, &output, &options),
        download_url(&second_url, &output, &options)
    );
    assert_eq!(one.unwrap().bytes_written, a.len() as u64);
    assert_eq!(two.unwrap().bytes_written, b.len() as u64);
    let actual = fs::read(&output).unwrap();
    assert!(
        actual == a || actual == b,
        "publication must contain exactly one complete response"
    );
    assert_eq!(first.request_count() + second.request_count(), 2);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[tokio::test]
async fn concurrent_nonforced_downloads_keep_the_first_complete_publication() {
    let body = vec![b'x'; 1024 * 1024];
    let server = TestHttpServer::start(body.clone());
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("shared.pbf");
    let options = FetchOptions {
        force: false,
        show_progress: false,
    };
    let url = server.url("/same.pbf");
    let (one, two) = tokio::join!(
        download_url(&url, &output, &options),
        download_url(&url, &output, &options)
    );
    let (one, two) = (one.unwrap(), two.unwrap());
    assert_ne!(
        one.cached, two.cached,
        "one publisher and one cached winner"
    );
    assert_eq!(fs::read(output).unwrap(), body);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
