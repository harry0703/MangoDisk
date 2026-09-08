/// A 32-bit 32x32 ICO with a transparent alpha channel and a fully transparent AND mask.
/// Both representations are supplied because Windows Shell may use either icon rendering path.
pub(super) const TRANSPARENT_ICON: [u8; 4_286] = transparent_icon();

const fn transparent_icon() -> [u8; 4_286] {
    let mut bytes = [0_u8; 4_286];
    bytes[2] = 1; // ICO, not a cursor.
    bytes[4] = 1; // One image.
    bytes[6] = 32;
    bytes[7] = 32;
    bytes[10] = 1;
    bytes[12] = 32;
    bytes[14] = 0xa8; // DIB header + BGRA pixels + row-aligned AND mask: 4264 bytes.
    bytes[15] = 0x10;
    bytes[18] = 22; // Image offset after ICONDIR and ICONDIRENTRY.
    bytes[22] = 40; // BITMAPINFOHEADER length.
    bytes[26] = 32;
    bytes[30] = 64; // Combined XOR and AND bitmap height.
    bytes[34] = 1;
    bytes[36] = 32;
    let mut index = 22 + 40 + 32 * 32 * 4;
    while index < bytes.len() {
        bytes[index] = 0xff;
        index += 1;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_icon_has_consistent_offsets_and_both_transparency_masks() {
        let data = TRANSPARENT_ICON;
        let u32_at = |index| u32::from_le_bytes(data[index..index + 4].try_into().unwrap());
        assert_eq!(u32_at(18) + u32_at(14), data.len() as u32);
        assert_eq!(u32_at(26), 32);
        assert_eq!(u32_at(30), 64);
        assert!(data[62..4158].iter().all(|byte| *byte == 0));
        assert!(data[4158..].iter().all(|byte| *byte == 0xff));
    }
}
