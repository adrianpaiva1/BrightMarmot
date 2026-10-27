//! The Add Game dialog: pick from running applications or choose an .exe from disk.

use eframe::egui::{self, Align, Align2, FontId, Layout, Rect, Sense, pos2, vec2};

use super::{IconCache, theme};
use crate::apps::{RunningApp, running_apps};
use crate::config::Config;
use crate::win::windows_list::top_level_windows;

pub struct AddGameDialog {
    apps: Vec<RunningApp>,
    selected: Option<usize>,
}

pub enum DialogResult {
    Open,
    Cancelled,
    /// Exe path to add.
    Add(String),
}

impl AddGameDialog {
    pub fn new(config: &Config) -> Self {
        let mut d = AddGameDialog { apps: Vec::new(), selected: None };
        d.refresh(config);
        d
    }

    fn refresh(&mut self, config: &Config) {
        let own = std::env::current_exe().map(|p| crate::config::exe_name_from_path(&p.to_string_lossy())).unwrap_or_default();
        self.apps = running_apps(&top_level_windows(), &own, config);
        self.selected = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, icons: &mut IconCache, config: &Config) -> DialogResult {
        let mut result = DialogResult::Open;
        let modal = egui::Modal::new(egui::Id::unique("add_game_dialog"))
            .frame(theme::card().inner_margin(egui::Margin::same(16)).fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(egui::CornerRadius::same(8)))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                ui.label(theme::bold("Add Game", 18.0));
                ui.add_space(8.0);
                ui.label(theme::bold("Running applications", 14.0));
                ui.add_space(6.0);

                egui::Frame::new()
                    .fill(theme::ENTRY)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::vertical().max_height(340.0).min_scrolled_height(340.0).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for i in 0..self.apps.len() {
                                let icon = icons.get(ui.ctx(), &self.apps[i].exe_path);
                                let response = app_row(ui, &self.apps[i], icon.as_ref(), self.selected == Some(i));
                                if response.clicked() {
                                    self.selected = Some(i);
                                }
                                if response.double_clicked() {
                                    result = DialogResult::Add(self.apps[i].exe_path.clone());
                                }
                            }
                        });
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.refresh(config);
                    }
                    if ui.button("Pick .exe from disk…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("Executable", &["exe"]).pick_file() {
                            result = DialogResult::Add(path.to_string_lossy().into_owned());
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let chosen = self.selected.and_then(|i| self.apps.get(i));
                        if ui.add_enabled(chosen.is_some(), egui::Button::new("Add")).clicked() {
                            if let Some(app) = chosen {
                                result = DialogResult::Add(app.exe_path.clone());
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            result = DialogResult::Cancelled;
                        }
                    });
                });
            });
        if matches!(result, DialogResult::Open) && modal.should_close() {
            result = DialogResult::Cancelled;
        }
        result
    }
}

fn app_row(ui: &mut egui::Ui, app: &RunningApp, icon: Option<&egui::TextureHandle>, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            theme::SELECTED
        } else if response.hovered() {
            theme::HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        painter.rect_filled(rect, egui::CornerRadius::same(6), fill);
        let icon_rect = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 16.0), vec2(32.0, 32.0));
        theme::paint_icon(painter, icon_rect, icon);
        let clipped = painter.with_clip_rect(rect.shrink(4.0));
        let x = icon_rect.right() + 10.0;
        clipped.text(pos2(x, rect.center().y - 9.0), Align2::LEFT_CENTER, &app.title, FontId::new(14.5, egui::FontFamily::Name(theme::BOLD.into())), theme::TEXT);
        clipped.text(pos2(x, rect.center().y + 10.0), Align2::LEFT_CENTER, &app.process_name, FontId::proportional(12.0), theme::SECONDARY);
    }
    response
}
