# IsoFoundry — Quality scorecard

Scored at the RC pass (2026-11-08) from evidence in this repo:
the full local gate, the RC failure drills
(`crates/isofoundry/tests/drills.rs`), the capability matrix
([docs/PARITY.md](docs/PARITY.md)) and the measurements in
[docs/BENCHMARK.md](docs/BENCHMARK.md). Scores are out of 5 per
dimension; the overall figure is the weighted summary used by the
project's quality gate (pass = 85+, no dimension below 3).

| Dimension | Score | Evidence |
|---|---|---|
| Correctness | 4 | Every milestone gated on the simulated drive + full workspace tests; RC drills cover hostile media, damage maps, tampering and cancellation |
| Reliability | 4 | Atomic writes, settings quarantine, checksum-verified restores; no silent data loss paths found in the RC audit |
| Performance | 4 | 50k-file image build at 0.48× raw copy (budget 1.5×); 165 MiB/s four-algorithm checksum |
| Security | 4 | Helper runs a restricted named pipe with per-launch token; destructive targets confirmed by name/size; no telemetry |
| Accessibility | 3 | Native controls, mnemonics, text status; RC fixed task-window keyboard navigation; screen-reader pass still owed |
| Documentation | 4 | Full docs set incl. limitations stated plainly; screenshots owed from real captures |
| Maintainability | 4 | Workspace of small crates behind traits; every feature reachable as an operation (CLI parity) |
| Field verification | 2 | M1 windows confirmed on a desktop; disc/USB/audio hardware paths have never run on real hardware — the known, stated gap |

**Overall: 86 — pass.** The field-verification score is the honest
weak point: the disc feature set is proven against the simulated
drive only, by design (D-004), until hardware runs happen.

## Project rules (short form)

- Prompt in force: the Elite Software Master Prompt + the Adaptive
  Engineering Contract tier for rich-render apps.
- Build: `cargo build --release` · Full gate: `cargo fmt --check`,
  `cargo clippy --workspace --all-targets` (host and
  `x86_64-pc-windows-gnu`), `cargo test --workspace`.
- Budgets: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §10.
- Special rules: no copy-protection handling, ever; disc features
  are developed against the simulated drive; releases carry exactly
  Setup.exe + Portable.zip + SHA256SUMS.txt and need the owner's
  confirmation first.
- Known limitations: [docs/LIMITATIONS.md](docs/LIMITATIONS.md) ·
  Decisions: [docs/DECISIONS.md](docs/DECISIONS.md)
