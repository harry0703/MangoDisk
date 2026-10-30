//! Native resource sampling. Each sensor owns its OS handles and refresh policy;
//! product aggregation and desktop scheduling stay in their respective layers.
#[cfg(target_os = "macos")]
mod application_metadata_macos;
pub mod battery;
pub mod cpu;
pub mod disk;
pub mod disk_health;
pub mod disk_io;
#[cfg(any(target_os = "macos", windows))]
mod disk_smart;
pub mod disk_temperature;
pub mod fans;
pub mod gpu;
pub mod memory;
#[cfg(target_os = "macos")]
mod memory_macos;
pub mod network;
pub mod power;
pub mod process_cpu;
#[cfg(target_os = "macos")]
mod process_cpu_macos;
#[cfg(target_os = "macos")]
mod process_snapshot_macos;
#[cfg(windows)]
mod process_snapshot_windows;
pub mod release;
mod temperature_retry;

#[cfg(windows)]
mod process_image_windows;

#[cfg(target_os = "macos")]
mod macos_smc;
#[cfg(target_os = "macos")]
mod temperature_samples;

pub mod component_power;
#[cfg(target_os = "macos")]
mod macos_hid;
