use std::{
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::OsStringExt,
    path::{Component, Path},
    ptr,
};

use windows_sys::Win32::{
    Foundation::{ERROR_HANDLE_EOF, HANDLE},
    Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT},
    System::Ioctl::{
        FILE_LAYOUT_ENTRY, FILE_LAYOUT_NAME_ENTRY_DOS, FILE_LAYOUT_NAME_ENTRY_PRIMARY,
        FSCTL_QUERY_FILE_LAYOUT, QUERY_FILE_LAYOUT_FILTER_TYPE_NONE,
        QUERY_FILE_LAYOUT_INCLUDE_NAMES, QUERY_FILE_LAYOUT_INCLUDE_STREAMS,
        QUERY_FILE_LAYOUT_INCLUDE_STREAMS_WITH_NO_CLUSTERS_ALLOCATED, QUERY_FILE_LAYOUT_INPUT,
        QUERY_FILE_LAYOUT_OUTPUT, QUERY_FILE_LAYOUT_RESTART,
    },
};

use crate::windows::is_remote_placeholder_attributes;
use crate::windows::native_io::{device_io_control, read_copy, AlignedBuffer, RawLayoutValue};
use crate::FileSpaceUsage;

#[cfg(test)]
use super::DirectoryTotals;
use super::{
    CandidateRecord, CandidateSizeMetric, DirectoryBoundary, DirectoryNode, FileNameLink,
    LayoutCollection, LayoutCollectionMode, LayoutScanError, MAX_DEFERRED_PATHS,
};

const OUTPUT_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const NTFS_DATA_ATTRIBUTE: u32 = 0x80;
const SUPPORTED_FILE_LAYOUT_VERSION: u32 = 1;
const SUPPORTED_STREAM_LAYOUT_VERSION: u32 = 1;
const RESERVED_NTFS_RECORD_COUNT: u64 = 24;
const FILE_REFERENCE_NUMBER_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
// Some system component files expose more than 64 layout streams. This limit
// bounds loops caused by corrupt offset chains rather than ordinary files.
// 4096 remains far below the number of one-byte offsets a page could forge,
// while covering realistic NTFS hard-link and alternate-stream counts.
const MAX_NAME_CHAIN_LENGTH: usize = 4_096;
const MAX_STREAM_CHAIN_LENGTH: usize = 4_096;
const MAX_DIRECTORY_RECORDS: usize = 2_000_000;
const MAX_DEFERRED_CANDIDATES: usize = 100_000;
// The kernel handle owns FSCTL pagination and exposes no comparable userspace
// cursor. A cumulative limit rejects pathological repeated pages and bounds
// worst-case CPU and memory; very large volumes fall back to Win32 traversal.
const MAX_LAYOUT_ENTRIES: u64 = 10_000_000;

pub(super) fn enumerate_layout(
    volume: HANDLE,
    minimum_bytes: u64,
    mode: LayoutCollectionMode,
    candidate_size_metric: CandidateSizeMetric,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<LayoutCollection, LayoutScanError> {
    let mut input = QUERY_FILE_LAYOUT_INPUT::default();
    input.Anonymous.FilterEntryCount = 0;
    input.Flags = QUERY_FILE_LAYOUT_RESTART
        | QUERY_FILE_LAYOUT_INCLUDE_NAMES
        | QUERY_FILE_LAYOUT_INCLUDE_STREAMS
        | QUERY_FILE_LAYOUT_INCLUDE_STREAMS_WITH_NO_CLUSTERS_ALLOCATED;
    input.FilterType = QUERY_FILE_LAYOUT_FILTER_TYPE_NONE;

    let mut output_buffer = AlignedBuffer::new(OUTPUT_BUFFER_BYTES);
    let mut collection = LayoutCollection::default();
    loop {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        let returned = match device_io_control(
            volume,
            FSCTL_QUERY_FILE_LAYOUT,
            ptr::from_ref(&input).cast(),
            size_of::<QUERY_FILE_LAYOUT_INPUT>(),
            output_buffer.as_mut_ptr(),
            output_buffer.capacity_bytes(),
        ) {
            Ok(returned) => returned,
            Err(code) if code == ERROR_HANDLE_EOF => break,
            Err(code) => {
                return Err(LayoutScanError::Platform(format!(
                    "query_file_layout:{code}"
                )));
            }
        };
        if returned == 0 {
            return Err(LayoutScanError::Platform(
                "query_file_layout_empty_page".to_string(),
            ));
        }
        let page = output_buffer.as_bytes(returned).ok_or_else(|| {
            LayoutScanError::Platform("query_file_layout_output_out_of_bounds".to_string())
        })?;
        collection.page_count = collection.page_count.saturating_add(1);
        collection.returned_bytes = collection.returned_bytes.saturating_add(returned as u64);
        parse_layout_page(
            page,
            minimum_bytes,
            mode,
            candidate_size_metric,
            is_cancelled,
            &mut collection,
        )?;
        input.Flags &= !QUERY_FILE_LAYOUT_RESTART;
    }
    if collection.remote_file_count > 0 || collection.remote_directory_count > 0 {
        log::info!(
            "windows_file_layout_remote_placeholders_skipped mode={} file_count={} directory_count={} total_count={}",
            layout_mode_code(mode),
            collection.remote_file_count,
            collection.remote_directory_count,
            collection
                .remote_file_count
                .saturating_add(collection.remote_directory_count)
        );
    }
    Ok(collection)
}

fn layout_mode_code(mode: LayoutCollectionMode) -> &'static str {
    match mode {
        LayoutCollectionMode::CandidatesOnly => "candidates",
        LayoutCollectionMode::FullAnalysis => "analysis",
        LayoutCollectionMode::AnalysisWithFiles => "analysis_with_files",
    }
}

fn parse_layout_page(
    bytes: &[u8],
    minimum_bytes: u64,
    mode: LayoutCollectionMode,
    candidate_size_metric: CandidateSizeMetric,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    collection: &mut LayoutCollection,
) -> Result<(), LayoutScanError> {
    let output = read_copy::<QUERY_FILE_LAYOUT_OUTPUT>(bytes, 0)
        .ok_or_else(|| LayoutScanError::Platform("layout_header_truncated".to_string()))?;
    if output.FileEntryCount == 0 {
        return Err(LayoutScanError::Platform(
            "layout_page_has_no_entries".to_string(),
        ));
    }
    let mut offset = output.FirstFileOffset as usize;
    if offset < size_of::<QUERY_FILE_LAYOUT_OUTPUT>() {
        return Err(LayoutScanError::Platform(
            "layout_first_file_offset_overlaps_header".to_string(),
        ));
    }
    let mut parsed = 0u32;
    while parsed < output.FileEntryCount {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        if collection.entry_count >= MAX_LAYOUT_ENTRIES {
            return Err(LayoutScanError::Platform(
                "layout_entry_limit_exceeded".to_string(),
            ));
        }
        let file = read_copy::<FILE_LAYOUT_ENTRY>(bytes, offset)
            .ok_or_else(|| LayoutScanError::Platform("layout_entry_truncated".to_string()))?;
        if file.Version != SUPPORTED_FILE_LAYOUT_VERSION {
            return Err(LayoutScanError::Platform(format!(
                "unsupported_layout_version:{}",
                file.Version
            )));
        }
        let is_last_entry = parsed + 1 == output.FileEntryCount;
        let next_entry_offset = if is_last_entry {
            if file.NextFileOffset != 0 {
                return Err(LayoutScanError::Platform(
                    "layout_last_entry_has_next_offset".to_string(),
                ));
            }
            None
        } else {
            if file.NextFileOffset == 0 {
                return Err(LayoutScanError::Platform(
                    "layout_entry_chain_ended_early".to_string(),
                ));
            }
            if (file.NextFileOffset as usize) < size_of::<FILE_LAYOUT_ENTRY>() {
                return Err(LayoutScanError::Platform(
                    "layout_next_file_offset_overlaps_entry".to_string(),
                ));
            }
            Some(
                checked_relative_offset(offset, file.NextFileOffset as usize, bytes.len())
                    .map_err(LayoutScanError::Platform)?,
            )
        };
        // Name and stream offsets must remain inside the current
        // FILE_LAYOUT_ENTRY. Otherwise integers in the next record could be
        // misinterpreted as part of this record's variable-length chains.
        let entry_bytes = next_entry_offset.map_or(bytes, |end| &bytes[..end]);
        let is_directory = file.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        let is_reparse = file.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        let space_usage = if is_directory {
            None
        } else {
            read_default_data_space_usage(entry_bytes, offset, file.FirstStreamOffset as usize)?
        };
        let candidate_size = space_usage.map(|usage| candidate_size_metric.bytes(usage));
        // Allocation size cannot identify cloud residency: local NTFS resident streams and fully
        // sparse files also have no allocated clusters. Only documented offline/recall attributes
        // are accepted as the content-access boundary, preserving exact local file semantics.
        let is_remote_placeholder = is_remote_placeholder_attributes(file.FileAttributes);
        if is_remote_placeholder {
            if is_directory {
                collection.remote_directory_count =
                    collection.remote_directory_count.saturating_add(1);
            } else {
                collection.remote_file_count = collection.remote_file_count.saturating_add(1);
            }
        }
        if is_directory {
            if let Some(name) =
                read_first_long_name(entry_bytes, offset, file.FirstNameOffset as usize)?
            {
                if !collection
                    .directories
                    .contains_key(&file.FileReferenceNumber)
                    && collection.directories.len() >= MAX_DIRECTORY_RECORDS
                {
                    return Err(LayoutScanError::Platform(
                        "directory_record_limit_exceeded".to_string(),
                    ));
                }
                collection
                    .directories
                    .entry(file.FileReferenceNumber)
                    .or_insert(DirectoryNode {
                        parent_id: name.parent_id,
                        name: name.name,
                        boundary: if is_reserved_ntfs_record(file.FileReferenceNumber) {
                            DirectoryBoundary::Internal
                        } else if is_reparse {
                            DirectoryBoundary::Reparse
                        } else if is_remote_placeholder {
                            DirectoryBoundary::RemotePlaceholder
                        } else {
                            DirectoryBoundary::None
                        },
                    });
            }
        } else if !is_reserved_ntfs_record(file.FileReferenceNumber)
            && (mode != LayoutCollectionMode::CandidatesOnly
                || (!is_reparse
                    && !is_remote_placeholder
                    && candidate_size.is_some_and(|bytes| bytes >= minimum_bytes)))
        {
            // Candidate mode parses names only for files at or above the
            // threshold. Allocating an OsString for every small file would
            // erase the large-file fast path; full analysis needs all names.
            let names = read_long_names(entry_bytes, offset, file.FirstNameOffset as usize)?;
            if !names.is_empty() {
                if mode != LayoutCollectionMode::CandidatesOnly {
                    // Win32 directory enumeration returns one entry for each
                    // hard-link name, so totals also count each name link.
                    // Reparse and remote-only files contribute no bytes but increment the direct
                    // parent's skipped count to match the portable traversal's fail-closed policy.
                    let space_usage =
                        space_usage.unwrap_or_else(|| FileSpaceUsage::logical_only(0));
                    for name in &names {
                        if !collection.direct_totals.contains_key(&name.parent_id)
                            && collection.direct_totals.len() >= MAX_DIRECTORY_RECORDS
                        {
                            return Err(LayoutScanError::Platform(
                                "direct_parent_record_limit_exceeded".to_string(),
                            ));
                        }
                        let totals = collection.direct_totals.entry(name.parent_id).or_default();
                        if is_reparse || is_remote_placeholder {
                            totals.checked_add_skipped()?;
                        } else {
                            totals.checked_add_file(space_usage)?;
                            if mode == LayoutCollectionMode::AnalysisWithFiles
                                && space_usage.allocated_bytes > 0
                                && space_usage.allocated_bytes < minimum_bytes
                            {
                                const LIMIT: usize = 8192;
                                let file = (
                                    space_usage.allocated_bytes,
                                    name.parent_id,
                                    name.name.clone(),
                                    space_usage.logical_bytes,
                                );
                                if collection.analysis_files.len() < LIMIT {
                                    collection.analysis_files.push(std::cmp::Reverse(file));
                                } else if collection
                                    .analysis_files
                                    .peek()
                                    .is_some_and(|smallest| file > smallest.0)
                                {
                                    *collection
                                        .analysis_files
                                        .peek_mut()
                                        .expect("a full candidate heap is nonempty") =
                                        std::cmp::Reverse(file);
                                }
                            }
                        }
                    }
                }
                if !is_reparse
                    && !is_remote_placeholder
                    && candidate_size.is_some_and(|bytes| bytes >= minimum_bytes)
                {
                    if collection.candidates.len() >= MAX_DEFERRED_CANDIDATES {
                        return Err(LayoutScanError::Platform(
                            "candidate_record_limit_exceeded".to_string(),
                        ));
                    }
                    collection.candidate_path_count = collection
                        .candidate_path_count
                        .checked_add(names.len())
                        .ok_or_else(|| {
                            LayoutScanError::Platform("candidate_path_count_overflow".to_string())
                        })?;
                    if collection.candidate_path_count > MAX_DEFERRED_PATHS {
                        return Err(LayoutScanError::Platform(
                            "candidate_path_limit_exceeded".to_string(),
                        ));
                    }
                    collection.candidates.push(CandidateRecord { names });
                }
            }
        }
        collection.entry_count = collection.entry_count.saturating_add(1);
        parsed += 1;
        if let Some(next_entry_offset) = next_entry_offset {
            offset = next_entry_offset;
        } else {
            break;
        }
    }
    Ok(())
}

fn read_first_long_name(
    bytes: &[u8],
    file_offset: usize,
    relative_offset: usize,
) -> Result<Option<FileNameLink>, LayoutScanError> {
    Ok(read_long_names(bytes, file_offset, relative_offset)?
        .into_iter()
        .next())
}

fn read_long_names(
    bytes: &[u8],
    file_offset: usize,
    relative_offset: usize,
) -> Result<Vec<FileNameLink>, LayoutScanError> {
    if relative_offset == 0 {
        return Ok(Vec::new());
    }
    if relative_offset < size_of::<FILE_LAYOUT_ENTRY>() {
        return Err(LayoutScanError::Platform(
            "layout_name_offset_overlaps_entry".to_string(),
        ));
    }
    let mut result = Vec::new();
    let mut name_offset = checked_relative_offset(file_offset, relative_offset, bytes.len())
        .map_err(LayoutScanError::Platform)?;
    for _ in 0..MAX_NAME_CHAIN_LENGTH {
        let entry = read_copy::<FileLayoutNameHeader>(bytes, name_offset)
            .ok_or_else(|| LayoutScanError::Platform("layout_name_truncated".to_string()))?;
        let name_start = name_offset
            .checked_add(size_of::<FileLayoutNameHeader>())
            .ok_or_else(|| LayoutScanError::Platform("layout_name_overflow".to_string()))?;
        // Validate the body even when a DOS-only alias will be discarded.
        // Otherwise a corrupt alias could bypass the length check and make the
        // containing page appear valid.
        let name = read_utf16_os(bytes, name_start, entry.file_name_length as usize)
            .ok_or_else(|| LayoutScanError::Platform("layout_name_invalid".to_string()))?;
        // A name can be both Primary and DOS. Discard only DOS-only aliases;
        // otherwise a real directory whose name already fits 8.3 disappears
        // from the parent chain and forces its descendants onto Win32.
        if entry.flags & FILE_LAYOUT_NAME_ENTRY_DOS == 0
            || entry.flags & FILE_LAYOUT_NAME_ENTRY_PRIMARY != 0
        {
            // Root relationships may appear as "." or "..". They are not
            // joinable link names, so omitting them preserves valid layouts
            // without allowing a join to escape the scan root.
            if name != "." && name != ".." {
                if !is_single_path_component(&name) {
                    return Err(LayoutScanError::Platform(
                        "layout_name_is_not_a_component".to_string(),
                    ));
                }
                result.push(FileNameLink {
                    parent_id: entry.parent_file_reference_number,
                    name,
                });
            }
        }
        if entry.next_name_offset == 0 {
            return Ok(result);
        }
        let minimum_next_offset = size_of::<FileLayoutNameHeader>()
            .checked_add(entry.file_name_length as usize)
            .ok_or_else(|| LayoutScanError::Platform("layout_name_overflow".to_string()))?;
        if (entry.next_name_offset as usize) < minimum_next_offset {
            return Err(LayoutScanError::Platform(
                "layout_next_name_offset_overlaps_entry".to_string(),
            ));
        }
        name_offset =
            checked_relative_offset(name_offset, entry.next_name_offset as usize, bytes.len())
                .map_err(LayoutScanError::Platform)?;
    }
    Err(LayoutScanError::Platform(
        "layout_name_chain_limit_exceeded".to_string(),
    ))
}

fn read_default_data_space_usage(
    bytes: &[u8],
    file_offset: usize,
    relative_offset: usize,
) -> Result<Option<FileSpaceUsage>, LayoutScanError> {
    if relative_offset == 0 {
        return Ok(None);
    }
    if relative_offset < size_of::<FILE_LAYOUT_ENTRY>() {
        return Err(LayoutScanError::Platform(
            "layout_stream_offset_overlaps_entry".to_string(),
        ));
    }
    let mut stream_offset = checked_relative_offset(file_offset, relative_offset, bytes.len())
        .map_err(LayoutScanError::Platform)?;
    for _ in 0..MAX_STREAM_CHAIN_LENGTH {
        let stream = read_copy::<StreamLayoutHeader>(bytes, stream_offset)
            .ok_or_else(|| LayoutScanError::Platform("layout_stream_truncated".to_string()))?;
        if stream.version != SUPPORTED_STREAM_LAYOUT_VERSION {
            return Err(LayoutScanError::Platform(format!(
                "unsupported_stream_layout_version:{}",
                stream.version
            )));
        }
        let identifier_length = stream.stream_identifier_length as usize;
        let identifier_start = stream_offset
            .checked_add(size_of::<StreamLayoutHeader>())
            .ok_or_else(|| LayoutScanError::Platform("layout_stream_overflow".to_string()))?;
        let identifier_end = identifier_start
            .checked_add(identifier_length)
            .ok_or_else(|| LayoutScanError::Platform("layout_stream_overflow".to_string()))?;
        if !identifier_length.is_multiple_of(2)
            || bytes.get(identifier_start..identifier_end).is_none()
        {
            return Err(LayoutScanError::Platform(
                "layout_stream_identifier_invalid".to_string(),
            ));
        }
        if stream.attribute_type_code == NTFS_DATA_ATTRIBUTE && stream.stream_identifier_length == 0
        {
            if stream.end_of_file < 0 || stream._allocation_size < 0 {
                return Err(LayoutScanError::Platform(
                    "layout_stream_negative_size".to_string(),
                ));
            }
            return Ok(Some(FileSpaceUsage {
                logical_bytes: stream.end_of_file as u64,
                allocated_bytes: stream._allocation_size as u64,
            }));
        }
        if stream.next_stream_offset == 0 {
            return Ok(None);
        }
        let minimum_next_offset = size_of::<StreamLayoutHeader>()
            .checked_add(stream.stream_identifier_length as usize)
            .ok_or_else(|| LayoutScanError::Platform("layout_stream_overflow".to_string()))?;
        if (stream.next_stream_offset as usize) < minimum_next_offset {
            return Err(LayoutScanError::Platform(
                "layout_next_stream_offset_overlaps_entry".to_string(),
            ));
        }
        stream_offset = checked_relative_offset(
            stream_offset,
            stream.next_stream_offset as usize,
            bytes.len(),
        )
        .map_err(LayoutScanError::Platform)?;
    }
    Err(LayoutScanError::Platform(
        "layout_stream_chain_limit_exceeded".to_string(),
    ))
}

fn is_reserved_ntfs_record(file_reference_number: u64) -> bool {
    file_reference_number & FILE_REFERENCE_NUMBER_MASK < RESERVED_NTFS_RECORD_COUNT
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FileLayoutNameHeader {
    next_name_offset: u32,
    flags: u32,
    parent_file_reference_number: u64,
    file_name_length: u32,
    _reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct StreamLayoutHeader {
    version: u32,
    next_stream_offset: u32,
    _flags: u32,
    _extent_information_offset: u32,
    _allocation_size: i64,
    end_of_file: i64,
    _stream_information_offset: u32,
    attribute_type_code: u32,
    _attribute_flags: u32,
    stream_identifier_length: u32,
}

// These repr(C) kernel headers contain only integers, with no bool, reference,
// or Rust enum fields, so every bit pattern is valid. Flexible-array payloads
// are outside the structs and are read separately through checked ranges.
unsafe impl RawLayoutValue for QUERY_FILE_LAYOUT_OUTPUT {}
unsafe impl RawLayoutValue for FILE_LAYOUT_ENTRY {}
unsafe impl RawLayoutValue for FileLayoutNameHeader {}
unsafe impl RawLayoutValue for StreamLayoutHeader {}

fn checked_relative_offset(
    base: usize,
    relative: usize,
    buffer_length: usize,
) -> Result<usize, String> {
    let offset = base
        .checked_add(relative)
        .ok_or_else(|| "layout_offset_overflow".to_string())?;
    if relative == 0 || offset >= buffer_length {
        return Err("layout_offset_out_of_bounds".to_string());
    }
    Ok(offset)
}

fn read_utf16_os(bytes: &[u8], offset: usize, byte_length: usize) -> Option<OsString> {
    if !byte_length.is_multiple_of(2) {
        return None;
    }
    let end = offset.checked_add(byte_length)?;
    let raw = bytes.get(offset..end)?;
    let units = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    // NTFS name lengths exclude a terminator. A NUL inside the returned range
    // indicates corrupt length or offset data. Keeping that path would merely
    // defer the parse failure into a metadata error and hide the file.
    if units.contains(&0) {
        return None;
    }
    Some(OsString::from_wide(&units))
}

fn is_single_path_component(name: &OsString) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_ntfs_records_ignore_sequence_number() {
        assert!(is_reserved_ntfs_record((7_u64 << 48) | 5));
        assert!(!is_reserved_ntfs_record((7_u64 << 48) | 24));
    }

    #[test]
    fn primary_dos_name_is_not_discarded_as_an_alias() {
        let mut bytes = vec![0u8; 192];
        write_name_entry(
            &mut bytes,
            64,
            0,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY | FILE_LAYOUT_NAME_ENTRY_DOS,
            42,
            "temp",
        );

        let names = read_long_names(&bytes, 0, 64).expect("primary name should parse");

        assert_eq!(names.len(), 1);
        assert_eq!(names[0].parent_id, 42);
        assert_eq!(names[0].name, "temp");
    }

    #[test]
    fn layout_names_cannot_escape_their_parent() {
        let mut bytes = vec![0u8; 192];
        write_name_entry(
            &mut bytes,
            64,
            0,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            42,
            r"..\escape",
        );

        let result = read_long_names(&bytes, 0, 64);

        assert!(matches!(
            result,
            Err(LayoutScanError::Platform(error))
                if error == "layout_name_is_not_a_component"
        ));
    }

    #[test]
    fn hard_links_keep_long_names_and_drop_dos_only_aliases() {
        let mut bytes = vec![0u8; 384];
        write_name_entry(
            &mut bytes,
            64,
            64,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            42,
            "primary.bin",
        );
        write_name_entry(
            &mut bytes,
            128,
            64,
            FILE_LAYOUT_NAME_ENTRY_DOS,
            42,
            "PRIMAR~1.BIN",
        );
        write_name_entry(&mut bytes, 192, 0, 0, 84, "hardlink-Δ-🧪.bin");

        let names = read_long_names(&bytes, 0, 64).expect("hard-link name chain should parse");

        assert_eq!(names.len(), 2);
        assert_eq!(names[0].name, "primary.bin");
        assert_eq!(names[1].parent_id, 84);
        assert_eq!(names[1].name, "hardlink-Δ-🧪.bin");
    }

    #[test]
    fn full_analysis_counts_each_hard_link_and_keeps_large_candidate() {
        let mut bytes = vec![0u8; 512];
        write_copy_for_test(
            &mut bytes,
            0,
            QUERY_FILE_LAYOUT_OUTPUT {
                FileEntryCount: 1,
                FirstFileOffset: 64,
                ..Default::default()
            },
        );
        write_copy_for_test(
            &mut bytes,
            64,
            FILE_LAYOUT_ENTRY {
                Version: SUPPORTED_FILE_LAYOUT_VERSION,
                FileReferenceNumber: 100,
                FirstNameOffset: 128,
                FirstStreamOffset: 256,
                ..Default::default()
            },
        );
        write_name_entry(
            &mut bytes,
            192,
            64,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            42,
            "first.bin",
        );
        write_name_entry(
            &mut bytes,
            256,
            0,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            84,
            "second.bin",
        );
        write_stream_entry(&mut bytes, 320, 0, NTFS_DATA_ATTRIBUTE, 0, 123);
        let mut collection = LayoutCollection::default();

        parse_layout_page(
            &bytes,
            100,
            LayoutCollectionMode::FullAnalysis,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        )
        .expect("full analysis should parse hard links");

        assert_eq!(
            collection.direct_totals[&42],
            DirectoryTotals {
                logical_bytes: 123,
                allocated_bytes: 123,
                file_count: 1,
                direct_file_count: 1,
                skipped_count: 0,
            }
        );
        assert_eq!(collection.direct_totals[&84].logical_bytes, 123);
        assert_eq!(collection.direct_totals[&84].allocated_bytes, 123);
        assert_eq!(collection.candidates.len(), 1);
        assert_eq!(collection.candidate_path_count, 2);
    }

    #[test]
    fn full_analysis_skips_remote_attribute_streams_without_creating_candidates() {
        let mut bytes = vec![0u8; 384];
        write_copy_for_test(
            &mut bytes,
            0,
            QUERY_FILE_LAYOUT_OUTPUT {
                FileEntryCount: 1,
                FirstFileOffset: 64,
                ..Default::default()
            },
        );
        write_copy_for_test(
            &mut bytes,
            64,
            FILE_LAYOUT_ENTRY {
                Version: SUPPORTED_FILE_LAYOUT_VERSION,
                FileReferenceNumber: 100,
                FirstNameOffset: 128,
                FirstStreamOffset: 192,
                FileAttributes: 0x0000_1000,
                ..Default::default()
            },
        );
        write_name_entry(
            &mut bytes,
            192,
            0,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            42,
            "remote.bin",
        );
        write_stream_entry_with_allocation(&mut bytes, 256, 0, NTFS_DATA_ATTRIBUTE, 0, 0, 123);
        let mut collection = LayoutCollection::default();

        parse_layout_page(
            &bytes,
            100,
            LayoutCollectionMode::FullAnalysis,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        )
        .expect("remote-only stream should parse");

        assert_eq!(
            collection.direct_totals[&42],
            DirectoryTotals {
                logical_bytes: 0,
                allocated_bytes: 0,
                file_count: 0,
                direct_file_count: 0,
                skipped_count: 1,
            }
        );
        assert!(collection.candidates.is_empty());
        assert_eq!(collection.candidate_path_count, 0);
        assert_eq!(collection.remote_file_count, 1);
        assert_eq!(collection.remote_directory_count, 0);
    }

    #[test]
    fn zero_allocation_local_stream_uses_the_selected_candidate_size_metric() {
        let mut bytes = vec![0u8; 384];
        write_copy_for_test(
            &mut bytes,
            0,
            QUERY_FILE_LAYOUT_OUTPUT {
                FileEntryCount: 1,
                FirstFileOffset: 64,
                ..Default::default()
            },
        );
        write_copy_for_test(
            &mut bytes,
            64,
            FILE_LAYOUT_ENTRY {
                Version: SUPPORTED_FILE_LAYOUT_VERSION,
                FileReferenceNumber: 100,
                FirstNameOffset: 128,
                FirstStreamOffset: 192,
                ..Default::default()
            },
        );
        write_name_entry(
            &mut bytes,
            192,
            0,
            FILE_LAYOUT_NAME_ENTRY_PRIMARY,
            42,
            "local.bin",
        );
        write_stream_entry_with_allocation(&mut bytes, 256, 0, NTFS_DATA_ATTRIBUTE, 0, 0, 123);
        let mut collection = LayoutCollection::default();

        parse_layout_page(
            &bytes,
            100,
            LayoutCollectionMode::FullAnalysis,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        )
        .expect("zero-allocation local stream should parse");

        assert_eq!(
            collection.direct_totals[&42],
            DirectoryTotals {
                logical_bytes: 123,
                allocated_bytes: 0,
                file_count: 1,
                direct_file_count: 0,
                skipped_count: 0,
            }
        );
        assert!(collection.candidates.is_empty());
        assert_eq!(collection.candidate_path_count, 0);
        assert_eq!(collection.remote_file_count, 0);

        let mut duplicate_collection = LayoutCollection::default();
        parse_layout_page(
            &bytes,
            100,
            LayoutCollectionMode::FullAnalysis,
            CandidateSizeMetric::Logical,
            &|| false,
            &mut duplicate_collection,
        )
        .expect("the duplicate scan should use logical stream size");
        assert_eq!(duplicate_collection.candidates.len(), 1);
        assert_eq!(duplicate_collection.candidate_path_count, 1);
    }

    #[test]
    fn default_data_stream_is_found_after_named_streams() {
        let mut bytes = vec![0u8; 256];
        write_stream_entry(&mut bytes, 64, 64, NTFS_DATA_ATTRIBUTE, 8, 12);
        write_stream_entry(&mut bytes, 128, 0, NTFS_DATA_ATTRIBUTE, 0, 98_765);

        let usage =
            read_default_data_space_usage(&bytes, 0, 64).expect("default stream should parse");

        assert_eq!(
            usage,
            Some(FileSpaceUsage {
                logical_bytes: 98_765,
                allocated_bytes: 98_765,
            })
        );
    }

    #[test]
    fn unknown_stream_versions_fail_closed() {
        let mut bytes = vec![0u8; 192];
        write_stream_entry(&mut bytes, 64, 0, NTFS_DATA_ATTRIBUTE, 0, 42);
        let mut stream =
            read_copy::<StreamLayoutHeader>(&bytes, 64).expect("test stream header should fit");
        stream.version = SUPPORTED_STREAM_LAYOUT_VERSION + 1;
        write_copy_for_test(&mut bytes, 64, stream);

        let result = read_default_data_space_usage(&bytes, 0, 64);

        assert!(matches!(
            result,
            Err(LayoutScanError::Platform(error))
                if error == "unsupported_stream_layout_version:2"
        ));
    }

    #[test]
    fn utf16_names_reject_odd_lengths_and_embedded_nul() {
        let bytes = [b'a', 0, 0, 0];

        assert!(read_utf16_os(&bytes, 0, 3).is_none());
        assert!(read_utf16_os(&bytes, 0, 4).is_none());
    }

    #[test]
    fn layout_parser_rejects_unknown_versions_and_truncated_headers() {
        let mut bytes = vec![0u8; 256];
        let output = QUERY_FILE_LAYOUT_OUTPUT {
            FileEntryCount: 1,
            FirstFileOffset: 64,
            ..Default::default()
        };
        let file = FILE_LAYOUT_ENTRY {
            Version: SUPPORTED_FILE_LAYOUT_VERSION + 1,
            ..Default::default()
        };
        write_copy_for_test(&mut bytes, 0, output);
        write_copy_for_test(&mut bytes, 64, file);
        let mut collection = LayoutCollection::default();

        let unknown = parse_layout_page(
            &bytes,
            1,
            LayoutCollectionMode::CandidatesOnly,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        );
        let truncated = parse_layout_page(
            &[],
            1,
            LayoutCollectionMode::CandidatesOnly,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        );

        assert!(matches!(
            unknown,
            Err(LayoutScanError::Platform(error))
                if error == "unsupported_layout_version:2"
        ));
        assert!(matches!(
            truncated,
            Err(LayoutScanError::Platform(error)) if error == "layout_header_truncated"
        ));
    }

    #[test]
    fn layout_parser_rejects_empty_pages_and_entry_limit_overflow() {
        let mut empty_page = vec![0u8; size_of::<QUERY_FILE_LAYOUT_OUTPUT>()];
        write_copy_for_test(&mut empty_page, 0, QUERY_FILE_LAYOUT_OUTPUT::default());
        let mut collection = LayoutCollection::default();

        let empty = parse_layout_page(
            &empty_page,
            1,
            LayoutCollectionMode::CandidatesOnly,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        );

        assert!(matches!(
            empty,
            Err(LayoutScanError::Platform(error)) if error == "layout_page_has_no_entries"
        ));

        let mut populated_page = vec![0u8; 128];
        write_copy_for_test(
            &mut populated_page,
            0,
            QUERY_FILE_LAYOUT_OUTPUT {
                FileEntryCount: 1,
                FirstFileOffset: 64,
                ..Default::default()
            },
        );
        write_copy_for_test(
            &mut populated_page,
            64,
            FILE_LAYOUT_ENTRY {
                Version: SUPPORTED_FILE_LAYOUT_VERSION,
                ..Default::default()
            },
        );
        collection.entry_count = MAX_LAYOUT_ENTRIES;

        let overflow = parse_layout_page(
            &populated_page,
            1,
            LayoutCollectionMode::CandidatesOnly,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        );

        assert!(matches!(
            overflow,
            Err(LayoutScanError::Platform(error)) if error == "layout_entry_limit_exceeded"
        ));
    }

    #[test]
    fn layout_parser_rejects_overlapping_variable_sections() {
        let mut page = vec![0u8; 256];
        write_copy_for_test(
            &mut page,
            0,
            QUERY_FILE_LAYOUT_OUTPUT {
                FileEntryCount: 1,
                FirstFileOffset: 1,
                ..Default::default()
            },
        );
        let mut collection = LayoutCollection::default();

        let first_file = parse_layout_page(
            &page,
            1,
            LayoutCollectionMode::CandidatesOnly,
            CandidateSizeMetric::Allocated,
            &|| false,
            &mut collection,
        );
        let name = read_long_names(&page, 0, 1);
        let stream = read_default_data_space_usage(&page, 0, 1);

        assert!(matches!(
            first_file,
            Err(LayoutScanError::Platform(error))
                if error == "layout_first_file_offset_overlaps_header"
        ));
        assert!(matches!(
            name,
            Err(LayoutScanError::Platform(error))
                if error == "layout_name_offset_overlaps_entry"
        ));
        assert!(matches!(
            stream,
            Err(LayoutScanError::Platform(error))
                if error == "layout_stream_offset_overlaps_entry"
        ));
    }

    fn write_name_entry(
        bytes: &mut [u8],
        offset: usize,
        next_offset: u32,
        flags: u32,
        parent_id: u64,
        name: &str,
    ) {
        let units = name.encode_utf16().collect::<Vec<_>>();
        let header = FileLayoutNameHeader {
            next_name_offset: next_offset,
            flags,
            parent_file_reference_number: parent_id,
            file_name_length: u32::try_from(units.len() * 2)
                .expect("test name should fit in a u32 byte length"),
            _reserved: 0,
        };
        write_copy_for_test(bytes, offset, header);
        let name_offset = offset + size_of::<FileLayoutNameHeader>();
        for (index, unit) in units.into_iter().enumerate() {
            let start = name_offset + index * 2;
            bytes[start..start + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }

    fn write_stream_entry(
        bytes: &mut [u8],
        offset: usize,
        next_offset: u32,
        attribute_type_code: u32,
        identifier_length: u32,
        end_of_file: i64,
    ) {
        write_stream_entry_with_allocation(
            bytes,
            offset,
            next_offset,
            attribute_type_code,
            identifier_length,
            end_of_file,
            end_of_file,
        );
    }

    fn write_stream_entry_with_allocation(
        bytes: &mut [u8],
        offset: usize,
        next_offset: u32,
        attribute_type_code: u32,
        identifier_length: u32,
        allocation_size: i64,
        end_of_file: i64,
    ) {
        write_copy_for_test(
            bytes,
            offset,
            StreamLayoutHeader {
                version: SUPPORTED_STREAM_LAYOUT_VERSION,
                next_stream_offset: next_offset,
                _flags: 0,
                _extent_information_offset: 0,
                _allocation_size: allocation_size,
                end_of_file,
                _stream_information_offset: 0,
                attribute_type_code,
                _attribute_flags: 0,
                stream_identifier_length: identifier_length,
            },
        );
    }

    fn write_copy_for_test<T: Copy>(bytes: &mut [u8], offset: usize, value: T) {
        let end = offset + size_of::<T>();
        assert!(end <= bytes.len(), "test structure must fit in the buffer");
        unsafe {
            ptr::write_unaligned(bytes[offset..end].as_mut_ptr().cast::<T>(), value);
        }
    }
}
