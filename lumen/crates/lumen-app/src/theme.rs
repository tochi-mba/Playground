//! Visual style: a dark theme tuned for a dim control booth, and the window icon.
//!
//! Everything here is pure data so it is testable without a window: [`visuals`] returns the
//! egui visuals and [`icon`] draws the icon into an RGBA buffer.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// Accent used for selection, handles and the active cue.
pub const ACCENT: Color32 = Color32::from_rgb(80, 180, 255);
/// Background of the canvas and output previews.
pub const CANVAS_BG: Color32 = Color32::from_gray(18);

/// The lumen dark theme.
pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_gray(30);
    v.window_fill = Color32::from_gray(34);
    v.extreme_bg_color = Color32::from_gray(14);
    v.faint_bg_color = Color32::from_gray(38);
    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(48));
    v.widgets.inactive.bg_fill = Color32::from_gray(48);
    v.widgets.hovered.bg_fill = Color32::from_gray(62);
    v.widgets.active.bg_fill = ACCENT.linear_multiply(0.5);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow.spread = 0;
    v.popup_shadow.spread = 0;
    v
}

/// Applies the theme, spacing and text sizes to a context. Idempotent.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals(visuals());
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.slider_width = 160.0;
        style.spacing.interact_size.y = 20.0;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(17.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// Draws the app icon: a projector beam (a light wedge) on a dark rounded square, with a
/// bright lens dot. Returns straight-alpha RGBA, row-major.
pub fn icon(size: u32) -> Vec<u8> {
    let n = size.max(8);
    let s = f64::from(n);
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (u, v) = ((f64::from(x) + 0.5) / s, (f64::from(y) + 0.5) / s);
            // Rounded square mask.
            let r = 0.18;
            let (cx, cy) = ((u - 0.5).abs() - (0.5 - r), (v - 0.5).abs() - (0.5 - r));
            let d = (cx.max(0.0).powi(2) + cy.max(0.0).powi(2)).sqrt() + cx.max(cy).min(0.0) - r;
            let alpha = (0.5 - d * s).clamp(0.0, 1.0);
            // Beam: a wedge from the lens at (0.2, 0.5) widening to the right.
            let (lx, ly) = (0.2, 0.5);
            let dx = u - lx;
            let inside_beam = dx > 0.0 && (v - ly).abs() < dx * 0.55 && u < 0.92;
            let beam = if inside_beam {
                (1.0 - dx / 0.9).clamp(0.3, 1.0)
            } else {
                0.0
            };
            let lens = f64::from(u8::from(
                ((u - lx).powi(2) + (v - ly).powi(2)).sqrt() < 0.09,
            ));
            let base = [24.0, 26.0, 32.0];
            let tint = [80.0, 180.0, 255.0];
            let mut px = [0u8; 4];
            for c in 0..3 {
                let col = base[c] + (tint[c] - base[c]) * beam * 0.9 + (255.0 - base[c]) * lens;
                px[c] = col.clamp(0.0, 255.0).round() as u8;
            }
            px[3] = (alpha * 255.0).round() as u8;
            out.extend_from_slice(&px);
        }
    }
    out
}

/// The icon as eframe wants it.
pub fn icon_data(size: u32) -> egui::IconData {
    let n = size.max(8);
    egui::IconData {
        rgba: icon(n),
        width: n,
        height: n,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_a_transparent_corner_an_opaque_centre_and_a_bright_lens() {
        let n = 64;
        let px = icon(n);
        assert_eq!(px.len(), (n * n * 4) as usize);
        let at = |x: u32, y: u32| {
            let i = ((y * n + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "corner is outside the rounded square");
        assert_eq!(at(32, 32)[3], 255);
        assert_eq!(at(13, 32), [255, 255, 255, 255], "lens is white");
        let beam = at(40, 32);
        assert!(beam[2] > beam[0] && beam[2] > 120, "beam is blue: {beam:?}");
        let dark = at(40, 4);
        assert!(dark[2] < 60, "outside the beam stays dark: {dark:?}");
        let d = icon_data(4);
        assert_eq!((d.width, d.height), (8, 8), "tiny sizes are clamped");
        assert_eq!(d.rgba.len(), 8 * 8 * 4);
    }

    #[test]
    fn theme_applies_to_a_context() {
        let ctx = egui::Context::default();
        apply(&ctx);
        apply(&ctx);
        let style = ctx.global_style();
        assert_eq!(style.visuals.selection.stroke.color, ACCENT);
        assert_eq!(style.text_styles[&TextStyle::Body].size, 13.0);
        assert!(visuals().dark_mode);
    }
}
