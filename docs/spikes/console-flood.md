# Spike: console flood

Question: what happens to the controller when a game server (or a plugin gone wrong) floods its console, and when browsers watching the console are slow or frozen? Audience: people changing `src/console/`.

## How the console path works

One tailer per running server reads `console.log` in 256 KiB chunks, splits it into lines and pushes each line into (a) a ring buffer of the last 1500 lines (served by `/console/backlog`) and (b) a `broadcast` channel (512 slots) that every WebSocket viewer subscribes to. Polling is 120 ms while output flows, and a full chunk is followed by an immediate re-read, so the tailer is not rate limited by the poll interval.

## Defects found by reading the code, and fixes

These are from analysis; the old code was not benchmarked, and the fixes are covered by tests below.

| Problem | Effect | Fix |
| --- | --- | --- |
| No cap on line length (the 64 KiB check only ran after a chunk's complete lines had been split) | A single 256 KiB line, times the 1500-line ring, is about 400 MB resident, plus one copy per broadcast slot and per browser | Lines are cut at 8 KiB with a `[line cut by Moss: N more bytes]` marker (`console/lines.rs`); the remainder is discarded as it streams in, so memory is bounded by 8 KiB regardless of line length |
| Line splitting was `find('\n')` plus `drain(..=i)` on one growing `String` | Each line removes bytes from the front of a 256 KiB buffer: quadratic per chunk | Single linear pass over the bytes, no front removal |
| A viewer's `send().await` had no timeout | A frozen browser (full TCP buffer) parked its task forever, holding the socket and its broadcast receiver | 5 s send timeout, then the viewer is dropped (`SEND_TIMEOUT`) |
| A lagging viewer was told "N lines skipped" forever | A permanently slow client kept costing a task and a stream of notices | Five lag events in 30 s disconnects it with a message asking to reload |

Memory bounds now: ring at most 1500 x 8 KiB = 12 MiB per server, broadcast channel 512 x 8 KiB = 4 MiB per server (both worst case, with only maximum-length lines), independent of viewers; slow viewers hold nothing beyond the channel because `broadcast` drops the oldest entries for lagging receivers.

## Measurements

Release build (thin settings: no LTO), AMD Ryzen 5 5500, Linux, data on a local SSD, page cache warm. Reproduce with
`MOSS_FLOOD_MB=200 cargo test --release --bin moss console::flood -- --ignored --nocapture --test-threads=1`.
The tests write the flood to a log file and run the real tailer against it (the file is written faster than it is consumed; the whole file is queued before the tailer finishes).

| Flood | Size | Lines | Tailer time | Throughput | CPU | Ring afterwards | Marker line latency |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Short lines (about 50 bytes) | 210 MB | 3.8 M | 0.7 s | 300 MB/s | 0.6 s | 1500 lines, 79 KB | 238 ms |
| 1 MB lines | 210 MB | 200 | 0.3 s | 733 MB/s | 0.2 s | 200 lines, 1.6 MB | 121 ms |
| One 200 MB line, no newline | 200 MB | 1 | n/a | n/a | n/a | one 8 KiB line | n/a |

Peak resident growth during every run was within measurement noise (the figure is the process high-water mark, so it cannot show a decrease; it stayed at about +1 MB over the baseline). The marker latency is the time for one new line to appear after the flood is drained and is dominated by the 120 to 250 ms poll interval, not by the backlog.

Viewer behaviour (unit tests, `console/flood_tests.rs`, no real sockets):

- A viewer whose `send` never completes is dropped after the send timeout (test uses 150 ms; production 5 s).
- A viewer that overruns the broadcast buffer is told how many lines were skipped, and is disconnected after five such events within 30 seconds.
- The broadcast channel never holds more than its 512 slots however much is written while a receiver is stuck; a fast viewer next to a slow one still receives every line.

## Not measured

- Real WebSocket and TCP behaviour with a frozen browser (the sink is a test double; the production `Sink` is the axum `WebSocket`, whose `send` blocks on the socket buffer, which is what the timeout guards).
- The game server's own stall when its stdout pipe is full: the log is a file, so the game never blocks on Moss.
- Disk-full while the game writes a flood (the game's problem; the low-disk notification warns before it).
- A browser rendering millions of lines: the UI keeps a bounded window, but this was not profiled here.
