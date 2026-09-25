# iRacing Pit Crew Coach (Rust)

A local practice coach for iRacing. Record a stint live, or open an `.ibt` telemetry file, to get:

- lap times and sectors;
- a track map with turn numbers;
- corner-by-corner comparisons;
- telemetry overlays, including lateral/longitudinal g and understeer/oversteer balance;
- a grip circle (g-g plot) showing how much of the tyres' grip each lap and corner used;
- per-corner grip use, balance and ABS use (for cars and files that record `BrakeABSactive`);
- written feedback from a local Ollama model.

## Requirements

- Rust toolchain
- [Ollama](https://ollama.com) on `PATH` for the written coach feedback (optional: without it you get a short fallback note). The model defaults to `llama3.2`. Change it with `--model` or the ⚙ menu in the UI.
- Optional, CLI only: `espeak-ng` or `espeak` on `PATH` for `--talk`
- **Live recording only:** Windows, on the same PC as iRacing. Everything else also works on macOS/Linux: the UI, `.ibt` files, replay and tests.

## Quick start

```sh
cargo run -- --ui
```

Then open http://127.0.0.1:8787 (`--ui-port` to change the port).

- **Start recording** / **Stop & analyze** records a live run (Windows + iRacing). Laps appear as you cross the line.
- **Open session** lists the `.ibt` files in your telemetry folder, newest first. Turn on telemetry logging in the sim with Alt+L. A session can only be opened after you leave it, because iRacing locks the file while recording.

Runs are kept in memory while the server is running. Restarting it clears the run list, but saved track maps stay.

## Where telemetry is found

iRacing writes `.ibt` files to `Documents\iRacing\telemetry`. The app finds that folder automatically, using the first one that exists:

1. `PCC_TELEMETRY_DIR`, if you set it (use this when Documents lives somewhere unusual, e.g. another drive)
2. `Documents` under your OneDrive folder, via Windows' `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial` variables. This covers renamed OneDrive folders like "OneDrive - Company".
3. `%USERPROFILE%\OneDrive\Documents`
4. `%USERPROFILE%\Documents` (`$HOME/Documents` on macOS/Linux)

If none exists, e.g. on a Mac, **Open session** lists `tests/fixtures` instead. You can type any folder into the **Open session** dialog. The last folder you used is remembered in the browser and takes priority over detection. The CLI takes `--file` / `--dir`.

## Developing without iRacing (e.g. on a Mac)

Replay a fixture as if it were live telemetry. **Start recording** then plays the file at `--replay-speed` × real time (default 4):

```sh
cargo run -- --ui --replay tests/fixtures/roadatlanta-full.ibt --replay-speed 8
```

Run the tests (they use the committed fixtures):

```sh
cargo test
```

## Test data

Real `.ibt` files are **not** committed (`*.ibt` is git-ignored). Their session info includes your name, iRacing ID, club, car setup and every other driver in the session.

Instead, make an anonymized fixture on the Windows PC after leaving the session:

```powershell
cargo run --release -- --make-fixture "path\to\session.ibt" --fixture-laps 4
```

This writes `tests/fixtures/<track>.ibt` (`--fixture-out` to choose the name), about 2 MB for 4 laps. It keeps:

- a run of consecutive clean laps, plus a few seconds either side;
- only the ~20 channels the app uses (speed, inputs, gear, lap distance, heading, GPS, lap times…);
- session info reduced to track name/length, sector boundaries and car. Driver names and IDs, clubs, setup, weather and the session date are dropped, not masked.

GPS in a fixture is the track's own location, which is public.

The exporter reads the result back and prints the lap times so you can sanity-check it. `cargo test` also checks the committed fixture for identifying session fields. Fixtures are the one exception to the `*.ibt` ignore rule, so review what you add.

To compare the heading-based map (what live mode uses) against GPS on any file:

```sh
PCC_IBT=path/to/session.ibt cargo test --release outline -- --ignored --nocapture
```

(PowerShell: `$env:PCC_IBT = "path\to\session.ibt"; cargo test --release outline -- --ignored --nocapture`)

## Reviewing a run

- **Headline stats:**
  - best lap;
  - optimal lap (sum of best sectors), or best 3-lap average when there are no sectors;
  - average and consistency (std dev);
  - trend (last laps vs first laps);
  - the focus sector, where a typical lap loses the most time.
- **Picking laps:** click a lap in the lap strip, pace chart or table. **Shift+click** another lap to compare against it; otherwise it compares against your best, or your next-best when the best is selected. **←/→** steps through laps, **B** jumps to the best lap.
- **Excluding laps:** untick laps in the table (out laps, spins) to leave them out of the stats.

Track map and telemetry (live runs and `.ibt` files):

- **Map:** colored by time gained/lost against the comparison lap, by speed, or by inputs (brake / part throttle / full throttle / coast).
- **Corner by corner:** time Δ, minimum speed and brake point per turn. Click a turn to zoom the traces to it.
- **Traces:** delta, speed, throttle, brake, gear and steering for both laps, with a cursor synced to the map. Drag to zoom, double-click to reset.
- **Timing:** lap times use iRacing's official `LapLastLapTime` when present. Sector times use the track's real sector boundaries from the session info.
- **Map source:** GPS in `.ibt` files. Live telemetry has no GPS, so live maps are rebuilt from heading (`YawNorth`) and lap distance. On real data the two agree to within ~5–7 m.
- **Saved maps:** each track's map is saved to `data/tracks/<track>.json` (git-ignored) and reused later, including sector boundaries for live runs.
- **Turn numbers** are auto-detected from curvature and won't always match official numbering. Use **Rename turns** once per track; the names are saved with the map.

## CLI

```sh
cargo run -- --file path/to/session.ibt       # summarize one .ibt (or a lap CSV like data/practice_session.csv)
cargo run -- --dir path/to/folder --talk      # every .ibt/.csv in a folder, pooled together, spoken
cargo run -- --live --live-duration-sec 30    # Windows + iRacing: sample live, then summarize
```

With no `--file`/`--dir`, the CLI pools every file in your telemetry folder (see above), falling back to `data/practice_session.csv`. `--dir` pools laps from all files into one summary, so it's best pointed at a folder from a single session.

## Project layout

| Path | What it does |
|---|---|
| `src/ibt.rs` | `.ibt` reader and the `--make-fixture` exporter |
| `src/live.rs` | Live iRacing capture (Windows) and `--replay` |
| `src/trace.rs` | Frames → laps, sectors, distance-based traces, track outline, turn detection |
| `src/track.rs` | Saved track maps in `data/tracks/` |
| `src/analysis.rs`, `src/coach.rs` | Run summary, suggestions, Ollama prompt/feedback |
| `src/web.rs` | Local HTTP API for the UI |
| `web/` | The UI (React + htm from a CDN, no build step) |
| `tests/fixtures/` | Anonymized `.ibt` test sessions |

## Notes

- `.ibt` files are read by a small built-in reader that only pulls the session-info fields it needs, so unusual session-metadata layouts don't break an import.
- iRacing's `Lap` counter stays at 0 in some sessions, so laps are detected from the start/finish line crossing and numbered in the order driven when the counter isn't available.
- Tyre temperatures are only shown when they change during a lap. iRacing only refreshes them in the pit stall, so on track they're frozen.
