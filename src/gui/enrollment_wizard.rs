use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::app_controller::AppController;

#[derive(PartialEq)]
pub enum WizardStatus {
    ReadyToStart,
    Recording,
    Processing,
    Done(PathBuf),
    Failed(String),
}

pub struct EnrollmentWizardState {
    pub seconds_target: u32,
    pub progress: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub status: WizardStatus,
    pub passage: String,
    worker: Option<JoinHandle<anyhow::Result<()>>>,
    started_at: Option<Instant>,
}

impl Default for EnrollmentWizardState {
    fn default() -> Self {
        Self::new()
    }
}

impl EnrollmentWizardState {
    pub fn new() -> Self {
        let passage = Self::load_passage().unwrap_or_else(|_| {
            "(passage file missing -- speak naturally for the duration)".into()
        });
        Self {
            seconds_target: 30,
            progress: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            status: WizardStatus::ReadyToStart,
            passage,
            worker: None,
            started_at: None,
        }
    }

    fn load_passage() -> anyhow::Result<String> {
        let path = crate::resolve_asset_path("enrollment_passages.txt")?;
        Ok(std::fs::read_to_string(path)?)
    }

    pub fn start_recording(&mut self, controller: &AppController) {
        self.progress.store(0, Ordering::Relaxed);
        self.cancel.store(false, Ordering::Relaxed);
        self.status = WizardStatus::Recording;
        self.started_at = Some(Instant::now());

        let seconds = self.seconds_target;
        let progress = self.progress.clone();
        let cancel = self.cancel.clone();
        let config = controller.config.read().unwrap().clone();

        self.worker = Some(thread::spawn(move || {
            use ringbuf::traits::Consumer;

            let silero_path = crate::resolve_model_path(&config.vad.model_path)?;
            let wespeaker_path = crate::resolve_model_path(&config.verification.model_path)?;
            let vad = crate::ml::vad::SileroVad::load(&silero_path)?;
            let ecapa = crate::ml::embedding::EcapaTdnn::load(&wespeaker_path)?;
            let mut session = crate::enrollment::enroll::EnrollmentSession::new(vad, ecapa);

            let (input_prod, mut input_cons) = crate::audio::ring_buffer::new_audio_ring(
                crate::audio::ring_buffer::RING_CAPACITY_SAMPLES,
            );
            let capture =
                crate::audio::capture::start_capture(Some(&config.audio.input_device), input_prod)?;

            let mut resampler = crate::audio::resampler::Resampler48to16::new()?;
            let mut scratch = vec![0.0f32; crate::audio::resampler::INPUT_CHUNK_SAMPLES];

            let start = std::time::Instant::now();
            let target = std::time::Duration::from_secs(u64::from(seconds));

            while start.elapsed() < target && !cancel.load(Ordering::Relaxed) {
                let n = input_cons.pop_slice(&mut scratch);
                if n == 0 {
                    thread::sleep(std::time::Duration::from_micros(500));
                    continue;
                }
                if n == crate::audio::resampler::INPUT_CHUNK_SAMPLES {
                    let out = resampler.process_block(&scratch)?;
                    session.push_audio(out);
                }
                progress.store(start.elapsed().as_secs() as u32, Ordering::Relaxed);
            }

            drop(capture);

            let centroid = session.finalize()?;
            let profile = crate::enrollment::profile::Profile::new(centroid);
            let out_path = crate::enrollment::profile::Profile::default_path()?;
            profile.save(&out_path)?;

            Ok(())
        }));
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn poll(&mut self) {
        if let Some(ref worker) = self.worker {
            if worker.is_finished() {
                let worker = self.worker.take().unwrap();
                match worker.join() {
                    Ok(Ok(())) => {
                        let path = crate::enrollment::profile::Profile::default_path()
                            .unwrap_or_else(|_| PathBuf::from("profile.bin"));
                        self.status = WizardStatus::Done(path);
                    }
                    Ok(Err(e)) => {
                        self.status = WizardStatus::Failed(e.to_string());
                    }
                    Err(_) => {
                        self.status = WizardStatus::Failed("enrollment thread panicked".into());
                    }
                }
            }
        }
    }

    pub fn elapsed_seconds(&self) -> u32 {
        self.progress.load(Ordering::Relaxed)
    }

    pub fn render(&mut self, ui: &mut egui::Ui, controller: &AppController) -> bool {
        let mut close = false;

        let primary = egui::Color32::from_rgb(0x7f, 0x13, 0xec);
        let text = egui::Color32::from_rgb(0xf7, 0xf6, 0xf8);
        let text_muted = egui::Color32::from_rgb(0x8a, 0x84, 0x94);
        let surface = egui::Color32::from_rgb(0x2a, 0x1b, 0x36);
        let danger = egui::Color32::from_rgb(0xE0, 0x23, 0x4E);
        let border = egui::Color32::from_rgba_premultiplied(255, 255, 255, 15);

        ui.label(
            egui::RichText::new("Voice Enrollment")
                .size(18.0)
                .strong()
                .color(text),
        );
        ui.add_space(4.0);

        match &self.status {
            WizardStatus::ReadyToStart => {
                ui.label(
                    egui::RichText::new("Please read the following text aloud:").color(text_muted),
                );
                ui.add_space(8.0);
                egui::Frame::none()
                    .fill(surface)
                    .rounding(8.0)
                    .stroke(egui::Stroke::new(1.0, border))
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(&self.passage).size(12.0).color(text));
                            });
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let start = egui::Button::new(
                        egui::RichText::new("Start Recording").strong().color(text),
                    )
                    .fill(primary)
                    .rounding(8.0);
                    if ui.add(start).clicked() {
                        self.start_recording(controller);
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            WizardStatus::Recording => {
                self.poll();
                ui.label(egui::RichText::new("Read the passage aloud:").color(text_muted));
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(surface)
                    .rounding(8.0)
                    .stroke(egui::Stroke::new(1.0, border))
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(100.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(&self.passage).size(12.0).color(text));
                            });
                    });
                ui.add_space(10.0);
                let elapsed = self.elapsed_seconds();
                let fraction = elapsed as f32 / self.seconds_target as f32;
                ui.add(
                    egui::ProgressBar::new(fraction.min(1.0))
                        .text(format!("{elapsed} s / {} s", self.seconds_target))
                        .fill(primary),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.cancel();
                        close = true;
                    }
                    if elapsed >= 15 && ui.button("Finish Early").clicked() {
                        self.cancel();
                        self.status = WizardStatus::Processing;
                    }
                });
            }
            WizardStatus::Processing => {
                self.poll();
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Processing enrollment...")
                        .size(14.0)
                        .color(text),
                );
                ui.add_space(8.0);
                ui.spinner();
            }
            WizardStatus::Done(path) => {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Enrollment complete!")
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::from_rgb(0x3D, 0xAE, 0x2B)),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("Saved: {}", path.display()))
                        .size(11.0)
                        .color(text_muted),
                );
                ui.add_space(12.0);
                let btn = egui::Button::new(egui::RichText::new("Close").strong().color(text))
                    .fill(primary)
                    .rounding(8.0);
                if ui.add(btn).clicked() {
                    close = true;
                }
            }
            WizardStatus::Failed(msg) => {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(format!("Enrollment failed: {msg}"))
                        .size(12.0)
                        .color(danger),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let retry =
                        egui::Button::new(egui::RichText::new("Retry").strong().color(text))
                            .fill(primary)
                            .rounding(8.0);
                    if ui.add(retry).clicked() {
                        *self = Self::new();
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            }
        }

        close
    }
}
