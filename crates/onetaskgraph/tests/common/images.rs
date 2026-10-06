//! Test images, generated in the test and never committed.
//!
//! [`png`] is the one source of every PNG a journey stores or copies, and of the range a
//! representative screenshot's size falls in: real screenshots on a plan are 50 to 500 KB, so
//! the PNGs the asset journeys carry are that size too, and a copy budget measured over them
//! is measured at the weight it has to hold. Their pixels are seeded noise, which does not
//! compress, so the size asked for is the size written. [`jpeg`], [`gif`] and [`webp`] are
//! small valid files of their format, for the journeys that prove an extension and its
//! content type rather than a copy's weight.

use std::ops::RangeInclusive;

/// The sizes, in bytes, [`png`] makes a PNG of: 50 KB to 500 KB.
pub const PNG_SIZE_RANGE: RangeInclusive<usize> = 50_000..=500_000;

/// How far from the size asked for a PNG [`png`] makes may be: 5 percent either way.
pub const PNG_SIZE_TOLERANCE_PERCENT: usize = 5;

/// The width of every PNG [`png`] makes, in pixels; the height is what the size needs.
const PNG_WIDTH: usize = 256;

/// `png(seed: u64, size: usize) -> Vec<u8>`: a valid 8-bit RGB PNG of `size` bytes, give or
/// take [`PNG_SIZE_TOLERANCE_PERCENT`] percent, whose pixels are distinct pseudo-random noise
/// drawn from `seed`.
///
/// Two seeds give two different images and one seed gives the same image every time. The
/// image data is stored uncompressed — noise does not compress, and stored blocks make the
/// file's size exactly computable — so the file is within one row of `size`.
///
/// # Panics
///
/// Fails the test when `size` is outside [`PNG_SIZE_RANGE`], naming the size and the range:
/// a journey asking for an image no plan would carry is measuring the wrong thing.
pub fn png(seed: u64, size: usize) -> Vec<u8> {
    assert!(
        PNG_SIZE_RANGE.contains(&size),
        "a test PNG of {size} bytes was asked for, outside {}..={} bytes — the range a real \
         screenshot on a plan falls in; ask for a size inside it",
        PNG_SIZE_RANGE.start(),
        PNG_SIZE_RANGE.end()
    );
    let row = 1 + PNG_WIDTH * 3;
    // Signature, IHDR, the IDAT chunk's own framing and the zlib header and trailer, IEND.
    let fixed = 8 + 25 + 12 + 2 + 4 + 12;
    let mut height = size.saturating_sub(fixed) / row;
    while fixed + height * row + 5 * stored_blocks(height * row) > size + row / 2 {
        height -= 1;
    }
    let mut noise = Noise::new(seed);
    let mut raw = Vec::with_capacity(height * row);
    for _ in 0..height {
        // Filter type 0: each row is its bytes as they are.
        raw.push(0);
        raw.extend((0..PNG_WIDTH * 3).map(|_| noise.byte()));
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&u32::try_from(PNG_WIDTH).expect("fits").to_be_bytes());
    header.extend_from_slice(&u32::try_from(height).expect("fits").to_be_bytes());
    // 8 bits per sample, colour type 2 (RGB), deflate, adaptive filtering, no interlace.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &stored_zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    let within = size * PNG_SIZE_TOLERANCE_PERCENT / 100;
    assert!(
        out.len().abs_diff(size) <= within,
        "the generator made {} bytes for {size} asked",
        out.len()
    );
    out
}

/// A valid 8×8 greyscale baseline JPEG whose comment segment carries `seed`, so two seeds
/// give two files.
pub fn jpeg(seed: u64) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    // A comment naming the seed.
    let comment = format!("onetaskgraph test image {seed}");
    segment(&mut out, 0xFE, comment.as_bytes());
    // One quantisation table, every entry 1.
    let mut table = vec![0x00];
    table.extend_from_slice(&[1; 64]);
    segment(&mut out, 0xDB, &table);
    // Baseline frame: 8-bit precision, 8×8, one component sampled 1×1 on table 0.
    segment(&mut out, 0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    // A DC table and an AC table with one one-bit code each: DC category 0, and end of block.
    let mut huffman = vec![0x00, 1];
    huffman.extend_from_slice(&[0; 15]);
    huffman.push(0);
    huffman.push(0x10);
    huffman.push(1);
    huffman.extend_from_slice(&[0; 15]);
    huffman.push(0);
    segment(&mut out, 0xC4, &huffman);
    // One scan of the one component, full spectral range.
    segment(&mut out, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
    // The one block: DC difference 0, then end of block, padded with ones.
    out.push(0b0011_1111);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A valid 1×1 GIF whose one colour is drawn from `seed`.
pub fn gif(seed: u64) -> Vec<u8> {
    let mut noise = Noise::new(seed);
    let mut out = b"GIF89a".to_vec();
    // 1×1, a global colour table of two entries, background 0, no aspect ratio.
    out.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
    out.extend_from_slice(&[noise.byte(), noise.byte(), noise.byte(), 0, 0, 0]);
    // One image descriptor covering the canvas, then its LZW data: minimum code size 2, one
    // two-byte sub-block encoding clear, index 0, end.
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
    out.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
    out.push(0x3B);
    out
}

/// A valid 1×1 lossless WebP whose one pixel's colour is drawn from `seed`.
pub fn webp(seed: u64) -> Vec<u8> {
    let mut noise = Noise::new(seed);
    let mut bits = Bits::default();
    bits.push(0x2F, 8);
    // Width and height less one, no alpha, version 0.
    bits.push(0, 14);
    bits.push(0, 14);
    bits.push(0, 1);
    bits.push(0, 3);
    // No transform, no colour cache, no meta prefix codes.
    bits.push(0, 1);
    bits.push(0, 1);
    bits.push(0, 1);
    // Green, red, blue and alpha: each a simple prefix code of one eight-bit symbol, so the
    // one pixel costs no bits at all.
    for value in [noise.byte(), noise.byte(), noise.byte(), 0xFF] {
        bits.push(1, 1);
        bits.push(0, 1);
        bits.push(1, 1);
        bits.push(u32::from(value), 8);
    }
    // Distance: a simple code of the one one-bit symbol 0.
    bits.push(1, 1);
    bits.push(0, 1);
    bits.push(0, 1);
    bits.push(0, 1);
    let data = bits.finish();
    let mut out = b"RIFF".to_vec();
    let riff = 4 + 8 + data.len() + data.len() % 2;
    out.extend_from_slice(&u32::try_from(riff).expect("fits").to_le_bytes());
    out.extend_from_slice(b"WEBPVP8L");
    out.extend_from_slice(&u32::try_from(data.len()).expect("fits").to_le_bytes());
    out.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A seeded xorshift stream: the same bytes for one seed, different bytes for another.
struct Noise(u64);

impl Noise {
    fn new(seed: u64) -> Self {
        // Never zero, which xorshift cannot leave.
        Self(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    fn byte(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.to_le_bytes()[3]
    }
}

/// How many stored deflate blocks `length` bytes take.
fn stored_blocks(length: usize) -> usize {
    length.div_ceil(65_535).max(1)
}

/// `data` as a zlib stream of stored (uncompressed) deflate blocks.
fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks = data.chunks(65_535).collect::<Vec<_>>();
    let blocks = if blocks.is_empty() {
        vec![&data[..0]]
    } else {
        blocks
    };
    let last = blocks.len() - 1;
    for (index, block) in blocks.into_iter().enumerate() {
        out.push(u8::from(index == last));
        let length = u16::try_from(block.len()).expect("a stored block fits");
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// One PNG chunk: its length, type, data and CRC.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).expect("fits").to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// One JPEG marker segment.
fn segment(out: &mut Vec<u8>, marker: u8, data: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&u16::try_from(data.len() + 2).expect("fits").to_be_bytes());
    out.extend_from_slice(data);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Bits written least significant first, as a VP8L stream reads them.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: usize,
}

impl Bits {
    fn push(&mut self, value: u32, count: usize) {
        for bit in 0..count {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if value >> bit & 1 == 1 {
                *self.bytes.last_mut().expect("a byte was pushed") |= 1 << (self.used % 8);
            }
            self.used += 1;
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
