//! System resource snapshots and explicit memory reclamation, independent of desktop UI.
mod application_groups;
mod application_identity;
pub mod cpu;
pub mod disk;
pub mod disk_io;
pub mod fans;
pub mod gpu;
mod memory;
pub mod memory_sampling;
pub mod metrics;
pub mod models;
pub mod network;
pub mod network_addresses;
pub mod network_traffic;
pub mod process_cpu;
pub mod process_memory;
pub mod readings;
pub mod release;
pub mod release_policy;

mod gpu_history;
#[cfg(test)]
mod gpu_selection_tests;
mod gpu_temperature;

#[cfg(test)]
mod disk_health_tests;
