//! Small checked little-endian slice operations shared by byte-only parsers.

pub(crate) type Result<T> = std::result::Result<T, &'static str>;
pub(crate) fn slice(bytes: &[u8], start: usize, length: usize) -> Result<&[u8]> {
    let end = start.checked_add(length).ok_or("range overflow")?;
    bytes.get(start..end).ok_or("range exceeds input")
}
pub(crate) fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let b = slice(bytes, offset, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
pub(crate) fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let b = slice(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
