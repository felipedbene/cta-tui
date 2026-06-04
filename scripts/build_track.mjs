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
// fan out from a shared trunk to two south termini. We keep EACH feature as a
// branch (a full line from the common terminus to its own end), so the map can
// draw both. Stations are assigned to a branch by proximity to its polyline, so
// trunk stations appear on both branches and branch-tail stations only on theirs.
const fc = JSON.parse(fs.readFileSync(linesPath, "utf8"));
const branchPolys = {}; // route -> [coords, ...] (one per feature, longest first)
for (const f of fc.features) {
  const r = GEO_ROUTE[f.properties.route];
  if (!r) continue;
  const coords = f.geometry.type === "LineString" ? f.geometry.coordinates : f.geometry.coordinates.flat();
  (branchPolys[r] ||= []).push(coords);
}
for (const r of Object.keys(branchPolys)) {
  branchPolys[r].sort((a, b) => arcLen(b) - arcLen(a)); // primary (longest) first
}

// Planar point→polyline distance (deg, equirectangular) for station assignment.
const distToPoly = (lon, lat, coords) => {
  const px = lon * COS_LAT, py = lat;
  let best = Infinity;
  for (let i = 1; i < coords.length; i++) {
    const ax = coords[i - 1][0] * COS_LAT, ay = coords[i - 1][1];
    const bx = coords[i][0] * COS_LAT, by = coords[i][1];
    const dx = bx - ax, dy = by - ay;
    const seg2 = dx * dx + dy * dy;
    const t = seg2 <= 1e-18 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / seg2));
    const cx = ax + t * dx, cy = ay + t * dy;
    best = Math.min(best, Math.hypot(px - cx, py - cy));
  }
  return best;
};
const NEAR = 0.0015; // ~120m: a station is "on" a branch if within this

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

const round = (s) => ({ name: s.name, lat: +s.lat.toFixed(5), lon: +s.lon.toFixed(5) });

const out = {}; // route -> [ {polyline, stations}, ... ] (branches)
for (const route of Object.keys(branchPolys)) {
  const polys = branchPolys[route];
  const stns = [...(stations[route]?.values() || [])];
  out[route] = polys.map((coords) => {
    // Single-branch routes take all stations; branched routes assign by proximity.
    const onBranch = polys.length === 1 ? stns : stns.filter((s) => distToPoly(s.lon, s.lat, coords) <= NEAR);
    return {
      polyline: coords.map(([lon, lat]) => [+lon.toFixed(5), +lat.toFixed(5)]),
      stations: onBranch.map(round),
    };
  });
}

const dest = path.resolve(process.cwd(), "src/track.json");
fs.writeFileSync(dest, JSON.stringify(out));
for (const r of Object.keys(out)) {
  const b = out[r].map((br) => `${br.polyline.length}/${br.stations.length}st`).join("  ");
  console.log(`${r.padEnd(5)} ${out[r].length} branch(es): ${b}`);
}
console.log(`wrote ${dest} (${fs.statSync(dest).size} bytes)`);
