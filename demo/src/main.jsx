import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import "./styles.css";

const sampleUrl = "/saarland-sample.json";
const sampleName = "Saarland PBF sample";

const palette = [
  "#0f766e",
  "#2563eb",
  "#c2410c",
  "#7c3aed",
  "#15803d",
  "#be123c",
  "#a16207",
  "#0369a1",
  "#b45309",
  "#4338ca",
  "#047857",
  "#9f1239"
];

const autoClassKeys = [
  "amenity",
  "shop",
  "highway",
  "building",
  "landuse",
  "natural",
  "waterway",
  "railway",
  "leisure",
  "tourism",
  "healthcare",
  "office",
  "craft",
  "boundary",
  "place",
  "route",
  "public_transport"
];

const geometryFilterValues = ["Point", "LineString", "Polygon"];
const typeFilterValues = ["node", "way", "relation"];

function App() {
  const [features, setFeatures] = useState([]);
  const [datasetName, setDatasetName] = useState("Loading Saarland PBF sample");
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [tagKey, setTagKey] = useState("");
  const [tagValue, setTagValue] = useState("");
  const [classMode, setClassMode] = useState("auto");
  const [classTag, setClassTag] = useState("amenity");
  const [activeTypes, setActiveTypes] = useState(() => new Set(typeFilterValues));
  const [activeGeometries, setActiveGeometries] = useState(() => new Set(geometryFilterValues));
  const [hiddenLayers, setHiddenLayers] = useState(() => new Set());
  const [selectedKey, setSelectedKey] = useState("");

  const tagKeys = useMemo(() => getTagKeys(features), [features]);

  const classifiedFeatures = useMemo(() => {
    return features
      .filter((feature) => matchesFilters(feature, { search, tagKey, tagValue, activeTypes, activeGeometries }))
      .map((feature) => ({
        ...feature,
        layer: classifyFeature(feature, classMode, classTag)
      }));
  }, [features, search, tagKey, tagValue, activeTypes, activeGeometries, classMode, classTag]);

  const layers = useMemo(() => summarizeLayers(classifiedFeatures), [classifiedFeatures]);
  const visibleFeatures = useMemo(() => {
    return classifiedFeatures.filter((feature) => !hiddenLayers.has(feature.layer));
  }, [classifiedFeatures, hiddenLayers]);
  const selectedFeature = visibleFeatures.find((feature) => feature.key === selectedKey) || null;

  useEffect(() => {
    loadSample();
  }, []);

  useEffect(() => {
    const available = new Set(layers.map((layer) => layer.name));
    setHiddenLayers((current) => new Set([...current].filter((layer) => available.has(layer))));
  }, [layers]);

  async function handleFile(event) {
    const file = event.target.files?.[0];
    if (!file) {
      return;
    }
    try {
      const text = await file.text();
      const nextFeatures = parseData(text);
      if (!nextFeatures.length) {
        setError("The file parsed successfully, but no supported features with geometry were found.");
        return;
      }
      setFeatures(nextFeatures);
      setDatasetName(file.name);
      setHiddenLayers(new Set());
      setSelectedKey("");
      setError("");
    } catch (readError) {
      setError(readError instanceof Error ? readError.message : String(readError));
    }
  }

  async function loadSample() {
    try {
      const response = await fetch(sampleUrl);
      if (!response.ok) {
        throw new Error(`Could not load ${sampleUrl}: ${response.status}`);
      }
      const nextFeatures = parseData(await response.text());
      setFeatures(nextFeatures);
      setDatasetName(sampleName);
      setHiddenLayers(new Set());
      setSelectedKey("");
      setError("");
    } catch (sampleError) {
      setFeatures([]);
      setDatasetName("No dataset loaded");
      setError(sampleError instanceof Error ? sampleError.message : String(sampleError));
    }
  }

  function clearData() {
    setFeatures([]);
    setDatasetName("No dataset loaded");
    setHiddenLayers(new Set());
    setSelectedKey("");
    setError("");
  }

  function toggleSetValue(setter, value) {
    setter((current) => {
      const next = new Set(current);
      if (next.has(value)) {
        next.delete(value);
      } else {
        next.add(value);
      }
      return next;
    });
  }

  function showAllLayers() {
    setHiddenLayers(new Set());
  }

  function hideAllLayers() {
    setHiddenLayers(new Set(layers.map((layer) => layer.name)));
  }

  function toggleLayer(layerName) {
    setHiddenLayers((current) => {
      const next = new Set(current);
      if (next.has(layerName)) {
        next.delete(layerName);
      } else {
        next.add(layerName);
      }
      return next;
    });
  }

  function updateClassMode(value) {
    setClassMode(value);
    setHiddenLayers(new Set());
  }

  function updateClassTag(value) {
    setClassTag(value);
    if (classMode === "tag") {
      setHiddenLayers(new Set());
    }
  }

  return (
    <main className="app">
      <aside className="sidebar">
        <header className="brand">
          <h1>osmshrink layer demo</h1>
          <p>Load exported OSM data, filter features, and toggle tag classes as Leaflet layers.</p>
        </header>

        <div className="controls">
          <section className="section">
            <h2>Data</h2>
            <div className="button-row">
              <button className="primary" type="button" onClick={loadSample}>Load sample</button>
              <button type="button" onClick={clearData}>Clear</button>
            </div>
            <div className="field with-gap">
              <label htmlFor="fileInput">JSON, NDJSON, or GeoJSON file</label>
              <input id="fileInput" type="file" accept=".json,.ndjson,.geojson,application/json,application/geo+json" onChange={handleFile} />
            </div>
            {error ? <div className="error" role="alert">{error}</div> : null}
            <div className="status" aria-live="polite">
              <Stat value={features.length} label="loaded" />
              <Stat value={visibleFeatures.length} label="visible" />
              <Stat value={layers.length} label="layers" />
            </div>
          </section>

          <section className="section">
            <h2>Filters</h2>
            <div className="field">
              <label htmlFor="searchInput">Search tags, id, or type</label>
              <input id="searchInput" type="search" value={search} placeholder="name, amenity, 123..." onChange={(event) => setSearch(event.target.value)} />
            </div>
            <div className="field-row">
              <div className="field">
                <label htmlFor="tagKeyInput">Tag key</label>
                <input id="tagKeyInput" list="tagKeys" value={tagKey} placeholder="amenity" onChange={(event) => setTagKey(event.target.value)} />
              </div>
              <div className="field">
                <label htmlFor="tagValueInput">Value contains</label>
                <input id="tagValueInput" value={tagValue} placeholder="school" onChange={(event) => setTagValue(event.target.value)} />
              </div>
            </div>
            <datalist id="tagKeys">
              {tagKeys.map((key) => <option key={key} value={key} />)}
            </datalist>
            <div className="field-row">
              <div className="field">
                <label htmlFor="classMode">Class layers by</label>
                <select id="classMode" value={classMode} onChange={(event) => updateClassMode(event.target.value)}>
                  <option value="auto">Auto tag class</option>
                  <option value="type">OSM type</option>
                  <option value="geometry">Geometry</option>
                  <option value="tag">Selected tag key</option>
                </select>
              </div>
              <div className="field">
                <label htmlFor="classTagInput">Selected tag key</label>
                <input id="classTagInput" list="tagKeys" value={classTag} placeholder="amenity" onChange={(event) => updateClassTag(event.target.value)} />
              </div>
            </div>
            <FilterCheckboxes
              title="OSM type"
              values={typeFilterValues}
              labels={{ node: "Node", way: "Way", relation: "Relation" }}
              active={activeTypes}
              onToggle={(value) => toggleSetValue(setActiveTypes, value)}
            />
            <FilterCheckboxes
              title="Geometry"
              values={geometryFilterValues}
              labels={{ Point: "Point", LineString: "Line", Polygon: "Area" }}
              active={activeGeometries}
              onToggle={(value) => toggleSetValue(setActiveGeometries, value)}
            />
          </section>

          <section className="section">
            <h2>Layers</h2>
            <div className="button-row">
              <button type="button" onClick={showAllLayers}>Show all</button>
              <button type="button" onClick={hideAllLayers}>Hide all</button>
            </div>
            <div className="layers">
              {layers.map((layer, index) => (
                <label className="layer-item" key={layer.name} title={layer.name}>
                  <input type="checkbox" checked={!hiddenLayers.has(layer.name)} onChange={() => toggleLayer(layer.name)} />
                  <span className="swatch" style={{ backgroundColor: colorForIndex(index) }} />
                  <span className="layer-name">{layer.name}</span>
                  <span className="layer-count">{layer.count}</span>
                </label>
              ))}
            </div>
          </section>
        </div>
      </aside>

      <section className="map-shell">
        <header className="toolbar">
          <div className="map-title">
            <strong>{datasetName}</strong>
            <span>{visibleFeatures.length} of {features.length} features match the active filters.</span>
          </div>
        </header>
        <LeafletMap
          features={visibleFeatures}
          layers={layers}
          selectedKey={selectedKey}
          onSelect={setSelectedKey}
        />
        <footer className="inspector">
          <FeatureTable features={visibleFeatures} onSelect={setSelectedKey} />
          <FeatureDetails feature={selectedFeature} />
        </footer>
      </section>
    </main>
  );
}

function Stat({ value, label }) {
  return (
    <div className="stat">
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  );
}

function FilterCheckboxes({ title, values, labels, active, onToggle }) {
  return (
    <div className="field with-gap">
      <label>{title}</label>
      <div className="check-row">
        {values.map((value) => (
          <label className="check" key={value}>
            <input type="checkbox" checked={active.has(value)} onChange={() => onToggle(value)} />
            {labels[value] || value}
          </label>
        ))}
      </div>
    </div>
  );
}

function LeafletMap({ features, layers, selectedKey, onSelect }) {
  const mapNodeRef = useRef(null);
  const mapRef = useRef(null);
  const featureLayerRef = useRef(null);
  const featureSignatureRef = useRef("");

  useEffect(() => {
    if (!mapNodeRef.current || mapRef.current) {
      return;
    }

    const map = L.map(mapNodeRef.current, {
      zoomControl: true,
      preferCanvas: true
    }).setView([49.235, 6.997], 13);

    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 19,
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors'
    }).addTo(map);

    featureLayerRef.current = L.layerGroup().addTo(map);
    mapRef.current = map;

    return () => {
      map.remove();
      mapRef.current = null;
      featureLayerRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!mapRef.current || !featureLayerRef.current) {
      return;
    }

    const map = mapRef.current;
    const layerGroup = featureLayerRef.current;
    const layerColor = new Map(layers.map((layer, index) => [layer.name, colorForIndex(index)]));
    layerGroup.clearLayers();

    const bounds = L.latLngBounds();
    features.forEach((feature) => {
      const color = layerColor.get(feature.layer) || colorForIndex(0);
      const leafletLayer = toLeafletLayer(feature, color, feature.key === selectedKey);
      if (!leafletLayer) {
        return;
      }

      leafletLayer.on("click", () => onSelect(feature.key));
      leafletLayer.bindPopup(popupHtml(feature));
      leafletLayer.addTo(layerGroup);
      extendBounds(bounds, leafletLayer);
    });

    const featureSignature = features.map((feature) => feature.key).join("|");
    if (bounds.isValid() && featureSignature !== featureSignatureRef.current) {
      map.fitBounds(bounds.pad(0.18), { animate: false, maxZoom: 16 });
    }
    featureSignatureRef.current = featureSignature;
  }, [features, layers, selectedKey, onSelect]);

  return (
    <div className="stage">
      <div className="leaflet-map" ref={mapNodeRef} />
      {!features.length ? (
        <div className="empty-state">
          <div>
            <strong>No visible features</strong>
            <span>Load data or loosen the active filters and layer toggles.</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function FeatureTable({ features, onSelect }) {
  return (
    <section className="results">
      <h2>Visible features</h2>
      <div className="results-grid">
        <div className="head">Name</div>
        <div className="head">Layer</div>
        <div className="head">Geometry</div>
        <div className="head">ID</div>
        {features.slice(0, 100).map((feature) => (
          <button className="result-row" key={feature.key} type="button" onClick={() => onSelect(feature.key)}>
            <span>{titleFor(feature)}</span>
            <span>{feature.layer}</span>
            <span>{feature.geometry.type}</span>
            <span>{feature.id}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

function FeatureDetails({ feature }) {
  return (
    <section className="details">
      <h2>Selection</h2>
      {!feature ? (
        <div>Select a feature on the map or table.</div>
      ) : (
        <dl>
          <dt>id</dt>
          <dd>{feature.id}</dd>
          <dt>type</dt>
          <dd>{feature.type}</dd>
          <dt>geometry</dt>
          <dd>{feature.geometry.type}</dd>
          <dt>layer</dt>
          <dd>{feature.layer}</dd>
          {Object.entries(feature.tags).map(([key, value]) => (
            <React.Fragment key={key}>
              <dt>{key}</dt>
              <dd>{value}</dd>
            </React.Fragment>
          ))}
        </dl>
      )}
    </section>
  );
}

function normalizeMany(values) {
  return values.map(normalizeFeature).filter(Boolean);
}

function normalizeFeature(feature, index = 0) {
  if (!feature || typeof feature !== "object") {
    return null;
  }

  if (feature.type === "Feature") {
    const properties = feature.properties || {};
    const osmType = properties.osm_type || parseGeojsonType(feature.id) || "feature";
    const osmId = properties.osm_id || parseGeojsonId(feature.id) || index + 1;
    const tags = {};
    for (const [key, value] of Object.entries(properties)) {
      if (key !== "osm_id" && key !== "osm_type" && value !== null && typeof value !== "object") {
        tags[key] = String(value);
      }
    }
    return normalizeFeature({ id: osmId, type: osmType, tags, geometry: feature.geometry }, index);
  }

  const geometry = normalizeGeometry(feature.geometry);
  if (!geometry) {
    return null;
  }

  const tags = {};
  for (const [key, value] of Object.entries(feature.tags || {})) {
    if (value !== null && value !== undefined && typeof value !== "object") {
      tags[key] = String(value);
    }
  }

  const type = String(feature.type || "feature").toLowerCase();
  const id = feature.id ?? index + 1;
  return {
    key: `${type}/${id}`,
    id,
    type,
    tags,
    geometry
  };
}

function normalizeGeometry(geometry) {
  if (!geometry || !["Point", "LineString", "Polygon", "MultiPolygon"].includes(geometry.type)) {
    return null;
  }
  return {
    type: geometry.type,
    coordinates: geometry.coordinates
  };
}

function parseData(text) {
  const trimmed = text.trim();
  if (!trimmed) {
    return [];
  }

  if (trimmed[0] === "[" || trimmed[0] === "{") {
    try {
      const value = JSON.parse(trimmed);
      if (Array.isArray(value)) {
        return normalizeMany(value);
      }
      if (value.type === "FeatureCollection" && Array.isArray(value.features)) {
        return normalizeMany(value.features);
      }
      return normalizeMany([value]);
    } catch (error) {
      if (trimmed[0] !== "{") {
        throw error;
      }
    }
  }

  return trimmed
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line, index) => normalizeFeature(JSON.parse(line), index))
    .filter(Boolean);
}

function parseGeojsonType(id) {
  if (typeof id !== "string") {
    return "";
  }
  const match = id.match(/^(node|way|relation)\//);
  return match ? match[1] : "";
}

function parseGeojsonId(id) {
  if (typeof id !== "string") {
    return "";
  }
  const match = id.match(/\/(-?\d+)$/);
  return match ? Number(match[1]) : "";
}

function getTagKeys(features) {
  const keys = new Set();
  features.forEach((feature) => Object.keys(feature.tags).forEach((key) => keys.add(key)));
  return [...keys].sort((a, b) => a.localeCompare(b));
}

function classifyFeature(feature, classMode, classTag) {
  if (classMode === "type") {
    return feature.type || "feature";
  }
  if (classMode === "geometry") {
    return geometryGroup(feature.geometry.type);
  }
  if (classMode === "tag") {
    const key = classTag.trim();
    if (!key) {
      return "no tag selected";
    }
    return feature.tags[key] ? `${key}=${feature.tags[key]}` : `${key} missing`;
  }

  for (const key of autoClassKeys) {
    if (feature.tags[key]) {
      return `${key}=${feature.tags[key]}`;
    }
  }
  if (feature.tags.name) {
    return "named feature";
  }
  return feature.type || geometryGroup(feature.geometry.type);
}

function matchesFilters(feature, filters) {
  if (typeFilterValues.includes(feature.type) && !filters.activeTypes.has(feature.type)) {
    return false;
  }
  if (!filters.activeGeometries.has(geometryGroup(feature.geometry.type))) {
    return false;
  }

  const key = filters.tagKey.trim();
  const value = filters.tagValue.trim().toLowerCase();
  if (key && !(key in feature.tags)) {
    return false;
  }
  if (value && key && !String(feature.tags[key] || "").toLowerCase().includes(value)) {
    return false;
  }
  if (value && !key && !Object.values(feature.tags).some((tag) => tag.toLowerCase().includes(value))) {
    return false;
  }

  const query = filters.search.trim().toLowerCase();
  if (!query) {
    return true;
  }

  const haystack = [
    feature.id,
    feature.type,
    feature.geometry.type,
    ...Object.entries(feature.tags).flat()
  ].join(" ").toLowerCase();
  return haystack.includes(query);
}

function summarizeLayers(features) {
  const counts = new Map();
  features.forEach((feature) => counts.set(feature.layer, (counts.get(feature.layer) || 0) + 1));
  return [...counts.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

function geometryGroup(type) {
  return type === "Point" || type === "LineString" ? type : "Polygon";
}

function colorForIndex(index) {
  return palette[index % palette.length];
}

function titleFor(feature) {
  return feature.tags.name || `${feature.type}/${feature.id}`;
}

function popupHtml(feature) {
  const tags = Object.entries(feature.tags)
    .slice(0, 10)
    .map(([key, value]) => `<dt>${escapeHtml(key)}</dt><dd>${escapeHtml(value)}</dd>`)
    .join("");
  return `
    <strong>${escapeHtml(titleFor(feature))}</strong>
    <dl class="popup-grid">
      <dt>id</dt><dd>${escapeHtml(String(feature.id))}</dd>
      <dt>type</dt><dd>${escapeHtml(feature.type)}</dd>
      <dt>layer</dt><dd>${escapeHtml(feature.layer)}</dd>
      ${tags}
    </dl>
  `;
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function toLeafletLayer(feature, color, selected) {
  const baseStyle = {
    color: selected ? "#b45309" : color,
    fillColor: color,
    fillOpacity: feature.geometry.type === "MultiPolygon" ? 0.16 : 0.28,
    opacity: 0.9,
    weight: selected ? 6 : 4
  };

  if (feature.geometry.type === "Point") {
    const point = lonLatToLatLng(feature.geometry.coordinates);
    if (!point) {
      return null;
    }
    return L.circleMarker(point, {
      radius: selected ? 10 : 7,
      color: "white",
      weight: 2,
      fillColor: selected ? "#b45309" : color,
      fillOpacity: 0.95
    });
  }

  if (feature.geometry.type === "LineString") {
    return L.polyline(feature.geometry.coordinates.map(lonLatToLatLng).filter(Boolean), baseStyle);
  }

  if (feature.geometry.type === "Polygon") {
    return L.polygon(feature.geometry.coordinates.map((ring) => ring.map(lonLatToLatLng).filter(Boolean)), baseStyle);
  }

  if (feature.geometry.type === "MultiPolygon") {
    return L.polygon(
      feature.geometry.coordinates.map((polygon) => polygon.map((ring) => ring.map(lonLatToLatLng).filter(Boolean))),
      baseStyle
    );
  }

  return null;
}

function lonLatToLatLng(coordinate) {
  if (!Array.isArray(coordinate) || coordinate.length < 2) {
    return null;
  }
  const lon = Number(coordinate[0]);
  const lat = Number(coordinate[1]);
  if (!Number.isFinite(lon) || !Number.isFinite(lat)) {
    return null;
  }
  return [lat, lon];
}

function extendBounds(bounds, layer) {
  if (typeof layer.getBounds === "function") {
    const layerBounds = layer.getBounds();
    if (layerBounds.isValid()) {
      bounds.extend(layerBounds);
    }
    return;
  }
  if (typeof layer.getLatLng === "function") {
    bounds.extend(layer.getLatLng());
  }
}

createRoot(document.getElementById("root")).render(<App />);
