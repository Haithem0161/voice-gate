use std::time::Instant;

use crate::app_controller::AppController;
use crate::config::Config;
use crate::gui::enrollment_wizard::EnrollmentWizardState;

// -- Portfoliolio color palette --
const BG: egui::Color32 = egui::Color32::from_rgb(0x19, 0x10, 0x22);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x1b, 0x36);
const SURFACE_HOVER: egui::Color32 = egui::Color32::from_rgb(0x34, 0x21, 0x43);
const PRIMARY: egui::Color32 = egui::Color32::from_rgb(0x7f, 0x13, 0xec);
const PRIMARY_DIM: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x0d, 0x85);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xf7, 0xf6, 0xf8);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(0x8a, 0x84, 0x94);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x3D, 0xAE, 0x2B);
const WARNING: egui::Color32 = egui::Color32::from_rgb(0xf5, 0x9e, 0x0b);
const DANGER: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x23, 0x4E);
const LED_OFF: egui::Color32 = egui::Color32::from_rgb(0x3d, 0x2f, 0x4a);
const BORDER: egui::Color32 = egui::Color32::from_rgba_premultiplied(255, 255, 255, 15);
const METER_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x7f, 0x13, 0xec, 20);

enum Screen {
    Main,
    EnrollmentWizard(EnrollmentWizardState),
}

pub struct VoiceGateApp {
    controller: AppController,
    screen: Screen,
    error_banner: Option<(String, Instant)>,
    threshold: f32,
    hold_frames: u32,
    bypass_mode: usize,
}

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;

    v.dark_mode = true;
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.override_text_color = Some(TEXT);
    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_MUTED);
    v.widgets.noninteractive.rounding = egui::Rounding::same(8.0);

    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_MUTED);
    v.widgets.inactive.rounding = egui::Rounding::same(8.0);
    v.widgets.inactive.weak_bg_fill = SURFACE;

    v.widgets.hovered.bg_fill = SURFACE_HOVER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.hovered.rounding = egui::Rounding::same(8.0);

    v.widgets.active.bg_fill = PRIMARY_DIM;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.active.rounding = egui::Rounding::same(8.0);

    v.selection.bg_fill = PRIMARY;
    v.selection.stroke = egui::Stroke::new(1.0, TEXT);

    v.window_rounding = egui::Rounding::same(12.0);
    v.window_stroke = egui::Stroke::new(1.0, BORDER);

    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(20.0);

    ctx.set_style(style);
}

impl VoiceGateApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);

        let config = Config::load().unwrap_or_default();
        let threshold = config.verification.threshold;
        let hold_frames = config.gate.hold_frames;
        let controller = AppController::new(config);

        Self {
            controller,
            screen: Screen::Main,
            error_banner: None,
            threshold,
            hold_frames,
            bypass_mode: 0,
        }
    }

    fn show_error(&mut self, msg: String) {
        self.error_banner = Some((msg, Instant::now()));
    }

    fn render_header(&mut self, ui: &mut egui::Ui) {
        let status = self.controller.status_snapshot();

        ui.horizontal(|ui| {
            // App icon placeholder
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
            ui.painter().rect_filled(icon_rect, 6.0, PRIMARY);

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("VoiceGate")
                    .size(15.0)
                    .strong()
                    .color(TEXT),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Start/Stop button
                if status.is_running {
                    let btn =
                        egui::Button::new(egui::RichText::new("Stop").size(11.0).color(DANGER))
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_premultiplied(0xE0, 0x23, 0x4E, 100),
                            ))
                            .fill(egui::Color32::TRANSPARENT)
                            .rounding(8.0);
                    if ui.add(btn).clicked() {
                        if let Err(e) = self.controller.stop() {
                            self.show_error(e.to_string());
                        }
                    }
                } else {
                    let btn = egui::Button::new(
                        egui::RichText::new("Start").size(11.0).strong().color(TEXT),
                    )
                    .fill(PRIMARY)
                    .rounding(8.0);
                    if ui.add(btn).clicked() {
                        match self.controller.load_default_profile() {
                            Ok(profile) => {
                                if let Err(e) = self.controller.start(profile) {
                                    self.show_error(e.to_string());
                                }
                            }
                            Err(e) => {
                                self.show_error(format!(
                                    "No profile found. Please enroll your voice first. ({e})"
                                ));
                            }
                        }
                    }
                }

                // Status pill
                let pill_text = if status.is_running {
                    "Active"
                } else {
                    "Inactive"
                };
                let pill_color = if status.is_running {
                    SUCCESS
                } else {
                    TEXT_MUTED
                };
                let pill_bg = if status.is_running {
                    egui::Color32::from_rgba_premultiplied(0x3D, 0xAE, 0x2B, 30)
                } else {
                    egui::Color32::from_rgba_premultiplied(0x8a, 0x84, 0x94, 20)
                };

                let pill = ui.allocate_ui_with_layout(
                    egui::vec2(70.0, 20.0),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(70.0, 20.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 10.0, pill_bg);
                        ui.painter().rect_stroke(
                            rect,
                            10.0,
                            egui::Stroke::new(1.0, pill_color.linear_multiply(0.3)),
                        );

                        // Dot
                        if status.is_running {
                            let dot_center = egui::pos2(rect.left() + 14.0, rect.center().y);
                            ui.painter().circle_filled(dot_center, 3.0, pill_color);
                        }

                        let text_offset = if status.is_running { 8.0 } else { 0.0 };
                        ui.painter().text(
                            egui::pos2(rect.center().x + text_offset, rect.center().y),
                            egui::Align2::CENTER_CENTER,
                            pill_text,
                            egui::FontId::proportional(10.0),
                            pill_color,
                        );
                    },
                );
                let _ = pill;
            });
        });

        // Accent line
        let rect = ui.available_rect_before_wrap();
        let line_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left(), ui.cursor().top() + 4.0),
            egui::vec2(rect.width(), 1.0),
        );
        ui.painter().rect_filled(line_rect, 0.0, BORDER);
        ui.add_space(6.0);
    }

    fn render_similarity_card(&self, ui: &mut egui::Ui) {
        let status = self.controller.status_snapshot();

        egui::Frame::none()
            .fill(SURFACE)
            .rounding(12.0)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(16.0, 14.0))
            .show(ui, |ui| {
                // Purple accent line at top
                let card_rect = ui.max_rect();
                let accent = egui::Rect::from_min_size(
                    egui::pos2(card_rect.left(), card_rect.top() - 14.0),
                    egui::vec2(card_rect.width(), 2.0),
                );
                ui.painter()
                    .rect_filled(accent, 0.0, PRIMARY.linear_multiply(0.6));

                // Header: SIMILARITY label + score
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("SIMILARITY")
                            .size(10.0)
                            .strong()
                            .color(TEXT_MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.2}", status.similarity))
                                .size(26.0)
                                .strong()
                                .color(TEXT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
                });

                ui.add_space(8.0);

                // Custom meter bar
                let (meter_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 32.0),
                    egui::Sense::hover(),
                );

                // Track background
                ui.painter().rect_filled(meter_rect, 8.0, METER_BG);

                // Fill
                let fill_width = meter_rect.width() * status.similarity.clamp(0.0, 1.0);
                let fill_rect = egui::Rect::from_min_size(
                    meter_rect.left_top(),
                    egui::vec2(fill_width, meter_rect.height()),
                );
                ui.painter().rect_filled(fill_rect, 8.0, PRIMARY);

                // Glow effect on fill
                let glow_rect = fill_rect.expand2(egui::vec2(0.0, 2.0));
                ui.painter().rect_filled(
                    glow_rect,
                    8.0,
                    egui::Color32::from_rgba_premultiplied(0x7f, 0x13, 0xec, 30),
                );

                // Threshold marker
                let thr_x = meter_rect.left() + meter_rect.width() * self.threshold;
                let thr_top = meter_rect.top() - 4.0;
                let thr_bot = meter_rect.bottom() + 4.0;
                ui.painter().line_segment(
                    [egui::pos2(thr_x, thr_top), egui::pos2(thr_x, thr_bot)],
                    egui::Stroke::new(2.0, TEXT.linear_multiply(0.25)),
                );

                // THR label below
                ui.painter().text(
                    egui::pos2(thr_x, thr_bot + 8.0),
                    egui::Align2::CENTER_CENTER,
                    "THR",
                    egui::FontId::proportional(8.0),
                    TEXT_MUTED,
                );

                ui.add_space(12.0);
            });
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            // Threshold card
            egui::Frame::none()
                .fill(SURFACE)
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(14.0, 12.0))
                .show(&mut cols[0], |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("THRESHOLD")
                                .size(10.0)
                                .strong()
                                .color(TEXT_MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.2}", self.threshold))
                                    .size(14.0)
                                    .strong()
                                    .color(TEXT)
                                    .family(egui::FontFamily::Monospace),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    let resp = ui.add(
                        egui::Slider::new(&mut self.threshold, 0.30..=0.95)
                            .step_by(0.01)
                            .show_value(false),
                    );
                    if resp.changed() {
                        let mut cfg = self.controller.config.write().unwrap();
                        cfg.verification.threshold = self.threshold;
                    }
                });

            // Hold time card
            egui::Frame::none()
                .fill(SURFACE)
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(14.0, 12.0))
                .show(&mut cols[1], |ui| {
                    let hold_ms = self.hold_frames * 32;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("HOLD TIME")
                                .size(10.0)
                                .strong()
                                .color(TEXT_MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{}ms", hold_ms))
                                    .size(14.0)
                                    .strong()
                                    .color(TEXT)
                                    .family(egui::FontFamily::Monospace),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    let mut hold_ms_f = hold_ms as f32;
                    let resp = ui.add(
                        egui::Slider::new(&mut hold_ms_f, 32.0..=640.0)
                            .step_by(32.0)
                            .show_value(false),
                    );
                    if resp.changed() {
                        self.hold_frames = (hold_ms_f / 32.0) as u32;
                        let mut cfg = self.controller.config.write().unwrap();
                        cfg.gate.hold_frames = self.hold_frames;
                    }
                });
        });
    }

    fn render_status_strip(&self, ui: &mut egui::Ui) {
        let status = self.controller.status_snapshot();

        egui::Frame::none()
            .fill(SURFACE)
            .rounding(12.0)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(16.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Gate LED
                    let gate_on = status.gate_state == 2;
                    let gate_color = match status.gate_state {
                        2 => SUCCESS,
                        1 | 3 => WARNING,
                        _ => LED_OFF,
                    };
                    let gate_label = match status.gate_state {
                        0 => "Closed",
                        1 => "Opening",
                        2 => "Open",
                        3 => "Closing",
                        _ => "?",
                    };
                    self.render_led(ui, gate_color, gate_on, "Gate", gate_label);

                    ui.add_space(24.0);

                    // VAD LED
                    let vad_color = if status.vad_active { SUCCESS } else { LED_OFF };
                    let vad_label = if status.vad_active {
                        "Speech"
                    } else {
                        "Silence"
                    };
                    self.render_led(ui, vad_color, status.vad_active, "VAD", vad_label);
                });
            });
    }

    fn render_led(
        &self,
        ui: &mut egui::Ui,
        color: egui::Color32,
        glowing: bool,
        title: &str,
        state: &str,
    ) {
        ui.horizontal(|ui| {
            // Dot
            let (dot_rect, _) =
                ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            let center = dot_rect.center();
            if glowing {
                ui.painter()
                    .circle_filled(center, 8.0, color.linear_multiply(0.15));
            }
            ui.painter().circle_filled(center, 5.0, color);

            ui.vertical(|ui| {
                ui.add_space(0.0);
                ui.label(egui::RichText::new(title).size(11.0).strong().color(TEXT));
                ui.label(
                    egui::RichText::new(state.to_uppercase())
                        .size(9.0)
                        .color(TEXT_MUTED),
                );
            });
        });
    }

    fn render_action_row(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            // Bypass card
            egui::Frame::none()
                .fill(SURFACE)
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                .show(&mut cols[0], |ui| {
                    ui.label(
                        egui::RichText::new("BYPASS")
                            .size(10.0)
                            .strong()
                            .color(TEXT_MUTED),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let modes = ["Normal", "Pass", "Mute"];
                        for (i, label) in modes.iter().enumerate() {
                            let is_active = self.bypass_mode == i;
                            let btn_fill = if is_active { SURFACE_HOVER } else { BG };
                            let btn_text = if is_active { TEXT } else { TEXT_MUTED };
                            let btn = egui::Button::new(
                                egui::RichText::new(*label).size(10.0).color(btn_text),
                            )
                            .fill(btn_fill)
                            .rounding(4.0)
                            .min_size(egui::vec2(0.0, 22.0));
                            if ui.add(btn).clicked() {
                                self.bypass_mode = i;
                                self.controller.set_bypass(i as u8);
                            }
                        }
                    });
                });

            // Re-enroll button card
            let enroll_btn = egui::Button::new(
                egui::RichText::new("Re-enroll Voice  ->")
                    .size(12.0)
                    .strong()
                    .color(PRIMARY),
            )
            .fill(SURFACE)
            .stroke(egui::Stroke::new(1.0, PRIMARY.linear_multiply(0.3)))
            .rounding(12.0)
            .min_size(egui::vec2(cols[1].available_width(), 52.0));

            if cols[1].add(enroll_btn).clicked() {
                if self.controller.is_running() {
                    let _ = self.controller.stop();
                }
                self.screen = Screen::EnrollmentWizard(EnrollmentWizardState::new());
            }
        });
    }

    fn render_main_screen(&mut self, ui: &mut egui::Ui) {
        // Error banner
        let banner_expired = self
            .error_banner
            .as_ref()
            .is_some_and(|(_, created)| created.elapsed().as_secs() > 10);
        if banner_expired {
            self.error_banner = None;
        }
        if let Some((ref msg, _)) = self.error_banner {
            let msg_text = msg.clone();
            let dismiss = egui::Frame::none()
                .fill(egui::Color32::from_rgba_premultiplied(0xE0, 0x23, 0x4E, 30))
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&msg_text).size(11.0).color(DANGER));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.small_button("x").clicked()
                        })
                        .inner
                    })
                    .inner
                })
                .inner;
            if dismiss {
                self.error_banner = None;
            }
            ui.add_space(4.0);
        }

        self.render_header(ui);
        ui.add_space(2.0);
        self.render_similarity_card(ui);
        ui.add_space(2.0);
        self.render_controls(ui);
        ui.add_space(2.0);
        self.render_status_strip(ui);
        ui.add_space(2.0);
        self.render_action_row(ui);
    }
}

impl eframe::App for VoiceGateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
            )
            .show(ctx, |ui| match &mut self.screen {
                Screen::Main => {
                    self.render_main_screen(ui);
                }
                Screen::EnrollmentWizard(ref mut wizard) => {
                    if wizard.render(ui, &self.controller) {
                        self.screen = Screen::Main;
                    }
                }
            });

        if self.controller.is_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.controller.stop();
        if let Ok(cfg) = self.controller.config.read() {
            if let Err(e) = cfg.save() {
                tracing::warn!("failed to save config on exit: {e}");
            }
        }
    }
}

fn load_icon() -> Option<egui::IconData> {
    let png_bytes = include_bytes!("../../assets/icon.png");
    let image = image::load_from_memory(png_bytes).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

pub fn run() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([500.0, 520.0])
        .with_min_inner_size([440.0, 420.0]);

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "VoiceGate",
        options,
        Box::new(|cc| Ok(Box::new(VoiceGateApp::new(cc)))),
    )
}
