// Generates d3-geo golden fixtures for avenger-geo.
//
// Usage: cd tools/geo-fixtures && npm ci && npm run generate
// Output: avenger-geo/tests/fixtures/<projection>_<case>.json (checked in), where a case is a
// rotation (r0, r96, oblique) or a named clip or configuration, such as cap28 or albers.

import * as d3 from "d3-geo";
import * as d3p from "d3-geo-projection";
import { geoClipPolygon } from "d3-geo-polygon";
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
  winkel_tripel: () => d3p.geoWinkel3(),
  conic_equal_area: () => d3.geoConicEqualArea().parallels([29.5, 45.5]),
  conic_conformal: () => d3.geoConicConformal().parallels([35, 65]),
  conic_equidistant: () => d3.geoConicEquidistant().parallels([30, 60]),
  transverse_mercator: () => d3.geoTransverseMercator(),
  // Azimuthal projections, each with its default clip angle.
  orthographic: () => d3.geoOrthographic(),
  stereographic: () => d3.geoStereographic(),
  gnomonic: () => d3.geoGnomonic(),
  azimuthal_equal_area: () => d3.geoAzimuthalEqualArea(),
  azimuthal_equidistant: () => d3.geoAzimuthalEquidistant(),
  mollweide: () => d3p.geoMollweide(),
  sinusoidal: () => d3p.geoSinusoidal(),
  robinson: () => d3p.geoRobinson(),
  eckert4: () => d3p.geoEckert4(),
  hammer: () => d3p.geoHammer(),
  aitoff: () => d3p.geoAitoff(),
  miller: () => d3p.geoMiller(),
  cylindrical_equal_area: () => d3p.geoCylindricalEqualArea().parallel(38.58),
};

// A longitude/latitude box as a ring in RFC 7946 winding, as avenger-geo's
// SphericalClip::lon_lat_box builds it: the south edge west to east and the
// north edge east to west, each a vertex a degree or less apart, so that the
// edges follow their parallels. A box crosses the antimeridian when east < west.
function lonLatBox(west, south, east, north) {
  if (east < west) east += 360;
  const wrap = (lon) => (lon > 180 ? lon - 360 : lon);
  const steps = Math.max(1, Math.ceil(east - west));
  const ring = [];
  for (let i = 0; i <= steps; i++) ring.push([wrap(west + ((east - west) * i) / steps), south]);
  for (let i = 0; i <= steps; i++) ring.push([wrap(east - ((east - west) * i) / steps), north]);
  ring.push(ring[0]);
  return ring;
}

// The same ring in d3's winding: the first point first, the others reversed.
function d3Ring(ring) {
  return [ring[0], ...ring.slice(1, -1).reverse(), ring[0]];
}

// Clip overrides: a circle angle in degrees, or a lon/lat box in a frame. d3
// clips in the rotated frame, as avenger-geo's view regions do. avenger-geo
// turns an earth region with the rotation, so an earth box's vertices are
// rotated here.
function withClip(projection, clip, rotate) {
  if (clip?.circle) return projection.clipAngle(clip.circle);
  if (clip?.box) {
    let ring = d3Ring(lonLatBox(...clip.box));
    if (clip.frame === "earth") ring = ring.map(d3.geoRotation(rotate));
    return projection.preclip(geoClipPolygon({ type: "Polygon", coordinates: [ring] }));
  }
  return projection;
}

// Fixtures for clips that override a projection's default.
const clipCases = {
  orthographic_cap28: { projection: "orthographic", rotate: [-140, -35, 0], clip: { circle: 28 } },
  equal_earth_cap28: { projection: "equal_earth", rotate: [96, 0, 0], clip: { circle: 28 } },
  equal_earth_box_africa: { projection: "equal_earth", rotate: [0, 0, 0], clip: { box: [-30, -40, 60, 40], frame: "earth" } },
  equirectangular_box_pacific: { projection: "equirectangular", rotate: [180, 0, 0], clip: { box: [150, -50, -150, 10], frame: "earth" } },
  mercator_box_band: { projection: "mercator", rotate: [0, 0, 0], clip: { box: [-180, -65, 180, 75], frame: "earth" } },
  mercator_box_band_view: { projection: "mercator", rotate: [-145, 0, 0], clip: { box: [-180, -65, 180, 75], frame: "view" } },
  equal_earth_box_view: { projection: "equal_earth", rotate: [96, 0, 0], clip: { box: [-30, -40, 60, 40], frame: "view" } },
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
// The square spans 31 degrees of latitude. An edge of exactly 30 degrees, the resampler's
// minimum angular distance, splits or not by the last bit of sin and cos, which differs by
// platform.
const midLatSquare = {
  type: "Polygon",
  coordinates: [
    [
      [10, 10],
      [10, 41],
      [40, 41],
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

function writeFixture(fileName, projName, factory, rotate, center, clip) {
  const make = () =>
    withClip(factory(), clip, rotate).rotate(rotate).center(center).precision(Math.sqrt(0.5));
  const projection = make().scale(150).translate([480, 250]);

  // A point that the projection's clip leaves out streams nothing, and has no
  // inverse to check.
  const points = pointGrid.map(([lon, lat]) => {
    const visible = recordStream(projection, { type: "Point", coordinates: [lon, lat] }).length > 0;
    const p = projection([lon, lat]);
    const finite = p && p.every(Number.isFinite);
    const inv = finite && projection.invert ? projection.invert(p) : null;
    return {
      lonlat: [lon, lat],
      visible,
      xy: finite ? [round(p[0]), round(p[1])] : null,
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

  const fitProjection = make();
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
    clip: clip ?? null,
    scale: 150,
    translate: [480, 250],
    precision: Math.sqrt(0.5),
    points,
    streams,
    bounds,
    fit,
  };

  const file = join(outDir, `${fileName}.json`);
  writeFileSync(file, JSON.stringify(fixture));
  console.log(`wrote ${file}`);
}

for (const [projName, factory] of Object.entries(projections)) {
  for (const [rotName, { rotate, center }] of Object.entries(cases)) {
    // Only run the albers-center case for the conic it belongs to.
    if (rotName === "albers" && projName !== "conic_equal_area") continue;
    writeFixture(`${projName}_${rotName}`, projName, factory, rotate, center, null);
  }
}
for (const [name, { projection, rotate, clip }] of Object.entries(clipCases)) {
  writeFixture(name, projection, projections[projection], rotate, [0, 0], clip);
}
console.log("done");
