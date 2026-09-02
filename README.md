# osmshrink

`osmshrink` downloads OpenStreetMap regional extracts, filters nodes, ways, and
area relations from `.osm.pbf` files, and writes normalized `.ndjson`, `.json`,
or `.geojson` output for downstream data pipelines. It also provides a neutral
geospatial feature model and conversion layer so non-OSM datasets can move between
supported interchange formats without adopting OSM's node/way/relation model.

The first supported download provider is Geofabrik. Sources can be direct URLs or
short names such as:

```bash
geofabrik:europe/germany/baden-wuerttemberg
```

which resolves to:

```text
https://download.geofabrik.de/europe/germany/baden-wuerttemberg-latest.osm.pbf
```

## Why

OSM extracts are rich but large. `osmshrink` is a small Rust CLI and reusable
library core for downloading an extract once, applying practical tag and bbox
filters, and exporting only the data needed by an application.

The same repository also needs to consume datasets that originate outside OSM.
The geospatial interoperability layer therefore separates OSM-specific processing
from a format-neutral `GeoFeature` / `GeoDataset` model, with explicit conversion
loss reporting instead of pretending all geospatial formats are equivalent.

The implementation does not shell out to tools such as `osmium`.

## Install

From this repository:

```bash
cargo build --release
```

During development:

```bash
cargo build
```

## Usage

Download an extract:

```bash
cargo run -- fetch geofabrik:europe/germany/baden-wuerttemberg --output data/bw.osm.pbf
cargo run -- fetch https://download.geofabrik.de/europe/germany-latest.osm.pbf --output data/germany.osm.pbf
```

`fetch` reuses the output file when it already exists, so repeating the same
command does not download the extract again. Use `--force` to refresh it:

```bash
cargo run -- fetch geofabrik:europe/germany/baden-wuerttemberg --output data/bw.osm.pbf --force
```

Filter a local extract:

```bash
cargo run -- filter --input data/bw.osm.pbf --spec examples/schools.json --output out/schools.ndjson
cargo run -- filter --input data/bw.osm.pbf --spec examples/roads.yaml --format json --output out/roads.json
cargo run -- filter --input data/bw.osm.pbf --spec examples/areas_geojson.yaml --output out/areas.geojson
```

You can also pass the filter on the command line, similar to `jq`, or use
`--filter-file`/`-f` as an alias for `--spec`:

```bash
cargo run -- filter --input data/bw.osm.pbf --output out/schools.ndjson 'amenity=school'
cargo run -- filter --input data/bw.osm.pbf --output out/schools.ndjson '{key: amenity, value: school}'
cargo run -- filter --input data/bw.osm.pbf --output out/roads.ndjson '{types: [way], include: {any: [{key: highway, values: [primary, secondary]}]}}'
cargo run -- filter --input data/bw.osm.pbf -f examples/schools.json --output out/schools.ndjson
```

Convert a geospatial dataset:

```bash
cargo run -- convert data/features.geojson --output out/features.json
cargo run -- convert out/features.json --output out/features.ndjson
cargo run -- convert out/features.ndjson --output out/features.geojson
```

The first conversion batch supports GeoJSON, neutral `GeoDataset` JSON, and
feature-oriented NDJSON. Existing osmshrink JSON/NDJSON records are accepted and
promoted into the neutral model. Input/output formats are inferred from extensions;
use `--from` and `--to` to override detection:

```bash
cargo run -- convert data/export.json --from geojson --to ndjson --output out/features.data
```

Conversions report semantic loss. For example, NDJSON cannot carry dataset-level
metadata or CRS information, while RFC 7946 GeoJSON does not carry a custom CRS
member. See [`docs/formats.md`](docs/formats.md) for the model, loss semantics,
and the planned FlatGeobuf, TopoJSON, GeoParquet, GIS, and map-delivery adapters.

Fetch and filter in one command:

```bash
cargo run -- run \
  --source geofabrik:europe/germany/baden-wuerttemberg \
  --spec examples/schools.json \
  --output out/schools.ndjson
```

Inspect an input file:

```bash
cargo run -- inspect --input data/bw.osm.pbf
```

Validate a spec:

```bash
cargo run -- validate-spec --spec examples/schools.json
cargo run -- validate-spec '{key: amenity, value: school}'
```

Use `--verbose` for more logging and `--quiet` to suppress status output. Use
`--index auto|memory|disk`, `--index-dir`, and `--memory-node-limit` to override
node index behavior from the spec.

## Filter Specs

Specs can be JSON or YAML. A typical spec looks like:

```json
{
  "source": {
    "provider": "geofabrik",
    "region": "europe/germany/baden-wuerttemberg"
  },
  "filter": {
    "bbox": [8.5, 48.8, 9.3, 49.2],
    "types": ["node", "way", "relation"],
    "include": {
      "any": [
        { "key": "amenity", "values": ["school", "hospital"] },
        { "key": "highway", "values": ["primary", "secondary", "tertiary"] }
      ],
      "all": [
        { "key": "name", "exists": true }
      ]
    },
    "exclude": [
      { "key": "access", "values": ["private"] }
    ]
  },
  "processing": {
    "index": {
      "mode": "auto",
      "memory_node_limit": 5000000,
      "disk_dir": null
    }
  },
  "output": {
    "format": "ndjson",
    "geometry": "full",
    "fields": ["id", "type", "tags", "geometry"]
  }
}
```

Supported filter features:

- `types`: `node`, `way`, and `relation`. If omitted, nodes and ways are used.
- `bbox`: `[min_lon, min_lat, max_lon, max_lat]`.
- `include.any`: at least one condition must match.
- `include.all`: every condition must match.
- `exclude`: if any condition matches, the object is dropped.

Supported condition operators:

- `{ "key": "name", "exists": true }`
- `{ "key": "amenity", "value": "school" }`
- `{ "key": "amenity", "values": ["school", "hospital"] }`
- `{ "key": "name", "regex": "School|Hospital" }`
- add `"not": true` to invert a condition.

If `include` is omitted, objects pass tag filtering unless excluded. If both
`include.any` and `include.all` are present, both groups must pass.

Inline filters can be a full JSON/YAML spec, just the `filter` rules object, a
single tag condition, or a list of tag conditions. A single condition or list is
treated as `include.all`. For quick tag matches, `key=value`, `key!=value`,
`key~regex`, and `key` are accepted as shorthand conditions.

BBox behavior:

- nodes match by their own coordinates.
- ways match when any resolved node coordinate is inside the bbox.
- relations match when any assembled polygon coordinate is inside the bbox.

Relation behavior:

- only area relations are emitted: `type=multipolygon` and `type=boundary`.
- relation members with role `outer`, role `inner`, or an empty role are used.
- other member roles and non-way relation members are ignored and counted.
- relations are skipped if required member ways or nodes are missing, rings do
  not close, or inner rings cannot be assigned to an outer ring.

Index behavior:

- `processing.index.mode` can be `auto`, `memory`, or `disk`.
- `auto` starts with an in-memory node coordinate index and spills to a temporary
  `redb` database after `memory_node_limit` nodes.
- `disk_dir` optionally chooses where temporary disk-backed indexes are created.

## Output

`ndjson` is the default and is preferred for large OSM filter outputs. `json`
writes one array. `geojson` writes a GeoJSON `FeatureCollection`.

Each emitted OSM object has a normalized shape:

```json
{
  "id": 123,
  "type": "way",
  "tags": {
    "highway": "primary",
    "name": "B10"
  },
  "geometry": {
    "type": "LineString",
    "coordinates": [
      [8.70, 48.89],
      [8.71, 48.90]
    ]
  }
}
```

Nodes emit `Point` geometry. Ways emit `LineString` by default. If
`output.geometry` is `polygon`, closed ways with at least four coordinates emit a
`Polygon`. Area relations emit `Polygon` or `MultiPolygon`.

For GeoJSON output, `output.fields` must include `geometry`. If `id` is included,
the GeoJSON feature id is formatted as `node/123`, `way/123`, or `relation/123`,
with `osm_id` added to properties. If `type` is included, `osm_type` is added to
properties. If `tags` is included, OSM tags are copied to properties.

`output.fields` controls which fields are written, for example:

```json
["id", "geometry"]
```

## Interactive Demo

The React and Leaflet demo in `demo/` loads `.json`, `.ndjson`, and `.geojson`
output files from disk, applies type and tag filters, and groups matching
features into toggleable map layers. Its default sample is generated from the
Saarland Geofabrik PBF with `examples/saarland_demo.json`.

```bash
cd demo
bun install
bun run dev
```

The demo can also convert a local `.osm.pbf` or `.pbf` file directly in the
browser. Conversion runs in a Web Worker through the Rust wasm package; the PBF
is read from the upload input and no PBF bytes leave the browser. The first
browser version is upload-only: it does not fetch Geofabrik extracts or arbitrary
remote PBF URLs.

Browser conversion always uses a memory node index. In wasm builds,
`processing.index.mode = "auto"` stays in memory and `"disk"` is rejected because
browser runtimes do not provide the native disk-backed index used by the CLI.
The demo warns before processing files above 100 MiB and requires explicit
confirmation for large uploads.

Wasm demo requirements:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Build the wasm package and start Vite:

```bash
cd demo
bun run dev:wasm
```

Production builds also build the wasm package first:

```bash
cd demo
bun run build
```

Regenerate the sample data:

```bash
cargo run -- fetch geofabrik:europe/germany/saarland --output data/saarland.osm.pbf
cargo run -- filter --input data/saarland.osm.pbf --spec examples/saarland_demo.json --format json --output demo/public/saarland-sample.json
```

## Architecture

The crate is split into a reusable library and a CLI:

- `cli`: clap command definitions.
- `fetch`: streaming HTTP download to a temporary file, then rename on success.
- `geofabrik`: shorthand parsing and URL resolution.
- `spec`: serde models and validation for JSON/YAML specs.
- `filter`: predicate compilation, PBF filtering pipeline, and area relation assembly.
- `geometry`: bbox checks and OSM point/way/relation geometry creation.
- `index`: in-memory, disk-backed, and auto-spilling node coordinate indexes.
- `model`: stable OSM-specific normalized output model.
- `geo`: neutral geospatial feature/dataset model plus reader/writer adapter seam.
- `convert`: format detection, GeoJSON/JSON/NDJSON adapters, and typed loss reports.
- `output`: stable OSM JSON, NDJSON, and GeoJSON writers.
- `inspect`: cheap filesystem-level input inspection.
- `error`: typed application errors.

The OSM model remains public and backward-compatible. `GeoFeature` / `GeoDataset`
are an additional interoperability boundary rather than a breaking replacement.

## Library Use

The OSM data model is available as Rust structs:

```rust
use osmshrink::{CollectRunOptions, FilterSpec, collect_pbf};
use osmshrink::index::IndexOptions;

let spec = FilterSpec::from_inline("amenity=school")?;
let index_options = IndexOptions::from_spec(&spec.processing.index);
let collected = collect_pbf(CollectRunOptions {
    input: "data/bw.osm.pbf".into(),
    spec,
    index_options,
})?;

for feature in collected.features {
    println!("{} {:?}", feature.id, feature.tags);
}
# Ok::<(), osmshrink::OsmshrinkError>(())
```

`Feature`, `ElementKind`, `Geometry`, and `Tags` are public and support serde
serialization/deserialization, so JSON output can also be read directly into
`Vec<osmshrink::Feature>` when using array JSON.

For format-independent processing, use `GeoFeature`, `GeoDataset`,
`GeoFeatureReader`, and `GeoFeatureWriter`. OSM features can be promoted without
losing their identity:

```rust
use osmshrink::{Feature, GeoFeature};

fn promote(feature: &Feature) -> GeoFeature {
    GeoFeature::from_osm(feature)
}
```

The streaming `pipe_features` helper connects a `GeoFeatureReader` to a
`GeoFeatureWriter`. Binary/columnar adapters should implement this seam so large
files do not need to be materialized into a complete `GeoDataset`.

## Limitations

- Way geometry requires referenced nodes to exist in the extract. A way with
  missing node coordinates is skipped with a warning.
- Relation support is limited to area relations (`multipolygon` and `boundary`).
  Other relation types are skipped and counted.
- Relation assembly supports way members only. Nested relations are ignored.
- Auto indexing can spill node coordinates to disk, but relation member ways are
  still retained in memory while assembling candidate area relations.
- The first generic conversion batch supports GeoJSON, neutral JSON, and NDJSON;
  FlatGeobuf, TopoJSON, GeoParquet, GeoPackage/Shapefile, GPX/KML, and MVT/PMTiles
  are deliberately staged rather than implemented as ad-hoc parsers in one change.
- Conversion does not reproject coordinates. CRS incompatibilities are reported
  rather than silently transformed or relabeled.

## Tests

```bash
cargo test --all-features
```

Tests cover Geofabrik shorthand parsing, JSON/YAML spec parsing, condition
matching, bbox logic, field filtering, GeoJSON output, node indexes,
constructed-object relation assembly, neutral OSM feature promotion, adapter
piping, GeoJSON round trips, legacy osmshrink JSON ingestion, and conversion-loss
reporting.

Pull requests also run formatting, a no-default-features core check, Clippy across
all targets/features, and the full test suite in `.github/workflows/ci.yml`.

## Benchmarks

Run the end-to-end filter benchmark:

```bash
cargo bench --bench filter
```

The benchmark generates a deterministic synthetic `.osm.pbf` fixture with
20,000 nodes and 5,000 matching ways, then measures the real `filter_pbf`
pipeline using the in-memory node index.

## Roadmap

- FlatGeobuf adapter through the streaming reader/writer seam.
- TopoJSON adapter with topology-preservation and explicit topology-loss rules.
- GeoParquet and established GIS adapters after the streaming contract is proven.
- GPX/KML interoperability for tracks, routes, and point datasets.
- MVT/PMTiles output as a separate tiling/generalization layer.
- Python and Node bindings over the Rust library core.
- More relation types beyond area relations.
- Streaming relation member geometry storage for very large relation-heavy extracts.
