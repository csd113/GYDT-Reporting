use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Layout, Rect,
    RichText, Sense, Stroke, TextStyle, Ui, UiBuilder, Vec2, pos2, vec2,
};

pub(super) const SURFACE: Color32 = Color32::from_rgb(251, 252, 254);
pub(super) const WASH: Color32 = Color32::from_rgb(231, 241, 251);
pub(super) const TITLE: Color32 = Color32::from_rgb(220, 231, 245);
pub(super) const TOOLBAR: Color32 = Color32::from_rgb(236, 243, 252);
pub(super) const ACCENT_SUBTLE: Color32 = Color32::from_rgb(227, 234, 251);
pub(super) const CONTACT: Color32 = Color32::from_rgb(244, 247, 255);
pub(super) const CONTACT_LINE: Color32 = Color32::from_rgb(198, 206, 240);
pub(super) const BADGE: Color32 = Color32::from_rgb(228, 233, 250);
pub(super) const BUTTON: Color32 = Color32::from_rgb(247, 250, 255);
pub(super) const INK: Color32 = Color32::from_rgb(24, 35, 62);
pub(super) const MUTED: Color32 = Color32::from_rgb(84, 97, 132);
pub(super) const BORDER: Color32 = Color32::from_rgb(191, 205, 236);
pub(super) const CONTROL: Color32 = Color32::from_rgb(119, 133, 163);
pub(super) const INDIGO: Color32 = Color32::from_rgb(53, 56, 206);
pub(super) const DEEP_INDIGO: Color32 = Color32::from_rgb(9, 6, 169);
pub(super) const FOCUS: Color32 = Color32::from_rgb(96, 91, 229);
pub(super) const INVALID: Color32 = Color32::from_rgb(145, 40, 52);
pub(super) const INSET: f32 = 20.0;
pub(super) const GUTTER: f32 = 24.0;
pub(super) const CONTROL_HEIGHT: f32 = 36.0;
pub(super) const TITLE_HEIGHT: f32 = 57.0;
pub(super) const TABS_HEIGHT: f32 = 52.0;
pub(super) const FOOTER_HEIGHT: f32 = 61.0;
pub(super) const SETTINGS_HEADING_HEIGHT: f32 = 53.0;

const EMPHASIS: &str = "Interface emphasis";

pub(in crate::desktop) fn configure(ctx: &Context) {
    load_fonts(ctx);
    ctx.set_theme(egui::ThemePreference::Light);
    let mut style = egui::Style::default();
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = WASH;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = SURFACE;
    visuals.selection.bg_fill = ACCENT_SUBTLE;
    visuals.selection.stroke = Stroke::new(1.75_f32, FOCUS);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    for (widget, fill, border) in [
        (&mut visuals.widgets.inactive, SURFACE, CONTROL),
        (&mut visuals.widgets.hovered, BUTTON, CONTROL),
        (&mut visuals.widgets.active, ACCENT_SUBTLE, FOCUS),
        (&mut visuals.widgets.open, SURFACE, FOCUS),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0_f32, border);
        widget.fg_stroke = Stroke::new(1.0_f32, INK);
        widget.corner_radius = egui::CornerRadius::same(5);
        widget.expansion = 0.0;
    }
    style.visuals = visuals;
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.item_spacing = vec2(8.0, 4.0);
    style.spacing.combo_width = 100.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Heading, bold_font(20.0));
    ctx.set_global_style(style);
}

fn load_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let normal = FontFamily::Proportional;
    let mut emphasis = fonts.families.get(&normal).cloned().unwrap_or_default();
    // Use an installed system font on macOS. The default fonts are retained
    // for additional glyphs and platforms where the system font is absent.
    if cfg!(target_os = "macos")
        && let Ok(bytes) = std::fs::read("/System/Library/Fonts/HelveticaNeue.ttc")
    {
        for (name, index) in [("Helvetica Neue", 0), ("Helvetica Neue Bold", 1)] {
            let font = FontData {
                index,
                ..FontData::from_owned(bytes.clone())
            };
            fonts.font_data.insert(name.into(), Arc::new(font));
        }
        fonts
            .families
            .entry(normal)
            .or_default()
            .insert(0, "Helvetica Neue".into());
        emphasis.insert(0, "Helvetica Neue Bold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name(EMPHASIS.into()), emphasis);
    ctx.set_fonts(fonts);
}

pub(super) fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(EMPHASIS.into()))
}

pub(super) fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(bold_font(size))
}

pub(super) fn background(ui: &Ui) {
    let area = ui.max_rect();
    let mut mesh = egui::Mesh::default();
    for (point, colour) in [
        (area.left_top(), Color32::from_rgb(244, 247, 252)),
        (area.right_top(), Color32::from_rgb(236, 243, 252)),
        (area.right_bottom(), WASH),
        (area.left_bottom(), Color32::from_rgb(238, 244, 252)),
    ] {
        mesh.colored_vertex(point, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

pub(super) fn accent_shape(
    area: Rect,
    radius: egui::CornerRadius,
    enabled: bool,
    pressed: bool,
) -> egui::Shape {
    if !area.is_positive() {
        return egui::Shape::Noop;
    }
    let opacity = if enabled { 255 } else { 128 };
    let (top, bottom) = if pressed {
        ([69, 62, 218], [30, 28, 161])
    } else {
        ([80, 74, 240], [37, 35, 191])
    };
    let [tr, tg, tb] = top;
    let [br, bg, bb] = bottom;
    let top = Color32::from_rgba_unmultiplied(tr, tg, tb, opacity);
    let bottom = Color32::from_rgba_unmultiplied(br, bg, bb, opacity);
    let colour = |point: egui::Pos2| {
        top.lerp_to_gamma(
            bottom,
            ((point.y - area.top()) / area.height()).clamp(0.0, 1.0),
        )
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(area.center(), colour(area.center()));
    let quarter = std::f32::consts::FRAC_PI_2;
    let mut vertex = 1;
    for (corner, radius, direction, angle) in [
        (area.left_top(), radius.nw, vec2(1.0, 1.0), 2.0 * quarter),
        (area.right_top(), radius.ne, vec2(-1.0, 1.0), 3.0 * quarter),
        (area.right_bottom(), radius.se, vec2(-1.0, -1.0), 0.0),
        (area.left_bottom(), radius.sw, vec2(1.0, -1.0), quarter),
    ] {
        let radius = f32::from(radius);
        let center = corner + direction * radius;
        for point in 0_u8..=4 {
            let radians = angle + quarter * f32::from(point) / 4.0;
            let point = center + vec2(radians.cos(), radians.sin()) * radius;
            mesh.colored_vertex(point, colour(point));
            if vertex > 1 {
                mesh.add_triangle(0, vertex - 1, vertex);
            }
            vertex += 1;
        }
    }
    mesh.add_triangle(0, vertex - 1, 1);
    egui::Shape::mesh(mesh)
}

pub(super) fn focus_ring(ui: &Ui, response: &egui::Response, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            radius,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
}

#[derive(Clone, Copy)]
pub(super) enum Rule {
    Top,
    Bottom,
}

pub(super) fn band<R>(
    ui: &mut Ui,
    height: f32,
    fill: Color32,
    rule: Rule,
    inset: Vec2,
    contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect_filled(area, 0, fill);
    let y = match rule {
        Rule::Top => area.top() + 0.5,
        Rule::Bottom => area.bottom() - 0.5,
    };
    ui.painter().line_segment(
        [pos2(area.left(), y), pos2(area.right(), y)],
        Stroke::new(1.0_f32, BORDER),
    );
    let content = Rect::from_min_max(area.min + inset, area.max - inset);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(content)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(area));
    contents(&mut child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_and_outline_tessellate_together() {
        let ctx = Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(170.0, CONTROL_HEIGHT));
            ui.painter()
                .add(accent_shape(rect, egui::CornerRadius::same(7), true, false));
            ui.painter().rect_stroke(
                rect.expand(2.0),
                8,
                Stroke::new(1.75_f32, FOCUS),
                egui::StrokeKind::Outside,
            );
        });
        assert!(
            !ctx.tessellate(output.shapes, output.pixels_per_point)
                .is_empty()
        );
    }
}
