//! Read verified CPU temperatures without installing a helper or exposing process data.
use mangodisk_platform::system_resources::cpu::{
    details::CpuDetailsReader, temperature::CpuTemperatureReader,
};
use std::time::{Duration, Instant};
fn main() {
    let samples = std::env::args()
        .nth(1)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10)
        .clamp(1, 1000);
    let mut details = CpuDetailsReader::default();
    let identity = details.read(false, false).identity;
    let start = Instant::now();
    let mut reader = match CpuTemperatureReader::new(identity.model.as_deref()) {
        Ok(reader) => reader,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"status":"unavailable", "code":format!("{:?}",error.code()), "diagnostic":error.diagnostic(), "coldMicros":start.elapsed().as_micros()})
            );
            return;
        }
    };
    let cold = start.elapsed().as_micros();
    for _ in 0..samples {
        let start = Instant::now();
        let value = reader.read();
        println!(
            "{}",
            serde_json::json!({"coldMicros":cold,"readMicros":start.elapsed().as_micros(),"value":value.as_ref().ok(), "error":value.as_ref().err().map(|e|e.diagnostic())})
        );
        std::thread::sleep(Duration::from_secs(1));
    }
}
