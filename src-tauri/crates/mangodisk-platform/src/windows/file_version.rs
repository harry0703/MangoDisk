use std::{ffi::c_void, path::Path, slice};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_core::HSTRING;

const MAX_VERSION_RESOURCE_BYTES: u32 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct FileVersionMetadata {
    pub product_name: Option<String>,
    pub description: Option<String>,
    pub company_name: Option<String>,
    pub product_version: Option<String>,
}

pub(crate) fn file_version_metadata(path: &Path) -> Option<FileVersionMetadata> {
    if !path.is_file() {
        return None;
    }
    let path = HSTRING::from(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 || size > MAX_VERSION_RESOURCE_BYTES {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, buffer.as_mut_ptr().cast()) }.ok()?;
    let translations = version_translations(&buffer);
    let (language, code_page) = translations.first().copied().unwrap_or((0x0409, 0x04b0));
    Some(FileVersionMetadata {
        product_name: version_string(&buffer, language, code_page, "ProductName"),
        description: version_string(&buffer, language, code_page, "FileDescription"),
        company_name: version_string(&buffer, language, code_page, "CompanyName"),
        product_version: version_string(&buffer, language, code_page, "ProductVersion"),
    })
}

fn version_translations(buffer: &[u8]) -> Vec<(u16, u16)> {
    let query = HSTRING::from(r"\VarFileInfo\Translation");
    let mut pointer = std::ptr::null_mut::<c_void>();
    let mut byte_length = 0u32;
    if !unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            &query,
            &mut pointer,
            &mut byte_length,
        )
    }
    .as_bool()
        || pointer.is_null()
        || byte_length < 4
    {
        return Vec::new();
    }
    let words = unsafe { slice::from_raw_parts(pointer.cast::<u16>(), byte_length as usize / 2) };
    words
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn version_string(buffer: &[u8], language: u16, code_page: u16, key: &str) -> Option<String> {
    let query = HSTRING::from(format!(
        r"\StringFileInfo\{language:04x}{code_page:04x}\{key}"
    ));
    let mut pointer = std::ptr::null_mut::<c_void>();
    let mut character_length = 0u32;
    if !unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            &query,
            &mut pointer,
            &mut character_length,
        )
    }
    .as_bool()
        || pointer.is_null()
        || character_length == 0
    {
        return None;
    }
    let value = unsafe {
        slice::from_raw_parts(
            pointer.cast::<u16>(),
            character_length.saturating_sub(1) as usize,
        )
    };
    let value = String::from_utf16_lossy(value).trim().to_owned();
    (!value.is_empty()).then_some(value)
}
