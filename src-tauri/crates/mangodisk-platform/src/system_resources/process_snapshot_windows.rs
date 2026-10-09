//! Batch process measurements include system processes without privileged handles.
use super::process_cpu::ProcessCpuCounter;
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use std::{
    ffi::c_void,
    mem::size_of,
    sync::{Arc, Mutex, OnceLock},
};
use windows_sys::Win32::{
    Foundation::{STATUS_INFO_LENGTH_MISMATCH, STATUS_SUCCESS},
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        WindowsProgramming::SYSTEM_PROCESS_INFORMATION,
    },
};

type QuerySystemInformation = unsafe extern "system" fn(i32, *mut c_void, u32, *mut u32) -> i32;
const SYSTEM_PROCESS_INFORMATION_CLASS: i32 = 5;
const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const BUFFER_MARGIN: usize = 64 * 1024;

pub(super) struct ProcessReading {
    pub counter: ProcessCpuCounter,
    pub private_working_set_bytes: u64,
}

#[derive(Default)]
pub(super) struct ProcessSnapshotReader {
    buffer: Vec<u64>,
    images: Option<Arc<Mutex<super::process_image_windows::ImageCache>>>,
    fallback: bool,
}
impl ProcessSnapshotReader {
    pub(super) fn read(&mut self) -> Option<Vec<ProcessReading>> {
        match self.query() {
            Ok(mut rows) => {
                self.images
                    .get_or_insert_with(super::process_image_windows::shared_cache)
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .attach(rows.iter_mut().map(|row| &mut row.counter));
                if self.fallback {
                    log::info!(
                        "windows_process_snapshot_recovered processes={}",
                        rows.len()
                    );
                }
                self.fallback = false;
                Some(rows)
            }
            Err(error) => {
                if !self.fallback {
                    log::warn!(
                        "windows_process_snapshot_unavailable code={:?} error={}",
                        error.code(),
                        crate::diagnostics::text(&error)
                    );
                }
                self.fallback = true;
                None
            }
        }
    }

    fn query(&mut self) -> PlatformResult<Vec<ProcessReading>> {
        let query = query_function().ok_or_else(|| invalid("native batch API unavailable"))?;
        let before = super::process_cpu::public_windows_counters(std::process::id())
            .ok_or_else(|| invalid("public validation of native batch counters failed"))?;
        if self.buffer.is_empty() {
            self.grow(BUFFER_MARGIN)?;
        }
        // A growing process/thread set can invalidate ReturnLength; bound both retries and memory.
        for _ in 0..4 {
            let mut used = 0u32;
            let status = unsafe {
                query(
                    SYSTEM_PROCESS_INFORMATION_CLASS,
                    self.buffer.as_mut_ptr().cast(),
                    (self.buffer.len() * size_of::<u64>()) as u32,
                    &mut used,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                self.grow(used as usize)?;
                continue;
            }
            if status != STATUS_SUCCESS {
                return Err(invalid(&format!(
                    "native batch query NTSTATUS={status:#010x}"
                )));
            }
            let capacity = self.buffer.len() * size_of::<u64>();
            if used as usize > capacity {
                return Err(invalid("native batch returned an oversized length"));
            }
            // The aligned, initialized allocation remains alive while ImageName pointers are checked.
            let bytes = unsafe {
                std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), used as usize)
            };
            let rows = parse_rows(bytes)?;
            validate_native_layout(&rows, before)?;
            return Ok(rows);
        }
        Err(invalid("native batch exceeded retry limit"))
    }

    fn grow(&mut self, required: usize) -> PlatformResult<()> {
        if required > MAX_BUFFER_BYTES {
            return Err(invalid("native batch exceeded memory limit"));
        }
        let current = self.buffer.len() * size_of::<u64>();
        let target = required
            .saturating_add(BUFFER_MARGIN)
            .max(current.saturating_mul(2))
            .min(MAX_BUFFER_BYTES);
        if target <= current {
            return Err(invalid("native batch cannot grow further"));
        }
        let words = target.div_ceil(size_of::<u64>());
        self.buffer
            .try_reserve_exact(words - self.buffer.len())
            .map_err(|error| invalid(&format!("native batch allocation failed: {error}")))?;
        self.buffer.resize(words, 0);
        Ok(())
    }
}

fn query_function() -> Option<QuerySystemInformation> {
    static QUERY: OnceLock<Option<QuerySystemInformation>> = OnceLock::new();
    *QUERY.get_or_init(|| unsafe {
        // Ntdll is already loaded by Windows. Resolve dynamically, as required for this native API;
        // do not load a library from the working directory or make it a mandatory import.
        let module = GetModuleHandleW(windows_sys::w!("ntdll.dll"));
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, windows_sys::s!("NtQuerySystemInformation")).map(|function| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, QuerySystemInformation>(
                function,
            )
        })
    })
}

fn parse_rows(bytes: &[u8]) -> PlatformResult<Vec<ProcessReading>> {
    let mut rows = Vec::new();
    let mut offset = 0usize;
    loop {
        let header_end = offset
            .checked_add(size_of::<SYSTEM_PROCESS_INFORMATION>())
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("native batch header is truncated"))?;
        let header = unsafe {
            bytes
                .as_ptr()
                .add(offset)
                .cast::<SYSTEM_PROCESS_INFORMATION>()
                .read_unaligned()
        };
        let pid = u32::try_from(header.UniqueProcessId as usize)
            .map_err(|_| invalid("native batch PID is invalid"))?;
        let next = header.NextEntryOffset as usize;
        let row_end = if next == 0 {
            bytes.len()
        } else {
            offset
                .checked_add(next)
                .filter(|end| *end >= header_end && *end <= bytes.len())
                .ok_or_else(|| invalid("native batch offset is invalid"))?
        };
        if pid != 0 {
            // The SDK reserves this prefix. Windows 10/11's native layout stores CreateTime,
            // UserTime and KernelTime at offsets 32/40/48. Verify it against our own public
            // GetProcessTimes result each sample and fall back if this OS contract changes.
            let time = |index| {
                let value =
                    i64::from_le_bytes(header.Reserved1[index..index + 8].try_into().unwrap());
                u64::try_from(value).map_err(|_| invalid("native batch CPU time is negative"))
            };
            let started_at = time(24)?;
            let cpu_time_ms = time(32)?.saturating_add(time(40)?) / 10_000;
            let name_bytes = checked_name(bytes, offset, row_end, &header)?;
            let name = String::from_utf16_lossy(
                &name_bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            );
            if name.trim().is_empty() {
                return Err(invalid("native batch process name is empty"));
            }
            // Windows 10/11 store WorkingSetPrivateSize at offset 8, distinct from
            // PrivatePageCount (commit). CPU identity validates this native header;
            // reject memory that violates the documented working-set relationship.
            let private_working_set_bytes =
                u64::from_le_bytes(header.Reserved1[..8].try_into().unwrap());
            if private_working_set_bytes > header.WorkingSetSize as u64 {
                return Err(invalid(
                    "native private working set exceeds total working set",
                ));
            }
            rows.push(ProcessReading {
                private_working_set_bytes,
                counter: ProcessCpuCounter {
                    pid,
                    started_at,
                    name,
                    executable: None,
                    location_status: super::process_cpu::ProcessLocationStatus::Unavailable,
                    cpu_time_ms: Some(cpu_time_ms),
                },
            });
        }
        if next == 0 {
            return Ok(rows);
        }
        offset = row_end;
    }
}

fn checked_name<'a>(
    bytes: &'a [u8],
    row_start: usize,
    row_end: usize,
    header: &SYSTEM_PROCESS_INFORMATION,
) -> PlatformResult<&'a [u8]> {
    let length = header.ImageName.Length as usize;
    if length == 0
        || !length.is_multiple_of(2)
        || header.ImageName.Length > header.ImageName.MaximumLength
    {
        return Err(invalid("native batch name length is invalid"));
    }
    let start = (header.ImageName.Buffer as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .filter(|start| *start >= row_start + size_of::<SYSTEM_PROCESS_INFORMATION>())
        .ok_or_else(|| invalid("native batch name pointer is outside its entry"))?;
    let end = start
        .checked_add(length)
        .filter(|end| *end <= row_end)
        .ok_or_else(|| invalid("native batch name is truncated"))?;
    Ok(&bytes[start..end])
}

fn validate_native_layout(rows: &[ProcessReading], before: (u64, u64)) -> PlatformResult<()> {
    let pid = std::process::id();
    let own = rows
        .iter()
        .map(|row| &row.counter)
        .find(|row| row.pid == pid)
        .ok_or_else(|| invalid("native batch omitted the sampling process"))?;
    let (created, public_ms) = super::process_cpu::public_windows_counters(pid)
        .ok_or_else(|| invalid("public validation of native batch counters failed"))?;
    // Both queries return milliseconds truncated from 100 ns. Bracket the native snapshot
    // with public reads, allowing one millisecond for the independent counter rounding.
    if own.started_at != created
        || created != before.0
        || own
            .cpu_time_ms
            .is_none_or(|ms| ms < before.1.saturating_sub(1) || ms > public_ms.saturating_add(1))
    {
        return Err(invalid(
            "native batch CPU layout does not match public process times",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> PlatformError {
    PlatformError::new(PlatformErrorCode::OperationFailed, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(pid: u32, name: &str) -> Vec<u64> {
        let text = name.encode_utf16().collect::<Vec<_>>();
        let header_size = size_of::<SYSTEM_PROCESS_INFORMATION>();
        let mut words = vec![0u64; (header_size + text.len() * 2).div_ceil(8)];
        let mut header = SYSTEM_PROCESS_INFORMATION {
            UniqueProcessId: pid as usize as _,
            ..Default::default()
        };
        for (index, value) in [
            (24, 134_352_369_483_566_020i64),
            (32, 50_000_000),
            (40, 25_000_000),
        ] {
            header.Reserved1[index..index + 8].copy_from_slice(&value.to_le_bytes());
        }
        header.ImageName.Length = (text.len() * 2) as u16;
        header.ImageName.MaximumLength = header.ImageName.Length;
        header.ImageName.Buffer =
            unsafe { words.as_mut_ptr().cast::<u8>().add(header_size).cast() };
        unsafe {
            words
                .as_mut_ptr()
                .cast::<SYSTEM_PROCESS_INFORMATION>()
                .write(header);
            std::ptr::copy_nonoverlapping(text.as_ptr(), header.ImageName.Buffer, text.len());
        }
        words
    }

    fn bytes(words: &[u64]) -> &[u8] {
        unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), words.len() * 8) }
    }

    #[test]
    fn batch_parsing_preserves_cpu_units_and_excludes_idle_capacity() {
        let value = fixture(4, "System");
        let reading = parse_rows(bytes(&value)).unwrap().remove(0);
        let row = reading.counter;
        assert_eq!(row.pid, 4);
        assert_eq!(row.name, "System");
        assert_eq!(row.started_at, 134_352_369_483_566_020);
        assert_eq!(row.cpu_time_ms, Some(7500));
        assert!(parse_rows(bytes(&fixture(0, ""))).unwrap().is_empty());
        assert_eq!(
            parse_rows(bytes(&fixture(9, "\u{7cfb}\u{7edf}\u{7a0b}\u{5e8f}"))).unwrap()[0]
                .counter
                .name,
            "\u{7cfb}\u{7edf}\u{7a0b}\u{5e8f}"
        );
    }

    #[test]
    fn private_working_set_is_not_commit_and_impossible_counters_are_rejected() {
        let mut value = fixture(4, "System");
        let header = unsafe { &mut *value.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>() };
        header.Reserved1[..8].copy_from_slice(&1024u64.to_le_bytes());
        header.WorkingSetSize = 2048;
        header.PrivatePageCount = 8192;
        assert_eq!(
            parse_rows(bytes(&value)).unwrap()[0].private_working_set_bytes,
            1024
        );
        let header = unsafe { &mut *value.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>() };
        header.WorkingSetSize = 512;
        assert!(parse_rows(bytes(&value)).is_err());
    }

    #[test]
    fn batch_parser_rejects_truncated_offsets_pointers_names_and_negative_times() {
        assert!(parse_rows(&[0u8; 8]).is_err());
        for invalid_field in 0..6 {
            let mut value = fixture(4, "System");
            let header = unsafe { &mut *value.as_mut_ptr().cast::<SYSTEM_PROCESS_INFORMATION>() };
            match invalid_field {
                0 => header.NextEntryOffset = 1,
                1 => header.NextEntryOffset = (value.len() * 8) as u32,
                2 => header.ImageName.Buffer = std::ptr::null_mut(),
                3 => header.ImageName.Length = 1,
                4 => header.ImageName.Length = u16::MAX - 1,
                5 => header.Reserved1[32..40].copy_from_slice(&(-1i64).to_le_bytes()),
                _ => unreachable!(),
            }
            assert!(
                parse_rows(bytes(&value)).is_err(),
                "invalid field {invalid_field}"
            );
        }
    }

    #[test]
    fn batch_allocation_is_bounded_and_incompatible_layout_is_rejected() {
        let mut reader = ProcessSnapshotReader::default();
        assert!(reader.grow(MAX_BUFFER_BYTES + 1).is_err());
        assert!(reader.buffer.is_empty());
        reader.grow(BUFFER_MARGIN).unwrap();
        let first = reader.buffer.len();
        reader.grow(0).unwrap();
        assert!(reader.buffer.len() > first);
        let mut rows = parse_rows(bytes(&fixture(std::process::id(), "test"))).unwrap();
        rows[0].counter.started_at = 0;
        assert!(validate_native_layout(&rows, (0, 0)).is_err());
    }

    #[test]
    fn concurrent_readers_keep_fresh_counters_and_share_the_same_creation_identity() {
        let pid = std::process::id();
        let created = super::super::process_cpu::public_windows_counters(pid)
            .expect("this process must expose public counters")
            .0;
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let readers = (0..2)
                .map(|_| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut reader = ProcessSnapshotReader::default();
                        barrier.wait();
                        for _ in 0..4 {
                            let rows = reader
                                .read()
                                .expect("concurrent snapshot must remain valid");
                            let own = &rows
                                .iter()
                                .find(|row| row.counter.pid == pid)
                                .unwrap()
                                .counter;
                            assert_eq!(own.started_at, created);
                            assert!(own.cpu_time_ms.is_some());
                            assert!(own.executable.is_some());
                        }
                    })
                })
                .collect::<Vec<_>>();
            for reader in readers {
                reader
                    .join()
                    .expect("native reader must finish without a deadlock");
            }
        });
    }

    #[test]
    fn native_batch_includes_system_counters_and_matches_public_process_times() {
        let mut reader = ProcessSnapshotReader::default();
        let rows = reader
            .query()
            .expect("native CPU layout should match public process times");
        assert!(rows.iter().any(
            |row| row.counter.pid == 4 && row.counter.cpu_time_ms.is_some_and(|time| time > 0)
        ));
        assert!(rows.iter().any(|row| row.counter.pid == std::process::id()));
        assert!(rows
            .iter()
            .all(|row| row.counter.pid != 0 && row.counter.cpu_time_ms.is_some()));
        let capacity = reader.buffer.capacity();
        reader.query().unwrap();
        assert_eq!(reader.buffer.capacity(), capacity);
        let mut sampler = super::super::process_cpu::ProcessCpuSampler::default();
        use super::super::process_cpu::ProcessCpuSource;
        let snapshot = sampler.sample().unwrap();
        assert!(snapshot
            .processes
            .iter()
            .any(|row| row.pid == 4 && row.cpu_time_ms.is_some()));
        assert!(snapshot
            .processes
            .iter()
            .all(|row| row.pid != 0 && row.cpu_time_ms.is_some()));
        assert!(snapshot
            .processes
            .iter()
            .any(|row| row.pid == std::process::id() && row.executable.is_some()));
    }
}
