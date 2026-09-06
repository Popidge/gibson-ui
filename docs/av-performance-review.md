AV performance and code quality review, September 6, 2026.

The branch is `codex/visual-improvements`. The baseline includes the complete AV experiment and front-face filesystem activity effects from this task.
The comparison uses a saved executable from that working tree. It does not compare against the earlier released version.
No package, release, or commit was created during this review.

The main improvement reduces GPU work without reducing effect resolution or removing effects.
The repeat live comparison reduced measured GPU-pass time by 21%. The isolated vertical blur comparison reduced time by approximately 40–45%.

Measurements

The GPU was an AMD Radeon integrated GPU, using RADV RENOIR and Mesa 26.2.1.
Live runs used the debug executable, isolated settings, fullscreen presentation, 1995 Film style, Cinematic quality, and a 60 FPS cap.
The fixture contained 96 child directories. Each run included navigation and filesystem activity, then a settled view with 98 objects and 32 labels.
Reflections were enabled. Lightning, audio, wallpaper, and idle sweep were disabled for repeatability.
Each run lasted 22 seconds. These figures average the last four GPU log records, after the arrival effects ended.

| GPU measurement | Baseline | Optimized | Reduction |
|---|---:|---:|---:|
| Total sampled passes, first comparison | 9.478 ms | 4.654 ms | 51% |
| Total sampled passes, repeat comparison | 6.040 ms | 4.754 ms | 21% |
| Bloom and reflection blur, repeat comparison | 2.406 ms | 1.450 ms | 40% |
| Isolated vertical blur, first test | 0.3122 ms | 0.1702 ms | 45% |
| Isolated vertical blur, validation repeat | 0.3055 ms | 0.1854 ms | 39% |

The baseline runs show substantial variation. GPU clocks and other desktop work affect these short measurements.
The repeat comparison is the more conservative result. Both builds remained close to the 60 FPS cap.
These results do not establish an uncapped FPS gain, battery-life improvement, or equivalent gains on other GPUs.
Total sampled passes excludes GPU work outside the measured passes. It is not presentation latency.

The isolated test renders a 960 × 540 texture containing fine patterns, sharp edges, and isolated highlights.
It compares the original 17-sample Gaussian with the paired implementation, using GPU timestamps after warm-up.
The maximum pixel difference was 1/255. The mean absolute difference was 0.0109/255 across RGBA channels.
This comparison uses an RGBA8 target. Live Cinematic bloom uses RGBA16Float targets.

CPU microbenchmarks use optimized builds and 256 directories.

| Operation | Before | After |
|---|---:|---:|
| Classic scene generation | 26.904 µs | 24.875 µs |
| Tower label selection | 6.164 µs | 5.358 µs |
| Unchanged navigator snapshot | 0.012 µs | 0.012 µs |
| Cinematic scene generation with maximum-load lightning | Not measured | 11.246 µs |
| Active filesystem uniforms | Not measured | 0.084 µs |

The small CPU differences do not demonstrate a material improvement. Those paths already consume little frame time.
A separate optimized build with symbols supplied 870 CPU samples through `perf`, with no lost samples.
Scene construction, tower bands, lightning geometry, and label sorting dominated this synthetic workload. It did not expose a larger new CPU bottleneck.
The sample percentages describe the benchmark workload, not the complete application.

Changes retained

- Bloom and reflection blur process the visualiser plus filter padding, instead of the entire window.
- Scissor bounds use actual texture dimensions, including odd window sizes and split-pane offsets.
- Vertical bloom and reflection blur use nine bilinear samples for the existing 17-sample Gaussian.
- Horizontal downsampling and fractional reflection blur retain their original kernels.
- Cinematic glass skips the ordinary glass material calculation that it replaces.
- Shaders skip inactive lightning sources and disabled floor reflections.
- Activity uniforms reuse the existing bounded history order, removing a per-frame allocation and sort.
- Scene, label, and bloom shaders share one uniform definition in `uniforms.wgsl`.
- Rust activity discriminants explicitly match the shader values, replacing repeated conversion matches.

The code review covered rendering, scene effects, audio envelopes, navigator caching, filesystem events, and topology workers.
Existing bounds on geometry, activity history, previews, and topology remain in place.
The review did not justify a new scene cache, wider architectural refactoring, or reduced visual quality.
Small helpers remain where they express a rendering operation or support regression tests. Single use alone does not make inlining clearer.
The new GPU comparison stays separate from production rendering code.

Validation

All 91 regular tests passed with all features enabled. Clippy passed for all targets and features with warnings denied.
Formatting and whitespace checks passed. Optimized benchmark builds and the local build with Mixkit audio completed.
Three explicit GPU tests passed: shader validation, arrival occlusion, and paired-blur pixel comparison.
The new scissor test covers odd pane dimensions and a one-pixel target.

Live startup checks passed for Performance, Balanced, High, and Cinematic with reflections disabled.
The profiling runs also exercised Cinematic reflections, navigation, and filesystem activity without renderer validation errors.
These startup checks validate rendering on this GPU; they do not replace visual inspection of every preset.
The live audio-device test was not run during this review. Existing audio data tests passed, including the arrival envelope check.

To repeat the CPU benchmark:

```bash
cargo test --frozen --release profile_hot_paths -- --ignored --nocapture
```

To repeat the isolated GPU comparison:

```bash
cargo test --frozen paired_blur_matches_reference_on_gpu -- --ignored --nocapture
```

To run the application with GPU timing logs:

```bash
cargo run --frozen --features mixkit-audio -- --perf /path/to/directory
```

Use the same settings, window size, directory contents, and power conditions for comparisons. Exclude startup and compare repeated runs.
Local raw logs and the saved baseline executable are in `/tmp/gibson-perf-review`. They are temporary review artifacts.
