//! Right panel: one game's header and its five colour rows.

use eframe::egui::{self, Align, Layout, Sense, vec2};

use super::theme;
use crate::config::{Control, ControlKind, Game};

const LABEL_WIDTH: f32 = 120.0;
const BOX_WIDTH: f32 = 68.0;
const TOGGLE_WIDTH: f32 = 44.0;
const ROW_HEIGHT: f32 = 36.0;

/// Shows `game`; true if anything changed.
pub fn show(ui: &mut egui::Ui, game: &mut Game, icon: Option<&egui::TextureHandle>, nv_available: bool) -> bool {
    header(ui, game, icon);
    ui.add_space(18.0);
    ui.separator();
    ui.add_space(10.0);

    ui.label(theme::bold("Own colours for this game", 16.0));
    ui.label(theme::secondary(
        "Each value you switch on replaces your normal value while this game is in front.",
    ));
    ui.add_space(10.0);

    let mut changed = false;
    for kind in ControlKind::ALL {
        let needs_nvidia = matches!(kind, ControlKind::Vibrance | ControlKind::Hue);
        let available = nv_available || !needs_nvidia;
        changed |= ui.add_enabled_ui(available, |ui| control_row(ui, kind, game.control_mut(kind))).inner;
        if kind == ControlKind::Hue && !nv_available {
            ui.label(theme::secondary("Vibrance and hue need an NVIDIA GPU driving this screen.").small());
        }
    }
    changed
}

fn header(ui: &mut egui::Ui, game: &Game, icon: Option<&egui::TextureHandle>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
        theme::paint_icon(ui.painter(), rect, icon);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(theme::bold(&game.name, 18.0));
            // Games added by name only have no path; the exe name is what gets matched.
            let path = if game.path.is_empty() { &game.exe_name } else { &game.path };
            ui.add(egui::Label::new(theme::secondary(path)).truncate());
        });
    });
}

/// label | slider | number box | switch. Slider and box are disabled while the switch is off.
fn control_row(ui: &mut egui::Ui, kind: ControlKind, c: &mut Control) -> bool {
    let (lo, hi) = kind.range();
    let decimals = kind.decimals();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.allocate_ui_with_layout(vec2(LABEL_WIDTH, ROW_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(LABEL_WIDTH);
            ui.label(kind.label());
        });
        let gaps = 3.0 * ui.spacing().item_spacing.x + 8.0;
        let slider_width = (ui.available_width() - BOX_WIDTH - TOGGLE_WIDTH - gaps).max(80.0);
        ui.add_enabled_ui(c.enabled, |ui| {
            changed |= theme::slider(ui, &mut c.value, lo..=hi, decimals, slider_width).changed();
            ui.add_space(8.0);
            changed |= theme::number_box(ui, &mut c.value, lo..=hi, decimals).changed();
        });
        changed |= theme::toggle(ui, &mut c.enabled).changed();
    });
    if changed {
        c.value = egui::emath::round_to_decimals(c.value.clamp(lo, hi), decimals);
    }
    changed
}
