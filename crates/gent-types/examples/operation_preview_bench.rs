//! Synthetic boundary benchmark; not a provider/network latency measurement.
//! `cargo run -p gent-types --example operation_preview_bench --release --offline`

use std::{hint::black_box, time::Instant};

use gent_types::operation_preview;
use serde_json::json;

fn main() {
    for (name, input) in [
        (
            "command",
            json!({"command":"cargo test --workspace", "cwd":"X:/Dev/project"}),
        ),
        (
            "edit-2k",
            json!({"file_path":"X:/Dev/project/main.rs", "content":"fn main() {}\n".repeat(160)}),
        ),
        (
            "edit-300k",
            json!({"file_path":"X:/Dev/project/main.rs", "content":"界".repeat(100_000)}),
        ),
    ] {
        for _ in 0..100 {
            black_box(operation_preview(black_box(&input)));
        }
        let mut samples = Vec::with_capacity(10_000);
        let mut bytes = 0;
        for _ in 0..10_000 {
            let started = Instant::now();
            let preview = operation_preview(black_box(&input)).unwrap();
            let encoded = serde_json::to_vec(&preview).unwrap();
            samples.push(started.elapsed().as_nanos());
            bytes = black_box(encoded.len());
        }
        samples.sort_unstable();
        println!(
            "{name}: samples={} p50={}ns p95={}ns p99={}ns wire_bytes={bytes}",
            samples.len(),
            samples[5000],
            samples[9500],
            samples[9900]
        );
    }
}
