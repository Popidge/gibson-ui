Performance and code quality review, September 4, 2026.

The branch is `codex/performance-quality-pass`. The baseline is commit `ff32115`.
The largest measured reductions affect text preparation and label selection. Whole-frame gains are smaller because GPU work dominates these scenes.

Measurements used a Ryzen 5 PRO 4650U, integrated Radeon graphics, Mesa RADV 26.2.1, Rust 1.94.0, and the release build.
The fullscreen surface was 1920 × 1080. The tests used isolated settings and the repository directory as the starting location.
Both builds used the same GPU timing instrumentation. Kernel sampling with `perf` was unavailable on this machine.

The CPU microbenchmark creates 256 directories and measures repeated operations after initial preparation:

| Operation | Before | After | Result |
|---|---:|---:|---|
| Scene generation | 27.295 µs | 28.288 µs | No demonstrated improvement |
| Select 26 tower labels | 11.684 µs | 5.861 µs | 50% less time |
| Unchanged navigator snapshot | 6.063 µs | 0.011 µs | More than 99% less time |

These figures describe individual operations, not whole-frame speedups. The benchmark has no timing assertions because scheduling and power management affect results.

The graphics comparison used high quality, uncapped presentation, a stationary camera, and disabled lightning, floor pulses, and audio.
Each run lasted 18 seconds. The table uses the mean of the last four log records.
Classic mode used 801 objects and 26 labels. Film mode used 80 objects and 26 labels.

| Measurement | Before | After |
|---|---:|---:|
| Classic FPS | 730 | 755 |
| Film FPS | 497 | 520 |
| Classic text preparation | 0.417 ms | 0.002 ms |
| Film text preparation | 0.442 ms | 0.004 ms |
| Film GPU scene copy | 0.253 ms | 0.173 ms |

A second film comparison reversed the run order. FPS changed from 474 to 552, and GPU copy time changed from 0.333 ms to 0.169 ms.
Thus, film FPS improved by 5–16% across these short comparisons. Classic FPS improved by about 3%.
These results show substantial run variance. They do not establish a universal FPS gain or a battery-life improvement.

A separate film comparison enabled the idle camera sweep. Text preparation changed from 0.447 ms to 0.009 ms, with FPS changing from 497 to 532.
Text preparation includes glyph preparation. It does not include all application logic or surface acquisition.
The `frame_cpu_ms` field includes surface acquisition and presentation overhead. GPU timestamp results measure GPU execution separately.

The performance changes are:

- Navigator snapshots reuse an `Arc` until selection, directory contents, preview, focus, or dimensions change.
- Label selection partitions the candidate set before it sorts the visible labels. Current-tower priority and distance order remain intact.
- UI glyph preparation reuses unchanged content. Layout, theme, cursor, content, and offscreen glyph preparation invalidate the cache.
- Metadata text uses pixel-aligned coordinates. Subpixel camera movement no longer forces glyph preparation on every frame.
- Film mode copies the scene texture directly when the surface supports `COPY_DST`. Other surfaces retain the shader copy.
- Surface acquisition precedes renderer preparation. Skipped frames bypass that work and no longer inflate the FPS count.
- GPU pass timing uses asynchronous readback, at most ten samples per second, only while performance logging is enabled.

The robustness and cleanup changes are:

- Terminal snapshots capture their revision while the parser lock remains held. Previously, a concurrent update could assign a newer revision to older content.
- Both color readers share one parser. Non-ASCII values return an error result before byte slicing, which prevents a Unicode boundary panic.
- Classic file-change effects can reach the settled renderer. The previous nested conditions made this documented feature unreachable.
- The unreachable slab renderer, its state helper, and its unused hidden-file field are gone.
- The unused overlay menu branch, one-variant preview command enum, duplicate label-slot vector, and redundant theme argument are gone.
- Scene storage reserves the requested capacity correctly. Decorative geometry stops at the budget, and instance classification uses one pass.

Existing helpers remain where they describe a useful geometry or I/O operation. A single call site alone does not justify a larger caller.
The review did not justify a general scene cache or lower visual quality. Scene generation is already small, and additional invalidation rules introduce maintenance costs.

Validation passed with 79 tests and one explicitly ignored benchmark. Formatting, Clippy with warnings denied, and the frozen release build passed.
New regression tests cover navigator invalidation, label budgets, Unicode colors, pixel-aligned metadata, and visible file-effect expiration.
Existing PTY tests cover output caching and shell navigation.

The graphical smoke sequence exercised terminal output, navigator selection, orbit, settings changes, pane visibility, and fullscreen resizing.
Screenshots showed readable text and the expected glass scene. The tested runs reported no renderer validation errors.
Only the available Radeon device received graphical validation. Devices without timestamp support or surface copy support retain fallback behavior but lack hardware validation here.
Long sessions, network filesystems, and GPU device loss remain outside this measurement set.

For a CPU comparison, run:

```bash
cargo test --frozen --release profile_hot_paths -- --ignored --nocapture
```

For renderer measurements, run:

```bash
cargo run --frozen --release -- --uncapped --perf /path/to/directory
```

For comparable runs, use the same directory, surface size, settings, and power conditions. Exclude startup records.
GPU logs contain zero values for passes that did not run. Direct-copy timing also requires encoder timestamp support.
