//! Compare raw PID measurements and grouped publication from the exact same native sample.
use mangodisk_core::system_resources::memory_sampling::MemorySamplingService;
use mangodisk_platform::{
    system_resources::memory::{MemorySampler, MemorySource, NativeMemorySnapshot},
    PlatformResult,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
struct Source(Option<NativeMemorySnapshot>);
impl MemorySource for Source {
    fn sample(&mut self, _: bool) -> PlatformResult<NativeMemorySnapshot> {
        Ok(self
            .0
            .take()
            .expect("the diagnostic snapshot is consumed once"))
    }
}
fn main() {
    let mut sampler = MemorySampler::default();
    let started = Instant::now();
    let raw = sampler
        .sample(true)
        .expect("native memory snapshot is readable");
    let elapsed_us = started.elapsed().as_micros();
    let sampled_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let processes = raw
        .processes
        .as_ref()
        .unwrap()
        .iter()
        .map(|row| serde_json::json!({"pid":row.pid,"name":row.name,"usedBytes":row.used_bytes}))
        .collect::<Vec<_>>();
    let snapshot = MemorySamplingService::new(Source(Some(raw)))
        .sample(true, sampled_at_ms)
        .unwrap();
    println!(
        "{}",
        serde_json::json!({"snapshot":snapshot,"nativeProcesses":processes,"sampleElapsedMicros":elapsed_us})
    );
}
