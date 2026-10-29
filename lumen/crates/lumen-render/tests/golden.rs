//! Golden-image tests.
//!
//! Every case under `tests/golden/cases` is rendered by the software oracle and compared with
//! the committed PNG in `tests/golden`. Set `UPDATE_GOLDENS=1` to rewrite them. When a GPU
//! adapter is available (lavapipe on CI, a real card on a workstation) the same scene is
//! rendered with wgpu and compared with the oracle; set `LUMEN_REQUIRE_GPU=1` to turn a
//! missing adapter into a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use lumen_core::project::{demo_project, Project};
use lumen_render::compare::{diff, diff_image, Thresholds};
use lumen_render::gpu::{GpuContext, GpuRenderer};
use lumen_render::scene::{Scene, DEFAULT_MESH_DENSITY};
use lumen_render::software;
use lumen_sources::SourceSet;
use std::path::{Path, PathBuf};

const GOLDEN_W: u32 = 256;
const GOLDEN_H: u32 = 144;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn cases() -> Vec<(String, Project)> {
    let mut cases = vec![("demo".to_string(), demo_project())];
    let dir = golden_dir().join("cases");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    for path in files {
        let name = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".lumen.toml")
            .to_string();
        let project = Project::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(project.is_valid(), "{name}: {:?}", project.validate());
        cases.push((name, project));
    }
    cases
}

fn scenes(project: &Project) -> Vec<Scene> {
    let (mut sources, errors) = SourceSet::open(project, &golden_dir());
    assert!(errors.is_empty(), "{errors:?}");
    sources.poll();
    project
        .outputs
        .iter()
        .map(|o| Scene::build(project, &o.id, &sources, DEFAULT_MESH_DENSITY).unwrap())
        .collect()
}

fn gpu() -> Option<GpuRenderer> {
    match GpuContext::headless() {
        Ok(ctx) => Some(GpuRenderer::new(ctx.device, ctx.queue)),
        Err(e) => {
            assert!(
                std::env::var_os("LUMEN_REQUIRE_GPU").is_none(),
                "LUMEN_REQUIRE_GPU is set but: {e}"
            );
            eprintln!("skipping GPU comparison: {e}");
            None
        }
    }
}

#[test]
fn software_matches_goldens() {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some();
    let mut failures = Vec::new();
    for (name, project) in cases() {
        for scene in scenes(&project) {
            let golden = golden_dir().join(format!("{name}.{}.png", scene.output_id));
            let actual = software::render_at(&scene, GOLDEN_W, GOLDEN_H);
            if update || !golden.exists() {
                actual.save(&golden).unwrap();
                eprintln!("wrote {}", golden.display());
                continue;
            }
            let expected = image::open(&golden).unwrap().to_rgba8();
            let d =
                diff(&expected, &actual, Thresholds::EXACT.tolerance).expect("golden size matches");
            if !Thresholds::EXACT.accepts(&d) {
                actual.save(golden.with_extension("actual.png")).unwrap();
                diff_image(&expected, &actual, Thresholds::EXACT.tolerance)
                    .unwrap()
                    .save(golden.with_extension("diff.png"))
                    .unwrap();
                failures.push(format!("{}: {d:?}", golden.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches (see *.actual.png / *.diff.png):\n{}",
        failures.join("\n")
    );
}

#[test]
fn gpu_matches_software_oracle() {
    let Some(mut gpu) = gpu() else { return };
    let mut failures = Vec::new();
    for (name, project) in cases() {
        for scene in scenes(&project) {
            let oracle = software::render_at(&scene, GOLDEN_W, GOLDEN_H);
            let actual = gpu.render_to_image(&scene, GOLDEN_W, GOLDEN_H).unwrap();
            let d = diff(&oracle, &actual, Thresholds::GPU.tolerance).unwrap();
            eprintln!("{name}.{}: gpu vs software {d:?}", scene.output_id);
            if !Thresholds::GPU.accepts(&d) {
                let base = golden_dir().join(format!("{name}.{}", scene.output_id));
                actual.save(base.with_extension("gpu.actual.png")).unwrap();
                diff_image(&oracle, &actual, Thresholds::GPU.tolerance)
                    .unwrap()
                    .save(base.with_extension("gpu.diff.png"))
                    .unwrap();
                failures.push(format!("{name}.{}: {d:?}", scene.output_id));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "GPU/software mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn gpu_renders_are_stable_across_frames_and_sizes() {
    let Some(mut gpu) = gpu() else { return };
    let project = demo_project();
    let scene = &scenes(&project)[1];
    let a = gpu.render_to_image(scene, 128, 72).unwrap();
    let b = gpu.render_to_image(scene, 128, 72).unwrap();
    assert_eq!(
        a, b,
        "second frame reuses cached textures and must be identical"
    );
    let c = gpu.render_to_image(scene, 300, 50).unwrap();
    assert_eq!(c.dimensions(), (300, 50));
    gpu.retain_sources(|id| id == "grid");
    let d = gpu.render_to_image(scene, 128, 72).unwrap();
    assert_eq!(a, d, "re-uploading an evicted source changes nothing");
    assert_eq!(
        gpu.render_to_image(scene, 0, 0).unwrap().dimensions(),
        (1, 1)
    );
}

#[test]
fn gpu_renders_into_a_foreign_format_view() {
    let Some(mut gpu) = gpu() else { return };
    let project = demo_project();
    let scene = &scenes(&project)[0];
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    gpu.render_to_view(scene, &view, wgpu::TextureFormat::Bgra8UnormSrgb, 64, 32);
    gpu.render_to_view(scene, &view, wgpu::TextureFormat::Bgra8UnormSrgb, 64, 32);
}
