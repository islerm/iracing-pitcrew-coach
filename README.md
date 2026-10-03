# iRacing Pit Crew Coach (Rust)

A local practice coach for iRacing. Record a stint live, or open an `.ibt` telemetry file, to get:

- lap times and sectors, with laps that went off track marked `!` (and incident points, from `PlayerTrackSurface` / `PlayerCarMyIncidentCount`);
- a track map with turn numbers; hover it to see where the comparison lap was at the same moment;
- corner-by-corner comparisons: click a turn for an entry/exit breakdown against the comparison lap or your typical lap, and ask the coach model about that corner;
- telemetry overlays next to the track map, including RPM, elevation (`.ibt` only), lateral/longitudinal g and understeer/oversteer balance;
- a grip circle (g-g plot) showing how much of the tyres' grip each lap and corner used, with a built-in guide to reading it;
- upshift analysis against the car's shift light and redline (early/late shifts, time on the rev limiter), and which gear was quicker through each corner;
- elevation-aware corner notes (downhill braking zones, crests, compressions) for the corner coach;
- per-corner grip use, balance and ABS use (for cars and files that record `BrakeABSactive`);
- written feedback from a local Ollama model.

## Requirements

- Rust toolchain
- [Ollama](https://ollama.com) on `PATH` for the written coach feedback (optional: without it you get a short fallback note). The model defaults to `llama3.2`. Change it with `--model` or the ⚙ menu in the UI.
- Optional, for the coach's voice: [uv](https://docs.astral.sh/uv/) plus a voice model. Run `scripts/setup-voice.sh` (or `scripts/setup-voice.ps1` on Windows) for the default Piper voice, a British female (`en_GB-cori-high`, ~110 MB in `data/voice/piper/`). Pick the engine with `--tts piper|kokoro|espeak`: `scripts/setup-voice.sh kokoro` sets up Kokoro (more natural, slower, ~340 MB). Other voices go through `--voice`. If the chosen engine isn't set up, the voice falls back to `espeak-ng`/`espeak` when installed. Used by the UI's play buttons.
- **Live recording only:** Windows, on the same PC as iRacing. Everything else also works on macOS/Linux: the UI, `.ibt` files, replay and tests.

## Quick start

```sh
cargo run
```

Then open http://127.0.0.1:8787 (`--ui-port` to change the port). The server finds `web/`, `voice/` and `data/` relative to the project folder, so a built binary also works when started from elsewhere as long as it sits inside the project (e.g. `target/release/`).

- **Start recording** / **Stop & analyze** records a live run (Windows + iRacing). Laps appear as you cross the line.
- **Open session** lists the `.ibt` files in your telemetry folder, newest first. Turn on telemetry logging in the sim with Alt+L. A session can only be opened after you leave it, because iRacing locks the file while recording.

Runs are kept in memory while the server is running. Restarting it clears the run list, but saved track maps stay.

## Where telemetry is found

iRacing writes `.ibt` files to `Documents\iRacing\telemetry`. The app finds that folder automatically, using the first one that exists:

1. `PCC_TELEMETRY_DIR`, if you set it (use this when Documents lives somewhere unusual, e.g. another drive)
2. `Documents` under your OneDrive folder, via Windows' `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial` variables. This covers renamed OneDrive folders like "OneDrive - Company".
3. `%USERPROFILE%\OneDrive\Documents`
4. `%USERPROFILE%\Documents` (`$HOME/Documents` on macOS/Linux)

If none exists, e.g. on a Mac, **Open session** lists `tests/fixtures` instead. You can type any folder into the **Open session** dialog. The last folder you used is remembered in the browser and takes priority over detection.

## Developing without iRacing (e.g. on a Mac)

Replay a fixture as if it were live telemetry. **Start recording** then plays the file at `--replay-speed` × real time (default 4):

```sh
cargo run -- --replay tests/fixtures/roadatlanta-full.ibt --replay-speed 8
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

## Options

```sh
cargo run -- --ui-port 8790                   # serve on another port (default 8787)
cargo run -- --model qwen2.5:7b               # Ollama model for the coach (also in the ⚙ menu)
cargo run -- --tts kokoro --voice bf_isabella # voice engine and voice (default: piper, en_GB-cori-high)
```

## Project layout

| Path | What it does |
|---|---|
| `src/telemetry/` | Reading data into frames, laps and traces: `ibt.rs` (`.ibt` reader, `--make-fixture` exporter), `live.rs` (live iRacing capture on Windows, `--replay`), `trace.rs` (laps, sectors, distance-based traces, track outline, turn detection), `balance.rs` (understeer/oversteer balance), `track.rs` (saved track maps in `data/tracks/`), `weather.rs` (session conditions) |
| `src/analysis/` | Turning laps and traces into findings: `mod.rs` (run summary and suggestions), `corner.rs` (per-corner entry/exit metrics), `handling.rs` (handling and grip notes), `gears.rs` (upshift analysis) |
| `src/coach/` | The language model: `mod.rs` runs it through Ollama, `prompt.rs` builds the prompts |
| `src/voice/` | The coach's voice: `mod.rs` (engine and sidecar handling), `speech.rs` (text to spoken form), `radio.rs` (WAV and the radio effect) |
| `src/server/` | Local HTTP API for the UI: `mod.rs` (router, shared state), `splits.rs`, `recording.rs`, `files.rs`, `voice.rs` |
| `src/stats.rs` | Shared maths (median, percentile, mean) used by telemetry and analysis |
| `src/cli.rs`, `src/main.rs` | Command-line options, startup |
| `voice/` | Piper and Kokoro text-to-speech sidecars (Python, run through `uv`) |
| `web/` | The UI (React + htm from a CDN, no build step) |
| `scripts/` | Voice model setup |
| `tests/fixtures/` | Anonymized `.ibt` test sessions |

## Notes

- `.ibt` files are read by a small built-in reader that only pulls the session-info fields it needs, so unusual session-metadata layouts don't break an import.
- iRacing's `Lap` counter stays at 0 in some sessions, so laps are detected from the start/finish line crossing and numbered in the order driven when the counter isn't available.
