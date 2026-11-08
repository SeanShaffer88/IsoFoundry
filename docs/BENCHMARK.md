# Benchmarks

Measured for the RC pass (2026-11-08) against the budgets in
[ARCHITECTURE.md](ARCHITECTURE.md) §10. Numbers from the release
build of the command-line program on the development VM (Linux);
they show the engines are comfortably inside budget, but the
Windows figures are the ones that count and two rows below are
still owed from a real Windows desktop.

| Metric | Budget | Measured | How |
|---|---|---|---|
| ISO build, 50,000 files | ≤ 1.5× raw copy time | **0.48×** (2.6 s vs 5.3 s copy) | `create-image` on a 50k-file tree vs copying the same tree |
| Checksum throughput | — | **165 MiB/s** | CRC32+MD5+SHA-1+SHA-256 in one pass over a 128 MiB file (0.78 s) |
| Command-line start | — | **~2 ms** | `--version`, averaged over 5 runs |
| Idle CPU | 0% (no polling) | **By construction** | Event-driven message loop; drive arrival via `WM_DEVICECHANGE`; timers are one-shot debounces only |
| First window render | < 50 ms | *Owed — Windows desktop* | Needs a real Windows machine |
| Idle working set | < 15 MB | *Owed — Windows desktop* | Needs a real Windows machine |
| Burn buffer underruns | 0 at max speed, verify on | *Owed — real burner* | Needs burning hardware |

Re-measure before each release: build the release command-line
program, recreate the two fixtures (a 50,000-file tree of small
files; a 128 MiB data file), and repeat the three timed commands
above.
