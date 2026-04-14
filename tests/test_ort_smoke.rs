//! ORT + ONNX Runtime smoke test.
//!
//! Proves that the ort 2.x crate with load-dynamic can reach the system
//! libonnxruntime.so and load both of VoiceGate's ONNX models. Prints the
//! input and output tensor names for each model so the ml/vad.rs and
//! ml/embedding.rs implementations know what to reference.
//!
//! This file is intentionally lightweight. The full ML integration test
//! suite lands in tests/test_ml.rs in a later step.

use std::path::Path;

use ort::session::Session;

fn models_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/models"))
}

fn load_and_describe(name: &str) -> Option<()> {
    let path = models_dir().join(name);
    if !path.exists() {
        eprintln!(
            "skipping {name}: {} missing (run `make models`)",
            path.display()
        );
        return None;
    }

    let builder = Session::builder().unwrap_or_else(|e| panic!("Session::builder failed: {e}"));
    let session = builder
        .commit_from_file(&path)
        .unwrap_or_else(|e| panic!("failed to load {name}: {e}"));

    println!("=== {name} ===");
    println!("inputs:");
    for input in session.inputs.iter() {
        println!("  name={:?} type={:?}", input.name, input.input_type);
    }
    println!("outputs:");
    for output in session.outputs.iter() {
        println!("  name={:?} type={:?}", output.name, output.output_type);
    }
    println!();
    Some(())
}

#[test]
fn ort_can_load_silero_vad() {
    load_and_describe("silero_vad.onnx");
}

#[test]
fn ort_can_load_wespeaker_resnet34_lm() {
    load_and_describe("wespeaker_resnet34_lm.onnx");
}
