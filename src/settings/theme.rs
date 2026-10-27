//! Dark theme, colours, and the small custom widgets (pill switch, orange slider,
//! number box, icon tile) used by the Settings window.

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Painter, Rect, RichText,
    Sense, Stroke, StrokeKind, TextStyle, pos2, vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x1B, 0x1C, 0x1F);
pub const PANEL: Color32 = Color32::from_rgb(0x26, 0x27, 0x2B);
pub const BORDER: Color32 = Color32::from_rgb(0x36, 0x37, 0x3C);
pub const TEXT: Color32 = Color32::from_rgb(0xF0, 0xF0, 0xF0);
pub const SECONDARY: Color32 = Color32::from_rgb(0xA0, 0xA0, 0xA8);
pub const ENTRY: Color32 = Color32::from_rgb(0x11, 0x12, 0x14);
pub const ACCENT: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);

/// Row hover and selected-row fills for the game list.
pub const HOVER: Color32 = Color32::from_rgb(0x2E, 0x2F, 0x34);
pub const SELECTED: Color32 = Color32::from_rgb(0x45, 0x3A, 0x26);

const BOLD: &str = "bold";

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
        style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Name(BOLD.into())));

        let s = &mut style.spacing;
        s.item_spacing = vec2(8.0, 8.0);
        s.button_padding = vec2(12.0, 5.0);
        s.interact_size.y = 28.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.faint_bg_color = PANEL;
        v.extreme_bg_color = ENTRY;
        v.text_edit_bg_color = Some(ENTRY);
        v.weak_text_color = Some(SECONDARY);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        let states = [
            (&mut w.inactive, BORDER, Stroke::NONE),
            (&mut w.hovered, Color32::from_rgb(0x42, 0x43, 0x49), Stroke::new(1.0, Color32::from_rgb(0x55, 0x56, 0x5C))),
            (&mut w.active, Color32::from_rgb(0x4C, 0x4D, 0x54), Stroke::new(1.0, ACCENT)),
            (&mut w.open, BORDER, Stroke::new(1.0, BORDER)),
        ];
        for (wv, fill, stroke) in states {
            wv.bg_fill = fill;
            wv.weak_bg_fill = fill;
            wv.bg_stroke = stroke;
            wv.fg_stroke = Stroke::new(1.0, TEXT);
            wv.corner_radius = CornerRadius::same(6);
            wv.expansion = 0.0;
        }
    });
}

/// Uses Segoe UI (and Segoe UI Semibold for bold text) when Windows has them; egui's
/// built-in fonts stay as the fallback, so the `bold` family always exists.
fn load_fonts(ctx: &egui::Context) {
    let dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts");
    let mut fonts = FontDefinitions::default();
    let mut regular = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut bold = regular.clone();
    if let Ok(data) = std::fs::read(dir.join("segoeui.ttf")) {
        fonts.font_data.insert("segoe-ui".into(), Arc::new(FontData::from_owned(data)));
        regular.insert(0, "segoe-ui".into());
        bold.insert(0, "segoe-ui".into());
    }
    if let Ok(data) = std::fs::read(dir.join("seguisb.ttf")) {
        fonts.font_data.insert("segoe-ui-semibold".into(), Arc::new(FontData::from_owned(data)));
        bold.insert(0, "segoe-ui-semibold".into());
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name(BOLD.into()), bold);
    ctx.set_fonts(fonts);
}

/// Bold text in the main text colour.
pub fn bold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(FontFamily::Name(BOLD.into())).size(size).color(TEXT)
}

/// Grey secondary text.
pub fn secondary(text: impl Into<String>) -> RichText {
    RichText::new(text).color(SECONDARY)
}

/// A rounded panel ("card") on the near-black background.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(16))
}

/// Pill switch, about 44×24: accent with a dark knob when on, outlined when off.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let (enabled, value) = (ui.is_enabled(), *on);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, enabled, value, ""));

    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let radius = rect.height() / 2.0;
        let painter = ui.painter();
        if *on {
            painter.rect_filled(rect, radius, ACCENT);
        } else {
            painter.rect(rect, radius, ENTRY, Stroke::new(1.5, SECONDARY), StrokeKind::Inside);
        }
        let knob_radius = radius - 5.0 + if response.hovered() { 1.0 } else { 0.0 };
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if *on { BG } else { SECONDARY };
        painter.circle_filled(pos2(x, rect.center().y), knob_radius, knob);
    }
    response
}

/// egui slider without its value text: grey rail, accent fill and accent knob
/// (grey when the ui is disabled). Values are rounded to `decimals`.
pub fn slider(ui: &mut egui::Ui, value: &mut f64, range: RangeInclusive<f64>, decimals: usize, width: f32) -> egui::Response {
    ui.scope(|ui| {
        let colour = if ui.is_enabled() { ACCENT } else { SECONDARY };
        let spacing = ui.spacing_mut();
        spacing.slider_width = width;
        spacing.slider_rail_height = 6.0;
        spacing.interact_size.y = 20.0; // knob radius = height / 2.5
        let visuals = ui.visuals_mut();
        visuals.selection.bg_fill = colour;
        for wv in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
            wv.bg_fill = BORDER;
            wv.fg_stroke = Stroke::NONE;
            wv.corner_radius = CornerRadius::same(3);
            wv.expansion = 0.0;
        }

        let (lo, hi) = (*range.start(), *range.end());
        let response = ui.add(egui::Slider::new(value, range).show_value(false).fixed_decimals(decimals).trailing_fill(true));

        // egui paints the knob in the rail colour; paint the accent knob over it, same geometry.
        let rect = response.rect;
        let radius = rect.height() / 2.5;
        let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let grow = if response.hovered() || response.dragged() { 1.5 } else { 0.0 };
        ui.painter().circle_filled(pos2(x, rect.center().y), radius + grow, colour);
        response
    })
    .inner
}

/// Dark number box (`DragValue`) clamped to `range`, showing `decimals` places.
pub fn number_box(ui: &mut egui::Ui, value: &mut f64, range: RangeInclusive<f64>, decimals: usize) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for wv in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            wv.weak_bg_fill = ENTRY;
            wv.bg_fill = ENTRY;
        }
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .fixed_decimals(decimals)
                .speed(10f64.powi(-(decimals as i32)))
                .min_size(vec2(68.0, 28.0)),
        )
    })
    .inner
}

/// Draws an exe icon into `rect`, or a plain `#36373C` rounded square without one.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Option<&egui::TextureHandle>) {
    match icon {
        Some(tex) => {
            painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, CornerRadius::same(6), BORDER);
        }
    }
}
