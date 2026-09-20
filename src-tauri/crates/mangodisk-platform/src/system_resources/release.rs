//! Native reclamation with explicit target exclusions. Passive sampling never releases memory.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use crate::PlatformResult;

#[derive(Debug, Default)]
pub struct ReleaseOptions {
    pub excluded_paths: Vec<String>,
    pub skip_foreground: bool,
}

pub fn release_memory() -> PlatformResult<()> {
    release_memory_with(&ReleaseOptions::default())
}

pub fn release_memory_with(options: &ReleaseOptions) -> PlatformResult<()> {
    #[cfg(target_os = "macos")]
    {
        if !options.excluded_paths.is_empty() || options.skip_foreground {
            return Err(crate::PlatformError::new(
                crate::PlatformErrorCode::Unsupported,
                "process exclusions are unavailable",
            ));
        }
        macos::release()
    }
    #[cfg(windows)]
    {
        windows::release(options)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = options;
        Err(crate::PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "memory reclamation is unavailable",
        ))
    }
}
