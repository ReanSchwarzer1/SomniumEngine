//! The engine mark packed as a Windows `.ico`.
//!
//! MORROWIND-J step 2. [`crate::brand`] gives a *running* window its icon, which
//! covers the taskbar button, alt-tab and the window list. It does not cover the
//! executable sitting in a folder, or a shortcut pinned to the taskbar before
//! the editor is started: those read an icon resource linked into the binary,
//! and a resource is a file.
//!
//! The container is deliberately old-fashioned. An `.ico` entry may hold a PNG,
//! which would be smaller, but nothing in this crate's dependencies encodes one
//! and pulling in an encoder to write a file that changes about once a year is a
//! poor trade. The BMP payload is a header, the pixels bottom-up, and a mask
//! that 32-bit entries do not use.

/// One image inside the file.
struct Entry {
    size: u32,
    /// BGRA, top-down, `size * size * 4` bytes.
    bgra: Vec<u8>,
}

/// Sizes Windows asks for, from a list-view row to the extra-large grid.
///
/// 256 is left out on purpose: its uncompressed payload alone is a quarter of a
/// megabyte, and Windows scales 128 up for the one view that wants it.
pub const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 128];

/// Build an `.ico` holding the engine mark at every size in [`SIZES`].
///
/// `None` if the vendored drawing fails to rasterise, which is a broken build
/// rather than a runtime condition.
#[must_use]
pub fn engine_mark(tint: [u8; 4]) -> Option<Vec<u8>> {
    let entries: Option<Vec<Entry>> = SIZES
        .iter()
        .map(|&size| {
            let image = crate::brand::mark(size, tint)?;
            let bgra = image
                .rgba
                .chunks_exact(4)
                .flat_map(|px| [px[2], px[1], px[0], px[3]])
                .collect();
            Some(Entry { size, bgra })
        })
        .collect();
    Some(pack(&entries?))
}

/// ICONDIR, then one ICONDIRENTRY each, then the payloads.
fn pack(entries: &[Entry]) -> Vec<u8> {
    const DIR: usize = 6;
    const ENTRY: usize = 16;

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon, 2 = cursor
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut offset = (DIR + ENTRY * entries.len()) as u32;
    let payloads: Vec<Vec<u8>> = entries.iter().map(dib).collect();
    for (entry, payload) in entries.iter().zip(&payloads) {
        // 0 means 256 in this field, which is why 256 would still fit in a byte
        // if it were ever added back.
        out.push(u8::try_from(entry.size).unwrap_or(0));
        out.push(u8::try_from(entry.size).unwrap_or(0));
        out.push(0); // palette entries
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += payload.len() as u32;
    }
    for payload in payloads {
        out.extend_from_slice(&payload);
    }
    out
}

/// One entry's BITMAPINFOHEADER, its pixels, and its AND mask.
fn dib(entry: &Entry) -> Vec<u8> {
    let size = entry.size;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(size as i32).to_le_bytes());
    // Twice the height: the format expects the colour image and the mask
    // stacked, and the field describes both even when the mask is unused.
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&0u32.to_le_bytes()); // image size, may be 0
    out.extend_from_slice(&0i32.to_le_bytes()); // x pixels per metre
    out.extend_from_slice(&0i32.to_le_bytes()); // y
    out.extend_from_slice(&0u32.to_le_bytes()); // palette used
    out.extend_from_slice(&0u32.to_le_bytes()); // palette important

    // Bottom-up, which is the one thing about this format that catches people.
    let stride = (size * 4) as usize;
    for row in (0..size as usize).rev() {
        out.extend_from_slice(&entry.bgra[row * stride..(row + 1) * stride]);
    }

    // The AND mask, all zeros: every pixel is drawn, and the alpha channel
    // above decides what shows. Rows are padded to four bytes.
    let mask_stride = (size as usize).div_ceil(32) * 4;
    out.extend(std::iter::repeat_n(0u8, mask_stride * size as usize));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u16(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }
    fn read_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn the_directory_describes_the_file_that_follows_it() {
        // Every field here is one Windows reads to find the images. A wrong
        // offset does not fail loudly: the icon simply does not appear, in a
        // place nothing in this repository can look at.
        let ico = engine_mark([0x4C, 0x8D, 0xFF, 0xFF]).expect("vendored source parses");
        assert_eq!(read_u16(&ico, 0), 0, "reserved");
        assert_eq!(read_u16(&ico, 2), 1, "type: icon");
        assert_eq!(read_u16(&ico, 4) as usize, SIZES.len());

        for (index, &size) in SIZES.iter().enumerate() {
            let entry = 6 + index * 16;
            assert_eq!(u32::from(ico[entry]), size, "width of entry {index}");
            assert_eq!(u32::from(ico[entry + 1]), size, "height of entry {index}");
            assert_eq!(read_u16(&ico, entry + 6), 32, "bits per pixel");

            let bytes = read_u32(&ico, entry + 8) as usize;
            let offset = read_u32(&ico, entry + 12) as usize;
            assert!(
                offset + bytes <= ico.len(),
                "entry {index} points past the end of the file"
            );
            // Header, the pixels, and a mask padded to four bytes a row.
            let mask = (size as usize).div_ceil(32) * 4 * size as usize;
            assert_eq!(bytes, 40 + (size * size * 4) as usize + mask);
            assert_eq!(read_u32(&ico, offset), 40, "BITMAPINFOHEADER size");
            assert_eq!(
                read_u32(&ico, offset + 8),
                size * 2,
                "the stacked height the format expects"
            );
        }
    }

    #[test]
    fn the_entries_are_laid_end_to_end_with_no_gaps() {
        let ico = engine_mark([0xFF, 0xFF, 0xFF, 0xFF]).expect("vendored source parses");
        let mut expected = 6 + 16 * SIZES.len();
        for index in 0..SIZES.len() {
            let entry = 6 + index * 16;
            assert_eq!(read_u32(&ico, entry + 12) as usize, expected);
            expected += read_u32(&ico, entry + 8) as usize;
        }
        assert_eq!(expected, ico.len(), "trailing bytes nothing points at");
    }

    #[test]
    fn the_pixels_are_bottom_up_and_blue_first() {
        // Two conventions that are easy to get backwards and that produce an
        // upside-down icon in the wrong colour rather than an error.
        let tint = [0x10, 0x20, 0x30, 0xFF];
        let ico = engine_mark(tint).expect("vendored source parses");
        let entry = 6; // the 16 px image
        let offset = read_u32(&ico, entry + 12) as usize + 40;
        let opaque = ico[offset..]
            .chunks_exact(4)
            .take(16 * 16)
            .find(|px| px[3] > 0)
            .expect("the mark covers some of the smallest cell");
        assert_eq!(
            [opaque[0], opaque[1], opaque[2]],
            [tint[2], tint[1], tint[0]]
        );

        // The mark is drawn in the middle of the cell, so the first row read
        // bottom-up and the first row read top-down are both empty; what
        // distinguishes them is that a flipped image is a different sequence.
        let mark = crate::brand::mark(16, tint).expect("vendored source parses");
        let stride = 16 * 4;
        let last_row: Vec<u8> = mark.rgba[stride * 15..stride * 16]
            .chunks_exact(4)
            .flat_map(|px| [px[2], px[1], px[0], px[3]])
            .collect();
        assert_eq!(&ico[offset..offset + stride], &last_row[..]);
    }
}
