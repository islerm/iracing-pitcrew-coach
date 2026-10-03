# iRacing Pit Crew Coach

Rust crate (`src/`) that parses iRacing telemetry (`.ibt`) and serves a coaching web UI (`web/`, plain JS/CSS/HTML). Test fixtures live in `tests/fixtures/`.

## Delegation policy

The main session (Opus) plans and owns the result. Hand off work to subagents when it fits them; you are explicitly allowed to spawn these agents without being asked each time.

- **Opus (yourself)** — understanding the request, designing the approach, splitting it into tasks, anything subtle (telemetry math, corner/handling analysis logic, cross-module refactors, debugging a failure a subagent couldn't fix), and reviewing subagent output before telling the user it's done. If a task is small enough that writing the handoff would take as long as doing it, just do it.
- **`implementer` (Sonnet)** — a scoped coding task once the plan is decided. Give it: the goal, the files to touch, the approach, and how to verify. Independent tasks can run in parallel.
- **`scout` (Haiku)** — searches, summarizing code, running `cargo build`/`cargo test`/`cargo clippy` and reporting results, inspecting fixtures, trivial mechanical edits.

Workflow for non-trivial requests:
1. Explore (use `scout` for broad searches) and write a short plan with numbered tasks, each tagged with who runs it.
2. Dispatch tasks; keep the ones that need judgment for yourself.
3. Review the diffs that come back, then have `scout` run the full test suite before reporting.
