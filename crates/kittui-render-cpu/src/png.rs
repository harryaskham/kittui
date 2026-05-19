//! Minimal PNG and APNG encoders. Sized for kittui's small rasters: a single
//! IDAT chunk per frame and no filtering optimisation. The output is a
//! standards-compliant PNG that every kitty-protocol-capable terminal accepts.

use std::io::Write;

use crc32fast::Hasher;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::pixmap::Pixmap;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

fn write_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut h = Hasher::new();
    h.update(ty);
    h.update(data);
    out.extend_from_slice(&h.finalize().to_be_bytes());
}

fn ihdr(width: u32, height: u32) -> [u8; 13] {
    let mut buf = [0u8; 13];
    buf[..4].copy_from_slice(&width.to_be_bytes());
    buf[4..8].copy_from_slice(&height.to_be_bytes());
    buf[8] = 8; // bit depth
    buf[9] = 6; // color type RGBA
    buf[10] = 0;
    buf[11] = 0;
    buf[12] = 0;
    buf
}

fn filtered_rows(pixmap: &Pixmap) -> Vec<u8> {
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let stride = w * 4;
    let mut out = Vec::with_capacity(h * (stride + 1));
    let data = pixmap.data();
    for row in 0..h {
        out.push(0); // filter type: none
        out.extend_from_slice(&data[row * stride..(row + 1) * stride]);
    }
    out
}

fn zlib(payload: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(payload).expect("zlib write");
    z.finish().expect("zlib finish")
}

/// Encode a [`Pixmap`] as a standard 8-bit RGBA PNG.
pub fn encode_png(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixmap.data().len() / 2 + 256);
    out.extend_from_slice(&SIGNATURE);
    write_chunk(&mut out, b"IHDR", &ihdr(pixmap.width(), pixmap.height()));
    let data = zlib(&filtered_rows(pixmap));
    write_chunk(&mut out, b"IDAT", &data);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode a series of pixmaps as an APNG. `frame_delays_ms` must have the
/// same length as `frames`. `loops == 0` means loop forever.
///
/// Note: kittui's animated path normally uploads each frame as a separate
/// kitty image and uses the protocol's native animation chain, so this
/// encoder exists primarily for offline export and golden snapshots.
pub fn encode_apng(frames: &[Pixmap], frame_delays_ms: &[u32], loops: u32) -> Vec<u8> {
    assert!(!frames.is_empty(), "apng requires at least one frame");
    assert_eq!(
        frames.len(),
        frame_delays_ms.len(),
        "apng frame count must equal delay count"
    );
    let width = frames[0].width();
    let height = frames[0].height();
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE);
    write_chunk(&mut out, b"IHDR", &ihdr(width, height));

    let mut actl = [0u8; 8];
    actl[..4].copy_from_slice(&(frames.len() as u32).to_be_bytes());
    actl[4..8].copy_from_slice(&loops.to_be_bytes());
    write_chunk(&mut out, b"acTL", &actl);

    let mut sequence: u32 = 0;
    for (i, frame) in frames.iter().enumerate() {
        let mut fctl = [0u8; 26];
        fctl[..4].copy_from_slice(&sequence.to_be_bytes());
        fctl[4..8].copy_from_slice(&width.to_be_bytes());
        fctl[8..12].copy_from_slice(&height.to_be_bytes());
        fctl[12..16].copy_from_slice(&0u32.to_be_bytes()); // x offset
        fctl[16..20].copy_from_slice(&0u32.to_be_bytes()); // y offset
        let delay = frame_delays_ms[i].min(u16::MAX as u32) as u16;
        fctl[20..22].copy_from_slice(&delay.to_be_bytes());
        fctl[22..24].copy_from_slice(&1000u16.to_be_bytes()); // numerator/denom = ms
        fctl[24] = 1; // dispose: background
        fctl[25] = 0; // blend: source
        write_chunk(&mut out, b"fcTL", &fctl);
        sequence = sequence.wrapping_add(1);
        let payload = zlib(&filtered_rows(frame));
        if i == 0 {
            write_chunk(&mut out, b"IDAT", &payload);
        } else {
            let mut fdat = Vec::with_capacity(payload.len() + 4);
            fdat.extend_from_slice(&sequence.to_be_bytes());
            fdat.extend_from_slice(&payload);
            sequence = sequence.wrapping_add(1);
            write_chunk(&mut out, b"fdAT", &fdat);
        }
    }
    write_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_png_has_signature_and_iend() {
        let pixmap = Pixmap::new(2, 2);
        let png = encode_png(&pixmap);
        assert_eq!(&png[..8], &SIGNATURE);
        assert!(png.ends_with(b"IEND\xae\x42\x60\x82"));
    }
}
