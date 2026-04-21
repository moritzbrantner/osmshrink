import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";

const sampleFeatureCount = 3383;
const sampleLayerCount = 24;

test.beforeEach(async ({ page }) => {
  await page.route(/tile\.openstreetmap\.org/, async (route) => {
    await route.fulfill({ status: 204, body: "" });
  });
  await page.goto("/");
});

test("loads the Saarland sample into the map and feature table", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "osmshrink layer demo" })).toBeVisible();
  await expect(page.getByText("Saarland PBF sample")).toBeVisible();

  await expect(statValue(page, "loaded")).toHaveText(String(sampleFeatureCount));
  await expect(statValue(page, "visible")).toHaveText(String(sampleFeatureCount));
  await expect(statValue(page, "layers")).toHaveText(String(sampleLayerCount));
  await expect(page.locator(".map-title span")).toHaveText(
    `${sampleFeatureCount} of ${sampleFeatureCount} features match the active filters.`
  );

  await expect(page.getByRole("button", { name: /Backwerk/ }).first()).toBeVisible();
  await expect(page.locator(".leaflet-map")).toHaveClass(/leaflet-container/);
});

test("filters sample features and updates selection details", async ({ page }) => {
  await waitForSample(page);

  await page.getByLabel("Search tags, id, or type").fill("Backwerk");
  await expect(page.locator(".map-title span")).toHaveText(
    `2 of ${sampleFeatureCount} features match the active filters.`
  );

  await page.getByRole("button", { name: /Backwerk/ }).first().click();
  await expect(selectionDetails(page)).toContainText("271513375");
  await expect(selectionDetails(page)).toContainText("shop");
  await expect(selectionDetails(page)).toContainText("bakery");

  await page.getByLabel("Search tags, id, or type").clear();
  await page.getByLabel("Tag key", { exact: true }).fill("amenity");
  await page.getByLabel("Value contains").fill("school");
  await expect(page.locator(".map-title span")).toHaveText(
    `48 of ${sampleFeatureCount} features match the active filters.`
  );

  await filterSection(page).getByLabel("Area").uncheck();
  await expect(page.locator(".map-title span")).toHaveText(
    `3 of ${sampleFeatureCount} features match the active filters.`
  );
  await expect(page.getByText("No visible features")).toBeHidden();

  await page.getByRole("button", { name: "Hide all" }).click();
  await expect(page.locator(".map-title span")).toHaveText(
    `0 of ${sampleFeatureCount} features match the active filters.`
  );
  await expect(page.getByText("No visible features")).toBeVisible();
});

test("imports GeoJSON and downloads the selected output format", async ({ page }) => {
  await uploadGeojsonFixture(page);

  await expect(statValue(page, "loaded")).toHaveText("2");
  await expect(statValue(page, "visible")).toHaveText("2");
  await expect(page.getByText("custom-fixture.geojson")).toBeVisible();

  await page.getByRole("button", { name: /Clinic Point/ }).click();
  await expect(selectionDetails(page)).toContainText("1001");
  await expect(selectionDetails(page)).toContainText("amenity");
  await expect(selectionDetails(page)).toContainText("clinic");

  await page.getByLabel("Download format").selectOption("ndjson");
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: "Download" }).click()
  ]);

  expect(download.suggestedFilename()).toBe("custom-fixture.ndjson");
  const path = await download.path();
  expect(path).toBeTruthy();

  const lines = (await readFile(path, "utf8")).trim().split("\n").map((line) => JSON.parse(line));
  expect(lines).toEqual([
    {
      id: 1001,
      type: "node",
      tags: { amenity: "clinic", name: "Clinic Point" },
      geometry: { type: "Point", coordinates: [7.01, 49.23] }
    },
    {
      id: 2002,
      type: "way",
      tags: { highway: "service", name: "Service Lane" },
      geometry: {
        type: "LineString",
        coordinates: [
          [7.01, 49.23],
          [7.02, 49.24]
        ]
      }
    }
  ]);
});

test("validates PBF conversion settings before starting conversion", async ({ page }) => {
  await page.getByLabel("Local .osm.pbf or .pbf file").setInputFiles({
    name: "tiny.osm.pbf",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("not a real pbf")
  });

  await page.getByLabel("Min lon").fill("7");
  await page.getByRole("button", { name: "Convert" }).click();
  await expect(page.getByRole("alert")).toHaveText("Fill all four bbox fields, or leave all bbox fields empty.");

  await page.getByLabel("Min lon").clear();
  await pbfSection(page).getByLabel("Node").uncheck();
  await pbfSection(page).getByLabel("Way").uncheck();
  await pbfSection(page).getByLabel("Relation").uncheck();
  await page.getByRole("button", { name: "Convert" }).click();
  await expect(page.getByRole("alert")).toHaveText("Select at least one PBF object type.");
});

function statValue(page, label) {
  return dataSection(page).locator(".stat").filter({ hasText: label }).locator("strong");
}

function dataSection(page) {
  return page.locator("section.section").filter({ has: page.getByRole("heading", { name: "Data" }) });
}

function filterSection(page) {
  return page.locator("section.section").filter({ has: page.getByRole("heading", { name: "Filters" }) });
}

function pbfSection(page) {
  return page.locator("section.section").filter({ has: page.getByRole("heading", { name: "PBF conversion" }) });
}

function selectionDetails(page) {
  return page.locator(".details");
}

async function waitForSample(page) {
  await expect(statValue(page, "loaded")).toHaveText(String(sampleFeatureCount));
}

async function uploadGeojsonFixture(page) {
  const geojson = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        id: "node/1001",
        properties: {
          osm_id: 1001,
          osm_type: "node",
          amenity: "clinic",
          name: "Clinic Point"
        },
        geometry: {
          type: "Point",
          coordinates: [7.01, 49.23]
        }
      },
      {
        type: "Feature",
        id: "way/2002",
        properties: {
          osm_id: 2002,
          osm_type: "way",
          highway: "service",
          name: "Service Lane"
        },
        geometry: {
          type: "LineString",
          coordinates: [
            [7.01, 49.23],
            [7.02, 49.24]
          ]
        }
      }
    ]
  };

  await page.getByLabel("JSON, NDJSON, or GeoJSON file").setInputFiles({
    name: "custom-fixture.geojson",
    mimeType: "application/geo+json",
    buffer: Buffer.from(JSON.stringify(geojson))
  });
}
