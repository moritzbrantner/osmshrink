# Geospatial format interoperability

`osmshrink` treats OpenStreetMap PBF as an important source format, but the reusable
library should not make OSM's node/way/relation model the universal representation
for every geospatial dataset.

The interoperability layer therefore has three parts:

1. format adapters read external datasets into `GeoFeature` / `GeoDataset`;
2. processing code works against those neutral types or the streaming
   `GeoFeatureReader` / `GeoFeatureWriter` seam;
3. output adapters serialize the neutral representation and report semantic loss
   when a destination cannot represent part of the source.

This keeps OSM-specific identifiers and tags useful without forcing them onto
GeoJSON, TopoJSON, FlatGeobuf, GeoParquet, or other formats.

## Neutral model

`GeoFeature` preserves:

- an optional string or numeric feature id;
- arbitrary JSON properties, rather than string-only OSM tags;
- any geometry supported by the `geojson` crate, including `MultiPoint`,
  `MultiLineString`, and `GeometryCollection`;
- an optional feature bounding box;
- format-specific/foreign metadata.

`GeoDataset` adds dataset-level bounding box, CRS metadata, and arbitrary metadata.
OSM `Feature` values can be promoted into this model. Their OSM identity is kept as
`osm_id` and `osm_type` properties and as a stable `node/123`, `way/123`, or
`relation/123` feature id.

The neutral model is intentionally not a replacement for topology-aware or
columnar representations. It is the interoperability boundary for feature-oriented
processing. Adapters may preserve extra format information in metadata or report
that a conversion cannot round-trip it.

## CLI conversion

Current first-batch formats:

| Format | Extension | Read | Write | Notes |
| --- | --- | --- | --- | --- |
| GeoJSON | `.geojson` | yes | yes | RFC 7946 feature/geometry semantics and foreign members |
| Neutral JSON | `.json` | yes | yes | `GeoDataset` envelope; preserves dataset CRS and metadata |
| NDJSON | `.ndjson` | yes | yes | feature records only; dataset envelope is intentionally omitted |
| Existing osmshrink JSON | `.json` | yes | via neutral formats | OSM records are promoted to `GeoFeature` |
| Existing osmshrink NDJSON | `.ndjson` | yes | via neutral formats | OSM records are promoted per line |

Examples:

```bash
osmshrink convert data/features.geojson --output out/features.json
osmshrink convert out/features.json --output out/features.ndjson
osmshrink convert out/features.ndjson --output out/features.geojson
```

When an extension is ambiguous, override detection explicitly:

```bash
osmshrink convert data/export.json --from geojson --to ndjson --output out/features.data
```

`--from` and `--to` currently accept `geojson`, `json`, and `ndjson`.

## Loss reporting

A conversion must not silently imply that formats are equivalent. `ConversionReport`
contains typed loss entries. The initial categories are:

- `metadata`: destination format cannot carry dataset or foreign metadata;
- `crs`: destination cannot carry the source CRS contract;
- `topology`: destination cannot preserve shared topology/arc identity.

For example, NDJSON is feature-oriented and has no dataset envelope. Converting a
`GeoDataset` with a dataset bounding box, metadata, or CRS to NDJSON reports that
those fields will be omitted.

RFC 7946 GeoJSON uses the WGS84/CRS84 coordinate contract and does not use the
legacy custom `crs` member. A neutral dataset carrying another CRS therefore gets
a `crs` loss entry when written as GeoJSON. `osmshrink` does **not** silently
reproject coordinates yet; a future projection adapter should make reprojection
explicit.

TopoJSON needs special treatment. Its shared arcs encode topology, so a
TopoJSON -> independent features -> TopoJSON round trip is not inherently
lossless. The TopoJSON adapter must preserve topology metadata where possible or
emit `topology` loss explicitly.

## Adapter direction

The next adapters should be added in this order unless consumer evidence changes
the priority:

1. **FlatGeobuf** — binary, streamable geospatial feature interchange with optional
   spatial indexing. Implement this through the streaming adapter seam so large
   datasets do not require full materialization.
2. **TopoJSON** — topology-aware import/export with explicit shared-arc loss rules.
3. **GeoParquet** — columnar analytical datasets and large corpus workflows.
4. **GeoPackage / Shapefile** — compatibility with established desktop GIS data.
5. **GPX / KML** — tracks, routes, points, and common interchange sources.
6. **MVT / PMTiles** — map-delivery formats; keep tiling/generalization decisions
   separate from generic dataset conversion.

Where mature Rust implementations exist, adapters should reuse them instead of
turning this repository into a new GDAL implementation. `geozero` is the preferred
first integration candidate for streaming-capable formats such as FlatGeobuf.

## Streaming contract

The core exposes:

```rust
pub trait GeoFeatureReader {
    fn next_feature(&mut self) -> Result<Option<GeoFeature>>;
}

pub trait GeoFeatureWriter {
    fn write_feature(&mut self, feature: &GeoFeature) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}
```

`pipe_features` connects the two. Format adapters that support streaming should
implement these traits. The current JSON conversion path may materialize a
`GeoDataset` because dataset-level metadata and compatibility parsing are part of
the first batch; large binary/columnar adapters should not copy that limitation.

## Compatibility boundary

The existing OSM filter pipeline remains OSM-specific and keeps its stable
`Feature`, `ElementKind`, `Tags`, and output behavior. The neutral model is an
additional interoperability layer, not a breaking replacement of the public OSM
API. Future refactors may route existing GeoJSON output through the neutral writer
once parity tests prove that public output is unchanged.
