//! The existing companion loads an embedded, architecture-matched XAML adapter.
//! Cache placement avoids locking installer/update files inside Explorer. An idle
//! adapter owns no strong XAML references or polling timer, but can stay mapped
//! until Explorer exits. The content key reuses one cache file per binary build.
use super::{
    layout::Bounds,
    reservation::{Failure, Request, Stage},
};
use std::{os::windows::ffi::OsStrExt, path::PathBuf};
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};

type Apply = unsafe extern "system" fn(u32, u32, u32, u32, *const u16, *mut RECT) -> i32;
type Release = unsafe extern "system" fn();

pub struct Lease {
    module: HMODULE,
    apply: Apply,
    release: Release,
    log_file: Vec<u16>,
}
impl Lease {
    pub fn new(log_file: &std::path::Path) -> Result<Self, Failure> {
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/taskbar_xaml.dll"));
        let key = blake3::hash(bytes).to_hex();
        let cache = log_file
            .parent()
            .and_then(|p| p.parent())
            .ok_or(Failure::new(Stage::Xaml, 0))?
            .join("taskbar-layout");
        std::fs::create_dir_all(&cache).map_err(io_failure)?;
        let path = cache.join(format!("xaml-{key}.dll"));
        // Only remove adapter files from older builds; a mapped file is left for
        // the next startup after Explorer releases it. Never replace mapped code.
        if let Ok(files) = std::fs::read_dir(&cache) {
            for entry in files.flatten() {
                let old = entry.path();
                if old != path
                    && old.extension().is_some_and(|e| e == "dll")
                    && old
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("xaml-"))
                {
                    let _ = std::fs::remove_file(old);
                }
            }
        }
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            let temporary: PathBuf = cache.join(format!("xaml-{}.tmp", std::process::id()));
            std::fs::write(&temporary, bytes).map_err(io_failure)?;
            std::fs::rename(&temporary, &path).map_err(io_failure)?;
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let module = LoadLibraryW(path.as_ptr());
            if module.is_null() {
                return Err(Failure::new(Stage::Xaml, GetLastError()));
            }
            let apply = GetProcAddress(module, c"MangoTaskbarApply".as_ptr().cast());
            let release = GetProcAddress(module, c"MangoTaskbarRelease".as_ptr().cast());
            let (Some(apply), Some(release)) = (apply, release) else {
                FreeLibrary(module);
                return Err(Failure::new(Stage::Xaml, ERROR_PROC_NOT_FOUND));
            };
            Ok(Self {
                module,
                apply: std::mem::transmute::<unsafe extern "system" fn() -> isize, Apply>(apply),
                release: std::mem::transmute::<unsafe extern "system" fn() -> isize, Release>(
                    release,
                ),
                log_file: log_file.as_os_str().encode_wide().chain(Some(0)).collect(),
            })
        }
    }
    pub fn apply(&self, request: Request) -> Result<Bounds, Failure> {
        let mut bounds = RECT::default();
        let code = unsafe {
            (self.apply)(
                request.width as u32,
                request.height as u32,
                request.gap as u32,
                match (request.edge, super::position::read_environment()) {
                    (
                        super::position::Edge::Right,
                        super::position::Environment::Windows11Centered,
                    ) => 3,
                    (super::position::Edge::Right, _) => 0,
                    (_, super::position::Environment::Windows11Centered) => 2,
                    _ => 1,
                },
                self.log_file.as_ptr(),
                &mut bounds,
            )
        };
        match code as u32 {
            0 => Ok(Bounds {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            }),
            0x8000000a => Err(Failure::new(Stage::Geometry, 0)), // E_PENDING: asynchronous attach.
            0x8007007a => Err(Failure::new(Stage::Space, code as u32)),
            _ => Err(Failure::new(Stage::Xaml, code as u32)),
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        unsafe {
            (self.release)();
            FreeLibrary(self.module);
        }
    }
}
fn io_failure(error: std::io::Error) -> Failure {
    Failure::new(Stage::Xaml, error.raw_os_error().unwrap_or(0) as u32)
}
