// Bakes CTA 'L' line geometry into src/track.json for the TUI track map (fio 4).
//
// Reads two files from the sibling Worker repo:
//   lines.geojson  — ordered LineString coordinates per route (the rail)
//   ctaData.js     — `ctaStops` array: station names, lat/lon, per-line flags
//
// Emits, per API route key, the concatenated polyline ([lon,lat] pairs) plus
// the de-duplicated set of stations on that line. The TUI orders stations and
// projects live trains onto the polyline at runtime.
//
//   node scripts/build_track.mjs [path-to-worker-repo]
// default worker repo: ../cta

import fs from "node:fs";
import path from "node:path";

const WORKER = process.argv[2] || path.resolve(process.cwd(), "../cta");
const linesPath = path.join(WORKER, "public/lines.geojson");
const dataPath = path.join(WORKER, "public/ctaData.js");

// route property in geojson -> CTA API route key (they already match)
const GEO_ROUTE = { red: "red", blue: "blue", brn: "brn", g: "g", org: "org", p: "p", pink: "pink", y: "y" };
// ctaData boolean column -> CTA API route key
const FLAG_ROUTE = { RED: "red", BLUE: "blue", G: "g", BRN: "brn", P: "p", Y: "y", Pnk: "pink", O: "org" };

// Equirectangular arc length of a [lon,lat] sequence (Chicago-scale planar).
const COS_LAT = 0.743;
const arcLen = (coords) => {
  let d = 0;
  for (let i = 1; i < coords.length; i++) {
    const dx = (coords[i][0] - coords[i - 1][0]) * COS_LAT;
    const dy = coords[i][1] - coords[i - 1][1];
    d += Math.hypot(dx, dy);
  }
  return d;
};

// --- polylines: pick the LONGEST single feature per route ---
// Branched lines (Green) ship as overlapping features that share a trunk and
// fan out; concatenating them folds the rail and scrambles ordering. Using the
// single longest feature keeps the main line monotone with real termini; branch
// stations/trains just project onto the nearest trunk point.
const fc = JSON.parse(fs.readFileSync(linesPath, "utf8"));
const polylines = {};
for (const f of fc.features) {
  const r = GEO_ROUTE[f.properties.route];
  if (!r) continue;
  const coords = f.geometry.type === "LineString" ? f.geometry.coordinates : f.geometry.coordinates.flat();
  if (!polylines[r] || arcLen(coords) > arcLen(polylines[r])) {
    polylines[r] = coords;
  }
}

// --- stations: eval the non-module ctaData.js to get the array ---
const src = fs.readFileSync(dataPath, "utf8");
const ctaStops = new Function(`${src}\n;return ctaStops;`)();

const parseLoc = (s) => {
  const m = /\(([-0-9.]+),\s*([-0-9.]+)\)/.exec(s || "");
  return m ? { lat: +m[1], lon: +m[2] } : null;
};

const stations = {}; // route -> Map(name -> {name,lat,lon})
for (const s of ctaStops) {
  const loc = parseLoc(s.Location);
  if (!loc) continue;
  for (const [flag, route] of Object.entries(FLAG_ROUTE)) {
    if (s[flag]) {
      (stations[route] ||= new Map()).set(s.STATION_NAME, { name: s.STATION_NAME, ...loc });
    }
  }
}

const out = {};
for (const route of Object.keys(polylines)) {
  out[route] = {
    polyline: polylines[route].map(([lon, lat]) => [+lon.toFixed(5), +lat.toFixed(5)]),
    stations: [...(stations[route]?.values() || [])].map((s) => ({
      name: s.name,
      lat: +s.lat.toFixed(5),
      lon: +s.lon.toFixed(5),
    })),
  };
}

const dest = path.resolve(process.cwd(), "src/track.json");
fs.writeFileSync(dest, JSON.stringify(out));
for (const r of Object.keys(out)) {
  console.log(`${r.padEnd(5)} polyline=${out[r].polyline.length}  stations=${out[r].stations.length}`);
}
console.log(`wrote ${dest} (${fs.statSync(dest).size} bytes)`);
