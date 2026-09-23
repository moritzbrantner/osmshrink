use super::*;
use serde_json::json;
use std::io;

fn point() -> GeoFeature {
    serde_json::from_value(json!({"id":"station-a", "properties":{"count":3,"active":true,"null":null,"nested":{"a":[1,false]}},
        "geometry":{"type":"Point","coordinates":[8.7,48.9]}, "metadata":{}})).unwrap()
}
struct FailingDestination;
impl Write for FailingDestination {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("injected final buffer failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn final_small_buffer_write_failure_is_not_reported_as_success() {
    let feature = point();
    let schema = PropertySchema::infer([&feature]);
    let path = Path::new("buffered.fgb");
    let mut writer = create_writer(path, &schema, None, "{}").unwrap();
    add_feature(path, &mut writer, &schema, &feature).unwrap();
    let error = write_buffered(path, writer, FailingDestination).unwrap_err();
    assert!(
        error.to_string().contains("injected final buffer failure"),
        "{error}"
    );
}
#[test]
fn a_flush_only_failure_is_propagated() {
    struct FlushFailure(Vec<u8>);
    impl Write for FlushFailure {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("injected flush failure"))
        }
    }
    let schema = PropertySchema::infer([&point()]);
    let path = Path::new("flush.fgb");
    let writer = create_writer(path, &schema, None, "{}").unwrap();
    assert!(
        write_buffered(path, writer, FlushFailure(Vec::new()))
            .unwrap_err()
            .to_string()
            .contains("injected flush failure")
    );
}
#[test]
fn extra_dimensions_are_rejected_before_overwriting_any_output() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("keep.fgb");
    for geometry in [
        json!({"type":"Point","coordinates":[8.7,48.9,123.4]}),
        json!({"type":"LineString","coordinates":[[8.7,48.9],[8.8,49.0,100.0]]}),
        json!({"type":"MultiPolygon","coordinates":[[[[0.,0.,1.],[1.,0.,1.],[0.,1.,1.],[0.,0.,1.]]]]}),
        json!({"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[0.,0.,1.]}]}),
    ] {
        std::fs::write(&output, b"previous complete dataset").unwrap();
        let mut feature = point();
        feature.geometry = Some(serde_json::from_value(geometry).unwrap());
        let dataset = GeoDataset {
            features: vec![feature],
            ..Default::default()
        };
        let error = write_flatgeobuf_dataset(&output, &dataset, false).unwrap_err();
        assert!(error.to_string().contains("XY"), "{error}");
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"previous complete dataset"
        );
    }
}
#[test]
fn external_z_header_is_rejected_by_both_read_paths() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("external-3d.fgb");
    let output = dir.path().join("keep.ndjson");
    let writer = FgbWriter::create_with_options(
        "3d",
        GeometryType::Unknown,
        FgbWriterOptions {
            has_z: true,
            write_index: false,
            ..Default::default()
        },
    )
    .unwrap();
    writer.write(File::create(&input).unwrap()).unwrap();
    std::fs::write(&output, b"old").unwrap();
    assert!(
        read_flatgeobuf_dataset(&input)
            .unwrap_err()
            .to_string()
            .contains("XY")
    );
    assert!(
        stream_flatgeobuf_to_ndjson(&input, &output)
            .unwrap_err()
            .to_string()
            .contains("XY")
    );
    assert_eq!(std::fs::read(output).unwrap(), b"old");
}
#[test]
fn malformed_late_ndjson_record_preserves_destination_and_reports_line() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("broken.ndjson");
    let output = dir.path().join("keep.fgb");
    std::fs::write(
        &input,
        format!("{}\n\n{{broken\n", serde_json::to_string(&point()).unwrap()),
    )
    .unwrap();
    std::fs::write(&output, b"old complete data").unwrap();
    assert!(
        stream_ndjson_to_flatgeobuf(&input, &output)
            .unwrap_err()
            .to_string()
            .contains("line 3")
    );
    assert_eq!(std::fs::read(output).unwrap(), b"old complete data");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
