//! Reproducible CPU microbenchmarks: cargo test --release profile_hot_paths -- --ignored --nocapture
use crate::{
    config::VisualStyle, filesystem::scan_directory, navigation::VisualState, navigator::Navigator,
    scene::*,
};
use glam::Vec3;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[test]
#[ignore = "timing benchmark; run explicitly in release mode"]
fn profile_hot_paths() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..256 {
        std::fs::create_dir(directory.path().join(format!("directory-{index:04}"))).unwrap();
    }
    let entries = scan_directory(directory.path()).unwrap();
    let mut scene = Scene::new(directory.path().to_owned());
    scene.update(directory.path().to_owned(), entries.clone(), &[]);
    let mut navigator = Navigator::new(directory.path().to_owned()).unwrap();
    navigator.update(directory.path().to_owned(), entries);
    // Resolve asynchronous preview work before measuring steady-state snapshots.
    std::thread::sleep(Duration::from_millis(100));
    navigator.poll();
    let mut objects = Vec::new();
    let now = Instant::now() + Duration::from_secs(2);
    let context = SceneRenderContext {
        now,
        state: VisualState::Settled,
        camera_eye: Vec3::new(0.0, 4.0, -8.0),
        camera_focus: Vec3::ZERO,
        max_objects: MAX_RENDER_OBJECTS,
        lightning: LightningOptions::OFF,
        visual_style: VisualStyle::Classic,
        palette: ScenePalette {
            primary: [0.0, 0.83, 0.91],
            secondary: [1.0, 0.02, 0.64],
            accent: [0.02, 0.82, 1.0],
            foreground: [0.34, 1.0, 0.95],
            subdued: [0.01, 0.39, 0.75],
            danger: [0.81, 0.03, 0.12],
        },
    };
    for _ in 0..100 {
        scene.write_render_objects(context, &mut objects);
    }
    measure("scene_256", 20_000, || {
        scene.write_render_objects(black_box(context), &mut objects);
        black_box(&objects);
    });
    measure("labels_256", 20_000, || {
        black_box(scene.tower_labels(
            Vec3::NEG_Z,
            context.state,
            context.camera_eye,
            context.camera_focus,
            26,
            VisualStyle::Classic,
        ));
    });
    measure("navigator_steady", 50_000, || {
        black_box(navigator.snapshot(40, 80, true));
    });
}

fn measure(name: &str, iterations: u32, mut operation: impl FnMut()) {
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    eprintln!(
        "BENCH {name} {:.3} us/op ({iterations} iterations)",
        start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
    );
}
