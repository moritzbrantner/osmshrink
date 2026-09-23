//! Public-API contracts for destructive I/O boundaries and format interoperability.
#![cfg(feature = "cli")]
use osmshrink::{
    ConvertOptions, GeoDataset, GeoFormat, convert_path, read_dataset, read_flatgeobuf_dataset,
    write_dataset, write_flatgeobuf_dataset,
};
use std::{fs, path::Path};
use tempfile::tempdir;

fn fixture() -> GeoDataset {
    serde_json::from_str(include_str!("fixtures/neutral-station.json")).unwrap()
}
fn large_fixture() -> GeoDataset {
    let mut dataset = fixture();
    dataset.features = (0..512)
        .map(|i| {
            let mut feature = fixture().features.remove(0);
            feature.id = Some(osmshrink::GeoFeatureId::String(format!("station-{i}")));
            feature
                .properties
                .insert("padding".into(), serde_json::json!("x".repeat(1024)));
            feature
        })
        .collect();
    dataset
}
#[test]
fn explicit_formats_override_unknown_and_missing_extensions_independently() {
    for (input_name, output_name) in [
        ("input", "output"),
        ("input.data", "output.data"),
        ("input.geojson", "output.fgb"),
    ] {
        let dir = tempdir().unwrap();
        let input = dir.path().join(input_name);
        let output = dir.path().join(output_name);
        fs::write(&input, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let report = convert_path(ConvertOptions {
            input,
            output: output.clone(),
            input_format: Some(GeoFormat::Json),
            output_format: Some(GeoFormat::Ndjson),
        })
        .unwrap();
        assert_eq!(report.features_written, 1);
        assert_eq!(
            read_dataset(&output, GeoFormat::Ndjson).unwrap().features,
            fixture().features
        );
    }
}
#[test]
fn unspecified_unknown_formats_are_still_rejected_without_touching_output() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("unknown.data");
    let output = dir.path().join("output.data");
    fs::write(&input, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    fs::write(&output, b"old output").unwrap();
    for (from, to) in [(None, Some(GeoFormat::Json)), (Some(GeoFormat::Json), None)] {
        assert!(
            convert_path(ConvertOptions {
                input: input.clone(),
                output: output.clone(),
                input_format: from,
                output_format: to
            })
            .is_err()
        );
        assert_eq!(fs::read(&output).unwrap(), b"old output");
    }
}
fn assert_alias_safety(input: &Path, output: &Path, may_reject: bool) {
    let old = fs::read(input).unwrap();
    assert!(old.len() > 64 * 1024, "fixture must exceed buffering");
    let expected = read_flatgeobuf_dataset(input).unwrap().features;
    match convert_path(ConvertOptions {
        input: input.into(),
        output: output.into(),
        input_format: Some(GeoFormat::Flatgeobuf),
        output_format: Some(GeoFormat::Ndjson),
    }) {
        Ok(report) => {
            assert_eq!(report.features_written, expected.len());
            assert_eq!(
                read_dataset(output, GeoFormat::Ndjson).unwrap().features,
                expected
            );
        }
        Err(error) => {
            assert!(
                may_reject,
                "regular in-place conversion should stage safely: {error}"
            );
            assert_eq!(fs::read(input).unwrap(), old);
        }
    }
}
#[test]
fn streaming_same_path_conversion_preserves_every_feature() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("input.fgb");
    write_flatgeobuf_dataset(&path, &large_fixture(), false).unwrap();
    assert_alias_safety(&path, &path, false);
}
#[test]
fn streaming_hardlink_alias_preserves_original_inode_and_all_output() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input.fgb");
    let output = dir.path().join("alias.ndjson");
    write_flatgeobuf_dataset(&input, &large_fixture(), false).unwrap();
    let old = fs::read(&input).unwrap();
    fs::hard_link(&input, &output).unwrap();
    assert_alias_safety(&input, &output, false);
    assert_eq!(fs::read(input).unwrap(), old);
}
#[test]
#[cfg(unix)]
fn symlink_alias_is_rejected_without_following_and_truncating_it() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input.fgb");
    let output = dir.path().join("alias.ndjson");
    write_flatgeobuf_dataset(&input, &large_fixture(), false).unwrap();
    std::os::unix::fs::symlink(&input, &output).unwrap();
    assert_alias_safety(&input, &output, true);
    assert!(
        fs::symlink_metadata(output)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
#[test]
fn truncated_stream_does_not_publish_partial_output() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("broken.fgb");
    let output = dir.path().join("keep.ndjson");
    write_flatgeobuf_dataset(&input, &large_fixture(), false).unwrap();
    let mut bytes = fs::read(&input).unwrap();
    bytes.truncate(bytes.len() - 100);
    fs::write(&input, bytes).unwrap();
    fs::write(&output, b"old complete output").unwrap();
    assert!(osmshrink::stream_flatgeobuf_to_ndjson(&input, &output).is_err());
    assert_eq!(fs::read(output).unwrap(), b"old complete output");
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no abandoned staging files"
    );
}
#[test]
fn projected_geojson_is_rejected_before_overwrite_but_neutral_json_remains_supported() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("keep.geojson");
    let mut dataset = fixture();
    dataset.crs = Some("EPSG:3857".into());
    fs::write(&path, b"old").unwrap();
    let error = write_dataset(&path, GeoFormat::Geojson, &dataset).unwrap_err();
    assert!(error.to_string().contains("reprojected"));
    assert_eq!(fs::read(&path).unwrap(), b"old");
    write_dataset(&path, GeoFormat::Json, &dataset).unwrap();
    assert_eq!(read_dataset(&path, GeoFormat::Json).unwrap(), dataset);
}
#[test]
fn geographic_crs_and_neutral_feature_types_roundtrip_across_all_adapters() {
    let dir = tempdir().unwrap();
    let source = fixture();
    for format in [
        GeoFormat::Json,
        GeoFormat::Ndjson,
        GeoFormat::Geojson,
        GeoFormat::Flatgeobuf,
    ] {
        let path = dir.path().join(format.as_str());
        write_dataset(&path, format, &source).unwrap();
        let restored = read_dataset(&path, format).unwrap();
        assert_eq!(restored.features, source.features, "{format}");
    }
}
#[test]
fn cli_format_overrides_work_end_to_end() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input");
    let output = dir.path().join("output.data");
    fs::write(&input, serde_json::to_vec(&fixture()).unwrap()).unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_osmshrink"))
        .args([
            "convert",
            input.to_str().unwrap(),
            "--from",
            "json",
            "--to",
            "ndjson",
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        read_dataset(&output, GeoFormat::Ndjson).unwrap().features,
        fixture().features
    );
}
