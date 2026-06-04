# CTA Track Grid — TUI

NORAD-style Chicago 'L' board in the terminal. Native Rust + [ratatui].
No Worker proxy needed (native app => no CORS); the API key lives in an env var.

```
╔ ◜ CTA TRACK GRID NORAD COMMAND  TRACKING ═══════════════════ UNCLASS  UPD 14:32:07 ╗
║┌ SYSTEM ──────────────┐┏ GREEN LINE [11 TRK] ←/→ ━━━━━━━━━━━━━━━┓┌ ★ KEDZIE ────────┐║
║│ ● RED   Normal Ser…  │┃ ↓ #002  1m     → 35th… ▸ Ashland/63rd  ┃│  1m ● Harlem/Lake │║
║│ ● GRN   Normal Ser…  │┃ → #003  1m APP → California ▸ Cottage…  ┃│  8m ● Ashland/63rd│║
║│ ● BLUE  Delays       │┃ ← #008 DUE APP → Clark/Lake ▸ Harlem…  ┃│ 22m ● Harlem/Lake │║
║└──────────────────────┘┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛└──────────────────┘║
╚ q  QUIT   r  RESCAN   ←/→  LINE   ↑/↓  SCROLL ═════════════ 87 TRAINS // 8 LINES TRACKED ╝
```

A double-ruled console: classification banner + rotating radar sweep `◜◝◞◟` and a
live clock in the top rule, the key legend + train/line counter in the bottom rule.
The focused line gets a heavy brand-colored frame; APP/DLY flags and any non-normal
system status blink like real annunciators (driven by a 4 fps render tick).

## Run

Get a free key: https://www.transitchicago.com/developers/traintrackerapply/

```sh
CTA_KEY=your_key_here cargo run --release
```

## Config (env vars)

| var              | default                          | meaning                         |
|------------------|----------------------------------|---------------------------------|
| `CTA_KEY`        | —                                | Train Tracker key (required)    |
| `CTA_ROUTES`     | `red,blue,brn,g,org,p,pink,y`    | routes to track                 |
| `CTA_HOME_MAPID` | `41070`                          | home station (Kedzie/Green)     |
| `CTA_HOME_NAME`  | `Kedzie`                         | label for the home panel        |
| `CTA_REFRESH`    | `30`                             | seconds between polls           |
| `CTA_ALERT_MIN`  | `6`                              | bell + flash when a home train is ≤ this many min away (`0` disables) |

## Debug modes (no terminal needed)

```sh
CTA_PROBE=1  CTA_KEY=… cargo run    # one snapshot dumped to stdout (data check)
CTA_RENDER=1 CTA_KEY=… cargo run    # draw one frame off-screen and print as text
                                    #   CTA_COLS / CTA_ROWS size the buffer
```

## Layout

- **left** — system board: the 8 'L' lines + live status (keyless `routes.aspx`).
- **center** — focused line. A **track map** strip on top (fio 4, below) and the
  train list under it: heading arrow, run #, ETA, DLY (amber) / APP (green)
  flags, next stop, terminal dest. `←/→` cycles lines; `↑/↓` moves the train
  cursor — the selected train is highlighted in the list and on the map, and
  its run # shows in the panel title.
- **right** — home-station arrivals ticker (`ttarrivals.aspx`).

## Track map (fio 4) — `src/track.rs` + `scripts/build_track.mjs`

The center strip is a NORAD-style line diagram: a rail of evenly-spaced station
ticks `┿`, `◆` termini, the home station starred `★` and labeled, and every live
train projected onto it. Travel direction comes from the compass heading dotted
with the local rail tangent: rightward trains ride the upper rail (`▸`/`▶`),
leftward the lower (`◂`/`◀`); a filled arrowhead means approaching.

Geometry is baked at build time by `scripts/build_track.mjs`, which reads the
Worker repo's `public/lines.geojson` (per-route rail polyline) and
`public/ctaData.js` (station names + lat/lon) into `src/track.json`. At runtime
each station and train lat/lon is projected to a 0..1 position along the rail;
positions are then warped into evenly-spaced *station space* so the dense
downtown stretch stays legible.

```sh
node scripts/build_track.mjs ../cta   # regenerate src/track.json from the Worker
```

The rail is drawn in a dimmed brand color with brighter ticks. **Landmark**
stations — major downtown/transfer anchors (Clark/Lake, Roosevelt, Fullerton,
Belmont, Howard, …) — are marked `◈` and labeled (a two-row packer drops any
label that would collide), so the line is navigable between its termini.

Press `/` to **fuzzy-find** any station across all 8 lines; selecting one jumps
to that line and **zooms** the map to a ~9-station window centered on it, with
every station labeled and trains in-window placed (`«`/`»` count the rest).

Caveat: branched lines (Green) ship as overlapping geojson features; the build
script keeps the **longest** one as the main line (so termini are real — Green
is Harlem/Lake ↔ Ashland/63rd), and the other branch's stations/trains project
onto the nearest trunk point. Single-feature lines (Red, Blue, …) are exact.

## Data layer (`src/cta.rs`)

Three feeds folded into one `Snapshot` per poll:
- `ttpositions.aspx` — live train positions (key)
- `ttarrivals.aspx`  — arrivals at home station (key)
- `routes.aspx`      — system status (keyless, filtered to the 8 rail lines)

CTA JSON collapses single-element arrays into bare objects; `OneOrMany<T>`
normalizes that everywhere.

## Roadmap (next fios)

- **fio 2** — Green Line alerts panel (`alerts.aspx?routeid=g`, has CDATA noise).
- **fio 3** — ✅ done: home-station approach notifier (bell + flashing arrivals
  panel when a train is within `CTA_ALERT_MIN`).
- **fio 4** — ✅ done: ASCII track map (see above).
- **fio 5** — desktop notification on delay, via `notify-rust`.
- per-branch track maps for the Green/branched lines (show both south termini).
- optional vertical track orientation for tall/narrow terminals.

[ratatui]: https://ratatui.rs
