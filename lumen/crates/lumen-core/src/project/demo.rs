//! The two-projector demo project.

use super::{
    BlendMode, ColorAdjust, Crop, Output, Project, Source, SourceKind, Surface, TestPattern,
};
use crate::blend::EdgeBlend;
use crate::mask::Mask;
use crate::warp::WarpKind;

/// A small two-projector demo used by the examples, the CLI tests and the golden images.
#[allow(clippy::too_many_lines)]
pub fn demo_project() -> Project {
    use crate::blend::{BlendCurve, BlendEdge};
    use crate::geom::Vec2;
    use crate::warp::Quad;
    let blend = |left: f64, right: f64| EdgeBlend {
        left: BlendEdge {
            width: left,
            curve: BlendCurve::Smooth,
        },
        right: BlendEdge {
            width: right,
            curve: BlendCurve::Smooth,
        },
        gamma: 2.2,
        ..EdgeBlend::default()
    };
    Project {
        name: "Two projector demo".into(),
        sources: vec![
            Source {
                id: "grid".into(),
                kind: SourceKind::TestPattern {
                    pattern: TestPattern::Grid,
                },
            },
            Source {
                id: "bars".into(),
                kind: SourceKind::TestPattern {
                    pattern: TestPattern::Bars,
                },
            },
        ],
        outputs: vec![
            Output {
                id: "left".into(),
                width: 1920,
                height: 1080,
                enabled: true,
                blend: blend(0.0, 0.2),
                color: ColorAdjust::default(),
                placement: None,
            },
            Output {
                id: "right".into(),
                width: 1920,
                height: 1080,
                enabled: true,
                blend: blend(0.2, 0.0),
                color: ColorAdjust::default(),
                placement: None,
            },
        ],
        surfaces: vec![
            Surface {
                id: "wall-left".into(),
                output: "left".into(),
                source: "grid".into(),
                z: 0,
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                crop: Crop {
                    x: 0.0,
                    y: 0.0,
                    w: 0.6,
                    h: 1.0,
                },
                warp: WarpKind::Quad(Quad {
                    corners: [
                        Vec2::new(0.05, 0.08),
                        Vec2::new(0.98, 0.04),
                        Vec2::new(0.96, 0.95),
                        Vec2::new(0.03, 0.9),
                    ],
                }),
                masks: Vec::new(),
                locked: false,
            },
            Surface {
                id: "wall-right".into(),
                output: "right".into(),
                source: "grid".into(),
                z: 0,
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                crop: Crop {
                    x: 0.4,
                    y: 0.0,
                    w: 0.6,
                    h: 1.0,
                },
                warp: WarpKind::default(),
                masks: Vec::new(),
                locked: false,
            },
            Surface {
                id: "logo".into(),
                output: "right".into(),
                source: "bars".into(),
                z: 1,
                visible: true,
                opacity: 0.8,
                blend_mode: BlendMode::Screen,
                crop: Crop::default(),
                warp: WarpKind::Quad(Quad {
                    corners: [
                        Vec2::new(0.6, 0.6),
                        Vec2::new(0.9, 0.62),
                        Vec2::new(0.88, 0.9),
                        Vec2::new(0.62, 0.88),
                    ],
                }),
                masks: vec![Mask {
                    points: vec![
                        Vec2::new(0.4, 0.4),
                        Vec2::new(0.6, 0.4),
                        Vec2::new(0.5, 0.6),
                    ],
                    feather: 0.05,
                    subtract: true,
                }],
                locked: false,
            },
        ],
        ..Project::default()
    }
}
