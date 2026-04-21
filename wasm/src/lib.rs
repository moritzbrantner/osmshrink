use std::sync::Once;

use osmshrink::{
    CollectBytesOptions, CollectReport, Feature, FilterSpec, collect_pbf_bytes, index::IndexOptions,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

static PANIC_HOOK: Once = Once::new();

#[derive(Serialize)]
struct ConversionResult {
    features: Vec<Feature>,
    report: ConversionReport,
}

#[derive(Serialize)]
struct ConversionReport {
    objects_collected: u64,
    ways_skipped_missing_nodes: u64,
    relations_skipped_non_area: u64,
    relations_skipped_missing_members: u64,
    relations_skipped_invalid_rings: u64,
    relation_members_ignored_role: u64,
    index_backend: &'static str,
}

impl From<CollectReport> for ConversionReport {
    fn from(report: CollectReport) -> Self {
        Self {
            objects_collected: report.objects_collected,
            ways_skipped_missing_nodes: report.ways_skipped_missing_nodes,
            relations_skipped_non_area: report.relations_skipped_non_area,
            relations_skipped_missing_members: report.relations_skipped_missing_members,
            relations_skipped_invalid_rings: report.relations_skipped_invalid_rings,
            relation_members_ignored_role: report.relation_members_ignored_role,
            index_backend: report.index_backend.as_str(),
        }
    }
}

#[wasm_bindgen]
pub fn convert_pbf(pbf: &[u8], spec: JsValue) -> Result<JsValue, JsValue> {
    PANIC_HOOK.call_once(console_error_panic_hook::set_once);

    let spec: FilterSpec = serde_wasm_bindgen::from_value(spec)
        .map_err(|error| JsValue::from_str(&format!("invalid filter spec: {error}")))?;
    let index_options = IndexOptions::from_spec(&spec.processing.index);
    let collected = collect_pbf_bytes(CollectBytesOptions {
        input: pbf,
        spec,
        index_options,
    })
    .map_err(|error| JsValue::from_str(&error.to_string()))?;

    let result = ConversionResult {
        features: collected.features,
        report: collected.report.into(),
    };
    let json = serde_json::to_string(&result)
        .map_err(|error| JsValue::from_str(&format!("failed to serialize result: {error}")))?;
    js_sys::JSON::parse(&json)
}
