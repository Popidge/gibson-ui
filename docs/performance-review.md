Performance and code quality review, September 4, 2026.

The branch is `codex/performance-quality-pass`. The baseline is commit `ff32115`.
The largest measured reductions affect text preparation and label selection. Whole-frame gains are smaller because GPU work dominates these scenes.

Measurements used a Ryzen 5 PRO 4650U, integrated Radeon graphics, Mesa RADV 26.2.1, Rust 1.94.0, and the release build.
The fullscreen surface was 1920 × 1080. The tests used isolated settings and the repository directory as the starting location.
Both builds used the same GPU timing instrumentation. The initial pass lacked `perf`. The sampled CPU follow-up below uses the subsequently installed tool.

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

Sampled CPU follow-up

The follow-up compares commit `d74cfd1` with the changes below. Both binaries use optimized release code with debug information and frame pointers:

```bash
cargo rustc --frozen --release -- -C debuginfo=1 -C strip=none -C force-frame-pointers=yes
perf record -p PID -e cycles:u -F 499 --call-graph dwarf,16384 -o /tmp/gibson.data -- sleep 15
perf stat -p PID -e task-clock:u,cycles:u,instructions:u,cache-misses:u -- sleep 15
```

The existing `perf_event_paranoid=2` permits these user-space samples. No kernel restrictions or system settings changed.
Samples cover application threads, including audio and driver workers. Some distribution libraries lack symbols, which limits their call-stack detail.
GPU pass timestamps provide the separate graphics measurements. These CPU samples do not measure GPU execution or kernel work.

Each workload uses fullscreen film mode at 60 FPS, disabled audio, and a stationary scene without lightning or floor pulses.
Sampling starts after six seconds and lasts fifteen seconds. The terminal producer runs in a separate process outside the sampled application.
The idle workload displays a static shell. The terminal workload emits 2,000 colored log lines per second in batches of forty.
The navigation workload changes directory every 400 milliseconds across 32 directories with 64 files each, modifying sixteen files per change.
It exercises shell directory notifications, filesystem watches, navigation, and rendering.

Muted music still decoded and mixed in the baseline. The soundscape now pauses its music stream while disabled, unfocused, or at zero volume.
It resumes from that position when audible again. Existing volume fades remain. The audio backend remains open to support prompt resumption.

Terminal changes previously rebuilt the entire rich-text buffer. The renderer now replaces rows individually and retains shaping for unchanged text and attributes.
Regression coverage compares text, styles, and glyph positions against the previous rich-text path, including Unicode, blank rows, trailing newlines, and row removal.
It also checks that unchanged rows retain their shaping cache.

Two comparisons ran in opposite build order. The table shows their mean totals over each fifteen-second sampling period.

| Workload | CPU time before → after | CPU time reduction | User-space instructions before → after |
|---|---:|---:|---:|
| Idle | 2.271 → 2.182 seconds | 4% | 1.166 → 0.855 billion |
| Terminal output | 5.452 → 3.979 seconds | 27% | 13.455 → 7.598 billion |
| Navigation | 3.120 → 2.855 seconds | 9% | 3.848 → 2.909 billion |

Terminal output used 43–44% fewer instructions in both comparisons. Its mean logged frame CPU time fell from 3.96 to 2.54 milliseconds.
All workloads stayed near the configured 60 FPS. These measurements establish CPU savings, not an uncapped FPS gain or a battery-life result.
The small idle CPU-time change illustrates the remaining rendering and backend costs despite 27% fewer user-space instructions.

In the initial terminal sample, font metrics accounted for 11% of sampled cycles and attribute lookup accounted for 9%.
After the changes, the first comparison showed about 5% for each, against a smaller total cycle count.
Glyph preparation remains a prominent navigation cost. A broader rendering-cache change lacks sufficient evidence to justify its additional invalidation rules here.
User-space sampling found actionable changes without relaxed kernel restrictions. Kernel behavior remains unmeasured.

The audio integration check requires a live output device and remains explicitly ignored during normal test runs:

```bash
cargo test --frozen muted_music_stops_advancing_and_resumes -- --ignored --nocapture
```

It checks disabled audio, lost focus, and zero volume. Each case verifies paused playback, a stationary stream position, and successful resumption.
Raw samples, generated workloads, isolated settings, and screenshots remain outside the repository under `/tmp`.

Follow-up validation passed: 80 normal tests, the explicit live audio integration check, formatting, strict Clippy, and the frozen release build.
The normal suite skips the timing benchmark and the hardware audio check. Sampling reported no lost events in any of the twelve captures.
The release smoke sequence passed terminal output, selection, orbit, settings, pane visibility, and fullscreen resizing with no logged renderer errors.

Release validation found that the default text buffer gave the first row Advanced shaping while later rows used Basic shaping.
The terminal now starts with an empty buffer, so every row uses Basic shaping.
The regression test uses the bundled proportional font to expose shaping differences independently of installed system fonts.
