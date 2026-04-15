use std::time::Instant;

use crate::app_controller::AppController;
use crate::config::Config;
use crate::gui::enrollment_wizard::EnrollmentWizardState;

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

impl VoiceGateApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

    fn render_main_screen(&mut self, ui: &mut egui::Ui) {
        let status = self.controller.status_snapshot();

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
            let dismiss = ui
                .horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, &msg_text);
                    ui.small_button("Dismiss").clicked()
                })
                .inner;
            ui.separator();
            if dismiss {
                self.error_banner = None;
            }
        }

        // Status
        ui.horizontal(|ui| {
            ui.label("Status:");
            if status.is_running {
                ui.colored_label(egui::Color32::GREEN, "Active");
            } else {
                ui.label("Inactive");
            }
        });

        ui.add_space(8.0);

        // Start / Stop button
        ui.horizontal(|ui| {
            if status.is_running {
                if ui.button("Stop").clicked() {
                    if let Err(e) = self.controller.stop() {
                        self.show_error(e.to_string());
                    }
                }
            } else if ui.button("Start").clicked() {
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
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // Similarity meter
        ui.horizontal(|ui| {
            ui.label("Similarity:");
            let color = if status.similarity >= self.threshold {
                egui::Color32::GREEN
            } else {
                egui::Color32::from_rgb(200, 60, 60)
            };
            let bar = egui::ProgressBar::new(status.similarity.clamp(0.0, 1.0))
                .text(format!("{:.2}", status.similarity))
                .fill(color);
            ui.add(bar);
        });

        ui.add_space(8.0);

        // Threshold slider
        ui.horizontal(|ui| {
            ui.label("Threshold:");
            let resp = ui.add(egui::Slider::new(&mut self.threshold, 0.30..=0.95).step_by(0.01));
            if resp.changed() {
                let mut cfg = self.controller.config.write().unwrap();
                cfg.verification.threshold = self.threshold;
            }
        });

        // Hold time slider (in frames, show milliseconds)
        ui.horizontal(|ui| {
            ui.label("Hold time:");
            let hold_ms = self.hold_frames * 32;
            let mut hold_ms_f = hold_ms as f32;
            let resp = ui.add(
                egui::Slider::new(&mut hold_ms_f, 32.0..=640.0)
                    .step_by(32.0)
                    .suffix(" ms"),
            );
            if resp.changed() {
                self.hold_frames = (hold_ms_f / 32.0) as u32;
                let mut cfg = self.controller.config.write().unwrap();
                cfg.gate.hold_frames = self.hold_frames;
            }
        });

        ui.add_space(8.0);

        // Bypass mode
        ui.horizontal(|ui| {
            ui.label("Bypass:");
            let modes = ["Normal", "On (pass all)", "Off (mute)"];
            egui::ComboBox::from_id_source("bypass")
                .selected_text(modes[self.bypass_mode])
                .show_ui(ui, |ui| {
                    for (i, label) in modes.iter().enumerate() {
                        if ui
                            .selectable_value(&mut self.bypass_mode, i, *label)
                            .changed()
                        {
                            self.controller.set_bypass(self.bypass_mode as u8);
                        }
                    }
                });
        });

        ui.add_space(8.0);

        // Re-enroll button
        if ui.button("Re-enroll Voice").clicked() {
            if self.controller.is_running() {
                let _ = self.controller.stop();
            }
            self.screen = Screen::EnrollmentWizard(EnrollmentWizardState::new());
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // Gate and VAD LEDs
        ui.horizontal(|ui| {
            let gate_color = match status.gate_state {
                2 => egui::Color32::GREEN,      // Open
                1 | 3 => egui::Color32::YELLOW, // Opening / Closing
                _ => egui::Color32::GRAY,       // Closed
            };
            let gate_label = match status.gate_state {
                0 => "CLOSED",
                1 => "OPENING",
                2 => "OPEN",
                3 => "CLOSING",
                _ => "?",
            };
            ui.colored_label(gate_color, format!("Gate: {gate_label}"));

            ui.add_space(16.0);

            let vad_color = if status.vad_active {
                egui::Color32::GREEN
            } else {
                egui::Color32::GRAY
            };
            let vad_label = if status.vad_active {
                "Speech"
            } else {
                "Silence"
            };
            ui.colored_label(vad_color, format!("VAD: {vad_label}"));
        });
    }
}

impl eframe::App for VoiceGateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| match &mut self.screen {
            Screen::Main => {
                self.render_main_screen(ui);
            }
            Screen::EnrollmentWizard(ref mut wizard) => {
                if wizard.render(ui, &self.controller) {
                    self.screen = Screen::Main;
                }
            }
        });

        // Keep the meter updating at ~20 Hz when the pipeline is running
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
        .with_inner_size([480.0, 400.0])
        .with_min_inner_size([400.0, 300.0]);

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
