// Generates d3-geo golden fixtures for avenger-geo.
//
// Usage: cd tools/geo-fixtures && npm install && npm run generate
// Output: avenger-geo/tests/fixtures/<projection>_<rotation>.json (checked in)

import * as d3 from "d3-geo";
import { geoWinkel3 } from "d3-geo-projection";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const outDir = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "avenger-geo",
  "tests",
  "fixtures"
);
mkdirSync(outDir, { recursive: true });

const projections = {
  equirectangular: () => d3.geoEquirectangular(),
  mercator: () => d3.geoMercator(),
  equal_earth: () => d3.geoEqualEarth(),
  natural_earth1: () => d3.geoNaturalEarth1(),
  winkel_tripel: () => geoWinkel3(),
  conic_equal_area: () => d3.geoConicEqualArea().parallels([29.5, 45.5]),
  conic_conformal: () => d3.geoConicConformal().parallels([35, 65]),
};

// Each case: rotation plus projection center (exercises d3 `.center()`
// recentering; `albers` is the classic d3.geoAlbers CONUS aspect).
const cases = {
  r0: { rotate: [0, 0, 0], center: [0, 0] },
  r96: { rotate: [96, 0, 0], center: [0, 0] },
  oblique: { rotate: [15, -30, 12], center: [0, 0] },
  albers: { rotate: [96, 0, 0], center: [-0.6, 38.7] },
};

// Test geometry. Polygon exteriors follow the d3 spherical winding
// convention (clockwise; the small region is the interior).
const antimeridianLine = {
  type: "LineString",
  coordinates: [
    [-170, 10],
    [170, 20],
  ],
};
const londonTokyo = {
  type: "LineString",
  coordinates: [
    [-0.1, 51.5],
    [139.7, 35.7],
  ],
};
const midLatSquare = {
  type: "Polygon",
  coordinates: [
    [
      [10, 10],
      [10, 40],
      [40, 40],
      [40, 10],
      [10, 10],
    ],
  ],
};
const southPoleRing = {
  type: "Polygon",
  coordinates: [
    [
      [0, -60],
      [60, -60],
      [120, -60],
      [180, -60],
      [-120, -60],
      [-60, -60],
      [0, -60],
    ],
  ],
};
const sphere = { type: "Sphere" };
const graticule = d3.geoGraticule10();

const streamObjects = {
  antimeridian_line: antimeridianLine,
  london_tokyo: londonTokyo,
  mid_lat_square: midLatSquare,
  south_pole_ring: southPoleRing,
  sphere,
  graticule,
};

function recordStream(projection, object) {
  const events = [];
  const sink = {
    point(x, y) {
      events.push(["pt", round(x), round(y)]);
    },
    lineStart() {
      events.push(["ls"]);
    },
    lineEnd() {
      events.push(["le"]);
    },
    polygonStart() {
      events.push(["ps"]);
    },
    polygonEnd() {
      events.push(["pe"]);
    },
    sphere() {
      events.push(["sp"]);
    },
  };
  d3.geoStream(object, projection.stream(sink));
  return events;
}

function round(v) {
  return Math.round(v * 1e9) / 1e9;
}

const pointGrid = [];
for (let lon = -170; lon <= 170; lon += 40) {
  for (let lat = -80; lat <= 80; lat += 40) {
    pointGrid.push([lon, lat]);
  }
}

for (const [projName, factory] of Object.entries(projections)) {
  for (const [rotName, { rotate, center }] of Object.entries(cases)) {
    // Only run the albers-center case for the conic it belongs to.
    if (rotName === "albers" && projName !== "conic_equal_area") continue;
    const projection = factory()
      .rotate(rotate)
      .scale(150)
      .translate([480, 250])
      .center(center)
      .precision(Math.sqrt(0.5));

    const points = pointGrid.map(([lon, lat]) => {
      const p = projection([lon, lat]);
      const inv = projection.invert ? projection.invert(p) : null;
      return {
        lonlat: [lon, lat],
        xy: [round(p[0]), round(p[1])],
        inv: inv ? [round(inv[0]), round(inv[1])] : null,
      };
    });

    const streams = {};
    for (const [name, object] of Object.entries(streamObjects)) {
      streams[name] = recordStream(projection, object);
    }

    const path = d3.geoPath(projection);
    const bounds = {
      sphere: path.bounds(sphere),
      mid_lat_square: path.bounds(midLatSquare),
    };

    const fitProjection = factory()
      .rotate(rotate)
      .center(center)
      .precision(Math.sqrt(0.5));
    fitProjection.fitExtent(
      [
        [0, 0],
        [800, 600],
      ],
      sphere
    );
    const fit = {
      sphere_extent_800x600: {
        scale: round(fitProjection.scale()),
        translate: fitProjection.translate().map(round),
      },
    };

    const fixture = {
      projection: projName,
      rotate,
      center,
      scale: 150,
      translate: [480, 250],
      precision: Math.sqrt(0.5),
      points,
      streams,
      bounds,
      fit,
    };

    const file = join(outDir, `${projName}_${rotName}.json`);
    writeFileSync(file, JSON.stringify(fixture));
    console.log(`wrote ${file}`);
  }
}
console.log("done");
