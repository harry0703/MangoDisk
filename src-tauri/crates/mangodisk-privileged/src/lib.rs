//! Windows capability transport and its independently installed service host.
pub mod artifacts;
pub mod signature_verification;
#[cfg(windows)]
pub mod windows;

#[cfg(any(windows, test))]
mod replacement;

#[cfg(any(windows, test))]
mod cpu_cache;

#[cfg(any(windows, test))]
pub mod cpu_temperature;
