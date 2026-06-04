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

## Debug modes (no terminal needed)

```sh
CTA_PROBE=1  CTA_KEY=… cargo run    # one snapshot dumped to stdout (data check)
CTA_RENDER=1 CTA_KEY=… cargo run    # draw one frame off-screen and print as text
                                    #   CTA_COLS / CTA_ROWS size the buffer
```

## Layout

- **left** — system board: every line + live status (keyless `routes.aspx`).
- **center** — focused line's trains: heading arrow, run #, ETA, next stop,
  DLY (amber) / APP (green) flags. `←/→` cycles lines.
- **right** — home-station arrivals ticker (`ttarrivals.aspx`).

## Data layer (`src/cta.rs`)

Three feeds folded into one `Snapshot` per poll:
- `ttpositions.aspx` — live train positions (key)
- `ttarrivals.aspx`  — arrivals at home station (key)
- `routes.aspx`      — system status (keyless)

CTA JSON collapses single-element arrays into bare objects; `OneOrMany<T>`
normalizes that everywhere. `lat`/`lon`/`heading` are kept on `Train` for the
next fio.

## Roadmap (next fios)

- **fio 2** — Green Line alerts panel (`alerts.aspx?routeid=g`, has CDATA noise).
- **fio 3** — Kedzie approach notifier: bell + flash when a Green train ≤6 min.
- **fio 4** — ASCII track diagram per line using `lat`/`lon` projected onto a
  precomputed station sequence (the real "NORAD map" in characters).
- **fio 5** — desktop notification on delay, via `notify-rust`.

[ratatui]: https://ratatui.rs
