//! The Settings window (`brightmarmot.exe --settings`): its own short-lived process.

mod add_game;
mod game_view;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Align, Align2, FontId, Layout, Margin, Rect, Sense, pos2, vec2};

use crate::config::{self, AddOutcome, Config, Game, LoadOutcome};
use crate::win::{display, exe_icon, ipc, registry};

const UNREADABLE_NOTE: &str = "Couldn't read config.json, so changes won't be saved this session.";
const SAVE_FAILED_NOTE: &str = "Couldn't save settings.";

pub fn run() -> i32 {
    let Some(_instance) = ipc::single_instance(ipc::MUTEX_SETTINGS) else {
        ipc::focus_window_titled(ipc::SETTINGS_TITLE);
        return 0;
    };

    let mut path = config::default_path();
    let (config, outcome) = path.as_deref().map_or((Config::default(), LoadOutcome::Missing), config::load);
    let load_note = match outcome {
        LoadOutcome::Loaded | LoadOutcome::Missing => None,
        LoadOutcome::Corrupt => Some(config::CORRUPT_NOTICE),
        // The file is still there: never save this empty config over it.
        LoadOutcome::Unreadable => {
            path = None;
            Some(UNREADABLE_NOTE)
        }
    };
    let app = SettingsApp {
        selected: if config.games.is_empty() { None } else { Some(0) },
        config,
        path,
        dirty: false,
        load_note,
        save_failed: false,
        nv_available: display::nvidia_colour_available(),
        gamma_range_enabled: registry::gamma_range_enabled(),
        icons: IconCache::default(),
        add_dialog: None,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(ipc::SETTINGS_TITLE)
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    let result = eframe::run_native(
        ipc::SETTINGS_TITLE,
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("BrightMarmot: settings window failed: {e}");
            1
        }
    }
}

/// Exe icons as textures, keyed by exe path. Empty paths and failed extractions are cached as `None`.
#[derive(Default)]
pub struct IconCache {
    textures: HashMap<String, Option<egui::TextureHandle>>,
}

impl IconCache {
    pub fn get(&mut self, ctx: &egui::Context, exe_path: &str) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.textures.get(exe_path) {
            return cached.clone();
        }
        let texture = exe_icon::exe_icon_rgba(exe_path).map(|(w, h, rgba)| {
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            ctx.load_texture(format!("exe-icon:{exe_path}"), image, egui::TextureOptions::LINEAR)
        });
        self.textures.insert(exe_path.to_string(), texture.clone());
        texture
    }
}

struct SettingsApp {
    config: Config,
    /// `None` when `%APPDATA%` is not set or the file couldn't be read: changes then live only for this session.
    path: Option<PathBuf>,
    selected: Option<usize>,
    dirty: bool,
    /// Shown in the app section when the load at startup was not clean.
    load_note: Option<&'static str>,
    /// The last save failed; cleared by the next one that succeeds.
    save_failed: bool,
    nv_available: bool,
    gamma_range_enabled: bool,
    icons: IconCache,
    add_dialog: Option<add_game::AddGameDialog>,
}

impl SettingsApp {
    fn open_add_dialog(&mut self) {
        self.add_dialog = Some(add_game::AddGameDialog::new(&self.config));
    }

    fn show_add_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.add_dialog.as_mut() else { return };
        match dialog.show(ctx, &mut self.icons, &self.config) {
            add_game::DialogResult::Open => {}
            add_game::DialogResult::Cancelled => self.add_dialog = None,
            add_game::DialogResult::Add(path) => {
                self.add_dialog = None;
                let (AddOutcome::Added(i) | AddOutcome::AlreadyExists(i)) = self.config.add_game(&path);
                self.selected = Some(i);
                self.dirty = true;
            }
        }
    }

    /// `dirty` is cleared only once the file is written; after a failure the next frame egui
    /// draws retries. Frames come from input, and nothing here requests a repaint.
    fn save(&mut self) {
        let Some(path) = &self.path else {
            self.dirty = false;
            return;
        };
        match config::save(path, &self.config) {
            Ok(()) => {
                self.dirty = false;
                self.save_failed = false;
                ipc::notify_tray(ipc::MSG_CONFIG_CHANGED);
            }
            Err(e) => {
                eprintln!("BrightMarmot: could not save {}: {e}", path.display());
                self.save_failed = true;
            }
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("app_section")
            .frame(egui::Frame::NONE)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Add game").clicked() {
                        self.open_add_dialog();
                    }
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new("Remove")).clicked() {
                        self.remove_selected();
                    }
                });
                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    self.app_section(ui);
                });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            theme::card().inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for i in 0..self.config.games.len() {
                        let game = &self.config.games[i];
                        let icon = self.icons.get(ui.ctx(), &game.path);
                        if game_row(ui, game, icon.as_ref(), self.selected == Some(i)).clicked() {
                            self.selected = Some(i);
                        }
                    }
                });
            });
        });
    }

    fn app_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Start with Windows");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut on = self.config.start_with_windows;
                if theme::toggle(ui, &mut on).changed() {
                    self.set_start_with_windows(on);
                }
            });
        });
        ui.add_space(4.0);
        if self.gamma_range_enabled {
            ui.label("Full gamma range: enabled");
        } else {
            ui.horizontal(|ui| {
                ui.label("Full gamma range: not enabled");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Enable…").clicked() {
                        // Blocks until the elevated helper exits (UAC prompt included).
                        registry::request_gamma_range_elevated();
                        self.gamma_range_enabled = registry::gamma_range_enabled();
                    }
                });
            });
            ui.label(theme::secondary("Needs admin once and a Windows restart.").small());
        }
        for note in [self.load_note, self.save_failed.then_some(SAVE_FAILED_NOTE)].into_iter().flatten() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(note).color(theme::ACCENT));
        }
    }

    fn set_start_with_windows(&mut self, on: bool) {
        let done = if on {
            std::env::current_exe().is_ok_and(|exe| registry::set_run_entry(&exe.to_string_lossy()))
        } else {
            // Check that the value is gone: the delete also returns false when it was already absent.
            registry::delete_run_entry();
            registry::get_run_entry().is_none()
        };
        // If the Run value could not be written or removed, leave the switch and config as they were.
        if done {
            self.config.start_with_windows = on;
            self.dirty = true;
        }
    }

    fn remove_selected(&mut self) {
        let Some(i) = self.selected else { return };
        self.config.remove_game(i);
        let n = self.config.games.len();
        self.selected = if n == 0 { None } else { Some(i.min(n - 1)) };
        self.dirty = true;
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        theme::card().inner_margin(Margin::same(24)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let Some(i) = self.selected.filter(|&i| i < self.config.games.len()) else {
                if empty_state(ui) {
                    self.open_add_dialog();
                }
                return;
            };
            let icon = self.icons.get(ui.ctx(), &self.config.games[i].path);
            if game_view::show(ui, &mut self.config.games[i], icon.as_ref(), self.nv_available) {
                self.dirty = true;
            }
        });
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let outer = 12;
        egui::Panel::left("games")
            .exact_size(300.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin { left: outer, right: 6, top: outer, bottom: outer }))
            .show(ui, |ui| self.left_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin { left: 6, right: outer, top: outer, bottom: outer }))
            .show(ui, |ui| self.right_panel(ui));

        self.show_add_dialog(ui.ctx());

        // Save once a change is complete: not mid-drag, but on release (or right away for clicks and typing).
        if self.dirty && !ui.input(|i| i.pointer.any_down()) {
            self.save();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty {
            self.save();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }
}

fn game_row(ui: &mut egui::Ui, game: &Game, icon: Option<&egui::TextureHandle>, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
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
        painter.with_clip_rect(rect.shrink(4.0)).text(
            pos2(icon_rect.right() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            &game.name,
            FontId::proportional(14.5),
            theme::TEXT,
        );
    }
    response
}

/// True when the user clicked "Add game".
fn empty_state(ui: &mut egui::Ui) -> bool {
    let mut open = false;
    ui.add_space((ui.available_height() / 2.0 - 40.0).max(0.0));
    ui.vertical_centered(|ui| {
        ui.label(theme::bold("Add a game to get started.", 16.0));
        ui.add_space(8.0);
        if ui.button("Add game").clicked() {
            open = true;
        }
    });
    open
}
