use super::{unavailable, DeviceCounters};
use std::{collections::BTreeMap, mem::size_of, ptr};
use windows_sys::Win32::System::Performance::*;

#[derive(Default)]
pub struct DiskIoReader {
    query: Option<Query>,
}
struct Query {
    handle: PDH_HQUERY,
    read: PDH_HCOUNTER,
    write: PDH_HCOUNTER,
}
impl Drop for Query {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}
impl Query {
    fn open() -> crate::PlatformResult<Self> {
        let mut handle = ptr::null_mut();
        if unsafe { PdhOpenQueryW(ptr::null(), 0, &mut handle) } != 0 {
            return Err(unavailable());
        }
        let mut query = Self {
            handle,
            read: ptr::null_mut(),
            write: ptr::null_mut(),
        };
        for (suffix, counter) in [("Read", &mut query.read), ("Write", &mut query.write)] {
            let path: Vec<u16> = format!("\\PhysicalDisk(*)\\Disk {suffix} Bytes/sec")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // English counter paths work with localized Windows installations.
            if unsafe { PdhAddEnglishCounterW(handle, path.as_ptr(), 0, counter) } != 0 {
                return Err(unavailable());
            }
        }
        Ok(query)
    }
}
fn values(counter: PDH_HCOUNTER) -> crate::PlatformResult<BTreeMap<String, u64>> {
    let (mut bytes, mut count) = (0, 0);
    if unsafe { PdhGetRawCounterArrayW(counter, &mut bytes, &mut count, ptr::null_mut()) }
        != PDH_MORE_DATA
        || bytes == 0
        || bytes > 4_194_304
    {
        return Err(unavailable());
    }
    // u64 storage provides the alignment required by PDH_RAW_COUNTER_ITEM_W.
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
    if unsafe {
        PdhGetRawCounterArrayW(counter, &mut bytes, &mut count, buffer.as_mut_ptr().cast())
    } != 0
        || count as usize > buffer.len() * 8 / size_of::<PDH_RAW_COUNTER_ITEM_W>()
    {
        return Err(unavailable());
    }
    let items = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<PDH_RAW_COUNTER_ITEM_W>(),
            count as usize,
        )
    };
    let mut result = BTreeMap::new();
    for item in items {
        // Names live inside PDH's returned buffer; bound every read to that allocation.
        let start = item.szName as usize;
        let end = buffer.as_ptr() as usize + buffer.len() * 8;
        if start < buffer.as_ptr() as usize || start >= end || !start.is_multiple_of(2) {
            return Err(unavailable());
        }
        let name = unsafe { std::slice::from_raw_parts(item.szName, (end - start) / 2) };
        let length = name.iter().position(|c| *c == 0).ok_or_else(unavailable)?;
        let name = String::from_utf16_lossy(&name[..length]);
        if name == "_Total" {
            continue;
        }
        if !matches!(
            item.RawValue.CStatus,
            PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
        ) {
            return Err(unavailable());
        }
        result.insert(
            name,
            item.RawValue
                .FirstValue
                .try_into()
                .map_err(|_| unavailable())?,
        );
    }
    Ok(result)
}
impl DiskIoReader {
    pub fn read(&mut self) -> crate::PlatformResult<Vec<DeviceCounters>> {
        if self.query.is_none() {
            self.query = Some(Query::open()?);
        }
        let result = (|| {
            let query = self.query.as_ref().expect("initialized query");
            if unsafe { PdhCollectQueryData(query.handle) } != 0 {
                return Err(unavailable());
            }
            let read = values(query.read)?;
            let mut write = values(query.write)?;
            if read.is_empty() || read.len() != write.len() {
                return Err(unavailable());
            }
            read.into_iter()
                .map(|(id, read_bytes)| {
                    let written_bytes = write.remove(&id).ok_or_else(unavailable)?;
                    Ok(DeviceCounters {
                        id,
                        read_bytes,
                        written_bytes,
                    })
                })
                .collect()
        })();
        // A failed provider query is retried with fresh handles on the next sample.
        if result.is_err() {
            self.query = None;
        }
        result
    }
}
