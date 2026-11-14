//! Narrow RFB 3.8 client for the private SSH channel.
use image::ImageDecoder;
use std::io::{self, Read, Write};

const MAX_DIMENSION: u16 = 4096;
const MAX_PIXELS: usize = 4096 * 2160;
const CURSOR: i32 = -239;
const TIGHT: i32 = 7;
const LAST_RECT: i32 = -224;
const MAX_RECTS_PER_UPDATE: u16 = 4096;
const MAX_CURSOR_SIDE: usize = 512;
const MAX_IGNORED_CUT_TEXT: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution(pub u16, pub u16);

impl Resolution {
    pub const FALLBACK: Self = Self(1024, 640);

    pub fn video_size(self) -> Self {
        let (max_width, max_height) = if self.0 >= self.1 {
            (1920.0, 1080.0)
        } else {
            (1080.0, 1920.0)
        };
        let scale = (max_width / f32::from(self.0))
            .min(max_height / f32::from(self.1))
            .min(1.0);
        Self::fit(
            (f32::from(self.0) * scale).round().max(320.0),
            (f32::from(self.1) * scale).round().max(200.0),
        )
    }

    fn valid(self) -> bool {
        self.0 > 0
            && self.1 > 0
            && self.0 <= MAX_DIMENSION
            && self.1 <= MAX_DIMENSION
            && usize::from(self.0) * usize::from(self.1) <= MAX_PIXELS
    }

    pub fn fit(width: f32, height: f32) -> Self {
        if !width.is_finite() || !height.is_finite() || width < 320.0 || height < 200.0 {
            return Self::FALLBACK;
        }
        let (width, height) = (width.round(), height.round());
        let exact = Self(width as u16, height as u16);
        if exact.valid() {
            return exact;
        }
        let scale = (f32::from(MAX_DIMENSION) / width)
            .min(f32::from(MAX_DIMENSION) / height)
            .min((MAX_PIXELS as f32 / (width * height)).sqrt());
        let fitted = Self(
            (width * scale).floor() as u16,
            (height * scale).floor() as u16,
        );
        if fitted.valid() && fitted.0 >= 320 && fitted.1 >= 200 {
            fitted
        } else {
            Self::FALLBACK
        }
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    pub layout: Option<(u32, u32)>,
    pub dirty: Option<Damage>,
}

#[derive(Default)]
pub struct Decoder {
    streams: [Option<flate2::Decompress>; 4],
    payload: Vec<u8>,
    pixels: Vec<u8>,
}

pub fn encodings(video: bool) -> Vec<u8> {
    let ids: &[i32] = if video {
        &[TIGHT, 0, -308, CURSOR, LAST_RECT, -255, -25]
    } else {
        &[0, -308, CURSOR]
    };
    let mut packet = vec![2, 0, 0, ids.len() as u8];
    for id in ids {
        packet.extend(id.to_be_bytes());
    }
    packet
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    pub x: usize,
    pub y: usize,
    pub right: usize,
    pub bottom: usize,
}

impl Damage {
    pub fn union(self, other: Self) -> Self {
        Self {
            x: self.x.min(other.x),
            y: self.y.min(other.y),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

impl Frame {
    pub fn resize_request(size: Resolution, (id, flags): (u32, u32)) -> Option<Vec<u8>> {
        if !size.valid() {
            return None;
        }
        let mut packet = vec![251, 0];
        for value in [size.0, size.1] {
            packet.extend(value.to_be_bytes());
        }
        packet.extend([1, 0]);
        packet.extend(id.to_be_bytes());
        packet.extend([0; 4]);
        for value in [size.0, size.1] {
            packet.extend(value.to_be_bytes());
        }
        packet.extend(flags.to_be_bytes());
        Some(packet)
    }
}
fn read<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn connect(input: &mut impl Read, output: &mut impl Write) -> io::Result<Frame> {
    let version = b"RFB 003.008\n";
    if read::<12>(input)? != *version {
        return Err(invalid("The server does not support RFB 3.8"));
    }
    output.write_all(version)?;
    output.flush()?;
    let [count] = read(input)?;
    let mut types = [0; 255];
    input.read_exact(&mut types[..usize::from(count)])?;
    if !types[..usize::from(count)].contains(&1) {
        return Err(invalid("Unexpected display authentication method"));
    }
    output.write_all(&[1])?;
    output.flush()?;
    if u32::from_be_bytes(read(input)?) != 0 {
        return Err(invalid("Display authentication failed"));
    }
    output.write_all(&[0])?;
    output.flush()?;
    let width = u16::from_be_bytes(read(input)?);
    let height = u16::from_be_bytes(read(input)?);
    if !Resolution(width, height).valid() {
        return Err(invalid("Display dimensions exceed the demo limits"));
    }
    let _format = read::<16>(input)?;
    let name_len = u32::from_be_bytes(read(input)?) as usize;
    if name_len > 4096 {
        return Err(invalid("Display name exceeds the demo limits"));
    }
    input.read_exact(&mut [0; 4096][..name_len])?;
    // 32-bit little-endian true color, shifts R16 G8 B0. Encoding 0 is Raw.
    output.write_all(&[
        0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
    ])?;
    // Ask the server to send its cursor separately from the framebuffer.
    // Rectangle draws its own pointer, so we consume the remote shape without painting it.
    output.write_all(&encodings(false))?;
    output.flush()?;
    Ok(Frame {
        width,
        height,
        rgba: vec![0; usize::from(width) * usize::from(height) * 4],
        layout: None,
        dirty: None,
    })
}

pub fn request(frame: &Frame, incremental: bool) -> [u8; 10] {
    let [w0, w1] = frame.width.to_be_bytes();
    let [h0, h1] = frame.height.to_be_bytes();
    [3, incremental as u8, 0, 0, 0, 0, w0, w1, h0, h1]
}

pub fn update(input: &mut impl Read, frame: &mut Frame, decoder: &mut Decoder) -> io::Result<()> {
    frame.dirty = None;
    let mut ignored = 0usize;
    loop {
        match read::<1>(input)?[0] {
            0 => break,
            2 => ignored += 1, // Bell.
            3 => {
                // Copy/paste uses a separate channel. Consume optional RFB clipboard
                // messages to stay in sync, but never decompress or apply them.
                let _padding = read::<3>(input)?;
                let length = i32::from_be_bytes(read(input)?).unsigned_abs() as usize;
                if length > MAX_IGNORED_CUT_TEXT {
                    return Err(invalid("Display clipboard message is too large"));
                }
                ignored += length + 8;
                let mut remaining = length;
                let mut scratch = [0u8; 4096];
                while remaining != 0 {
                    let n = remaining.min(scratch.len());
                    input.read_exact(&mut scratch[..n])?;
                    remaining -= n;
                }
            }
            kind => {
                return Err(invalid(&format!(
                    "Unexpected remote display message (RFB type {kind})"
                )));
            }
        }
        if ignored > MAX_IGNORED_CUT_TEXT + 8 {
            return Err(invalid("Too many messages before the next display update"));
        }
    }
    let _padding = read::<1>(input)?;
    let rectangles = u16::from_be_bytes(read(input)?);
    let open_ended = rectangles == u16::MAX;
    if !open_ended && rectangles > MAX_RECTS_PER_UPDATE {
        return Err(invalid("Too many rectangles in one update"));
    }
    // Allow a full frame and bounded overlap, even when the desktop is 4K.
    let mut budget = frame.rgba.len() * 2;
    let mut row = [0; MAX_DIMENSION as usize * 4];
    let limit = if open_ended {
        MAX_RECTS_PER_UPDATE + 1
    } else {
        rectangles
    };
    for index in 0..limit {
        let x = usize::from(u16::from_be_bytes(read(input)?));
        let y = usize::from(u16::from_be_bytes(read(input)?));
        let w = usize::from(u16::from_be_bytes(read(input)?));
        let h = usize::from(u16::from_be_bytes(read(input)?));
        let encoding = i32::from_be_bytes(read(input)?);
        if encoding == LAST_RECT {
            if x != 0 || y != 0 || w != 0 || h != 0 {
                return Err(invalid("Invalid end marker in display update"));
            }
            return Ok(());
        }
        if index == MAX_RECTS_PER_UPDATE {
            return Err(invalid(
                "Too many rectangles without a display update end marker",
            ));
        }
        if encoding == CURSOR {
            if w > MAX_CURSOR_SIDE || h > MAX_CURSOR_SIDE {
                return Err(invalid("Display cursor is too large"));
            }
            let mut remaining = (w * 4 + w.div_ceil(8)) * h;
            while remaining > 0 {
                let chunk = remaining.min(row.len());
                input.read_exact(&mut row[..chunk])?;
                remaining -= chunk;
            }
            continue;
        }
        if encoding == -308 {
            let count = usize::from(read::<4>(input)?[0]);
            if count > 16 {
                return Err(invalid("Too many virtual screens"));
            }
            let mut layout = None;
            for _ in 0..count {
                let data = read::<16>(input)?;
                layout = Some((
                    u32::from_be_bytes(data[..4].try_into().unwrap()),
                    u32::from_be_bytes(data[12..].try_into().unwrap()),
                ));
            }
            if x == 1 && y != 0 {
                continue;
            }
            if !Resolution(w as u16, h as u16).valid() {
                return Err(invalid("Resized display exceeds Rectangle's limits"));
            }
            frame.layout = layout.filter(|_| count == 1);
            if (w, h) != (usize::from(frame.width), usize::from(frame.height)) {
                frame.width = w as u16;
                frame.height = h as u16;
                frame.rgba = vec![0; w * h * 4];
                frame.dirty = Some(Damage {
                    x: 0,
                    y: 0,
                    right: w,
                    bottom: h,
                });
                budget = frame.rgba.len() * 2;
            }
            continue;
        }
        let stride = usize::from(frame.width);
        if !matches!(encoding, 0 | TIGHT)
            || w == 0
            || h == 0
            || x + w > stride
            || y + h > usize::from(frame.height)
        {
            return Err(invalid("Invalid rectangle from the display server"));
        }
        budget = budget
            .checked_sub(w * h * 4)
            .ok_or_else(|| invalid("Display update too large"))?;
        let damage = Damage {
            x,
            y,
            right: x + w,
            bottom: y + h,
        };
        frame.dirty = Some(frame.dirty.map_or(damage, |dirty| dirty.union(damage)));
        if encoding == TIGHT {
            decoder.tight(input, frame, damage)?;
            continue;
        }
        for scanline in y..y + h {
            input.read_exact(&mut row[..w * 4])?;
            let offset = (scanline * stride + x) * 4;
            for (dst, src) in frame.rgba[offset..offset + w * 4]
                .chunks_exact_mut(4)
                .zip(row.chunks_exact(4))
            {
                dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
            }
        }
    }
    Ok(())
}

fn compact_len(input: &mut impl Read) -> io::Result<usize> {
    let a = read::<1>(input)?[0];
    let mut length = usize::from(a & 127);
    if a & 128 != 0 {
        let b = read::<1>(input)?[0];
        length |= usize::from(b & 127) << 7;
        if b & 128 != 0 {
            length |= usize::from(read::<1>(input)?[0]) << 14;
        }
    }
    Ok(length)
}

impl Decoder {
    fn tight(&mut self, input: &mut impl Read, frame: &mut Frame, rect: Damage) -> io::Result<()> {
        let w = rect.right - rect.x;
        let h = rect.bottom - rect.y;
        let stride = usize::from(frame.width) * 4;
        let control = read::<1>(input)?[0];
        for (i, stream) in self.streams.iter_mut().enumerate() {
            if control & (1 << i) != 0 {
                *stream = None;
            }
        }
        let mode = control >> 4;
        let mut palette = [[0u8; 3]; 256];
        let mut colors = 0;
        let mut row_bytes = w * 3;
        match mode {
            8 => {
                let [r, g, b] = read(input)?;
                for y in rect.y..rect.bottom {
                    let offset = y * stride + rect.x * 4;
                    for px in frame.rgba[offset..offset + w * 4].chunks_exact_mut(4) {
                        px.copy_from_slice(&[r, g, b, 255]);
                    }
                }
                return Ok(());
            }
            9 => {
                self.payload.resize(compact_len(input)?, 0);
                input.read_exact(&mut self.payload)?;
                let jpeg =
                    image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&self.payload))
                        .map_err(io::Error::other)?;
                if jpeg.dimensions() != (w as u32, h as u32)
                    || jpeg.color_type() != image::ColorType::Rgb8
                {
                    return Err(invalid("Invalid JPEG display rectangle"));
                }
                self.pixels.resize(w * h * 3, 0);
                jpeg.read_image(&mut self.pixels)
                    .map_err(io::Error::other)?;
            }
            0..=7 => {
                let filter = if mode & 4 == 0 {
                    0
                } else {
                    read::<1>(input)?[0]
                };
                match filter {
                    0 | 2 => (),
                    1 => {
                        colors = usize::from(read::<1>(input)?[0]) + 1;
                        for color in &mut palette[..colors] {
                            *color = read(input)?;
                        }
                        // TigerVNC packs both one- and two-color palettes into
                        // one bit per pixel, with each row padded to a byte.
                        row_bytes = if colors <= 2 { w.div_ceil(8) } else { w };
                    }
                    _ => return Err(invalid("Unknown Tight display filter")),
                }
                let len = row_bytes * h;
                self.pixels.resize(len, 0);
                if len < 12 {
                    input.read_exact(&mut self.pixels)?;
                } else {
                    self.payload.resize(compact_len(input)?, 0);
                    input.read_exact(&mut self.payload)?;
                    let stream = self.streams[usize::from(mode & 3)]
                        .get_or_insert_with(|| flate2::Decompress::new(true));
                    let (before_in, before_out) = (stream.total_in(), stream.total_out());
                    stream
                        .decompress(
                            &self.payload,
                            &mut self.pixels,
                            flate2::FlushDecompress::Sync,
                        )
                        .map_err(io::Error::other)?;
                    if stream.total_in() - before_in != self.payload.len() as u64
                        || stream.total_out() - before_out != len as u64
                    {
                        return Err(invalid("Invalid compressed display size"));
                    }
                }
                if filter == 2 {
                    for y in 0..h {
                        for x in 0..w * 3 {
                            let i = y * row_bytes + x;
                            let left = if x >= 3 { self.pixels[i - 3] } else { 0 };
                            let above = if y > 0 { self.pixels[i - row_bytes] } else { 0 };
                            let corner = if x >= 3 && y > 0 {
                                self.pixels[i - row_bytes - 3]
                            } else {
                                0
                            };
                            let predict = (i16::from(left) + i16::from(above) - i16::from(corner))
                                .clamp(0, 255) as u8;
                            self.pixels[i] = predict.wrapping_add(self.pixels[i]);
                        }
                    }
                }
            }
            _ => return Err(invalid("Unknown Tight display mode")),
        }
        for y in 0..h {
            let offset = (rect.y + y) * stride + rect.x * 4;
            let row = &self.pixels[y * row_bytes..(y + 1) * row_bytes];
            for (x, dst) in frame.rgba[offset..offset + w * 4]
                .chunks_exact_mut(4)
                .enumerate()
            {
                let rgb = if colors == 0 {
                    &row[x * 3..x * 3 + 3]
                } else {
                    let index = if colors <= 2 {
                        (row[x / 8] >> (7 - x % 8)) & 1
                    } else {
                        row[x]
                    };
                    palette[..colors]
                        .get(usize::from(index))
                        .ok_or_else(|| invalid("Invalid display palette index"))?
                };
                dst.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        Ok(())
    }
}

pub fn key(symbol: u32, down: bool) -> [u8; 8] {
    let [a, b, c, d] = symbol.to_be_bytes();
    [4, down as u8, 0, 0, a, b, c, d]
}

pub fn pointer(mask: u8, x: u16, y: u16) -> [u8; 6] {
    let [a, b] = x.to_be_bytes();
    let [c, d] = y.to_be_bytes();
    [5, mask, a, b, c, d]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_and_applies_partial_frame() {
        let mut server = b"RFB 003.008\n".to_vec();
        server.extend([1, 1, 0, 0, 0, 0, 0, 2, 0, 1]);
        server.extend([0; 20]);
        server.extend([0, 0, 0, 2]);
        server.extend([0, 0, 0, 0, 0, 1, 0, 1]);
        server.extend(CURSOR.to_be_bytes());
        server.extend([255, 255, 255, 0, 128]);
        server.extend([0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 7, 8, 9, 0]);
        let mut input = &server[..];
        let mut client = Vec::new();
        let mut frame = connect(&mut input, &mut client).unwrap();
        assert_eq!(&client[..14], b"RFB 003.008\n\x01\x00");
        assert_eq!(
            &client[34..],
            &[
                2, 0, 0, 3, 0, 0, 0, 0, 255, 255, 254, 204, 255, 255, 255, 17
            ]
        );
        assert_eq!(request(&frame, true), [3, 1, 0, 0, 0, 0, 0, 2, 0, 1]);
        update(&mut input, &mut frame, &mut Decoder::default()).unwrap();
        assert_eq!(frame.rgba, [0, 0, 0, 0, 9, 8, 7, 255]);
        assert_eq!(
            frame.dirty,
            Some(Damage {
                x: 1,
                y: 0,
                right: 2,
                bottom: 1
            })
        );
    }

    #[test]
    fn tight_decodes_jpeg_and_reuses_a_deflate_stream() {
        use image::ImageEncoder;
        let mut frame = Frame {
            width: 4,
            height: 2,
            rgba: vec![0; 32],
            layout: None,
            dirty: None,
        };
        let rect = Damage {
            x: 0,
            y: 0,
            right: 4,
            bottom: 2,
        };
        let rgb = [210u8, 40, 70].repeat(8);
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .write_image(&rgb, 4, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let packet = |mode, data: &[u8]| {
            let len = data.len();
            assert!(len < 16384);
            let mut bytes = vec![mode];
            if len < 128 {
                bytes.push(len as u8);
            } else {
                bytes.extend([(len as u8 & 127) | 128, (len >> 7) as u8]);
            }
            bytes.extend(data);
            bytes
        };
        let mut decoder = Decoder::default();
        decoder
            .tight(&mut &packet(0x90, &jpeg)[..], &mut frame, rect)
            .unwrap();
        assert!(
            frame
                .rgba
                .chunks_exact(4)
                .all(|p| p[0] > 200 && p[1] < 50 && p[3] == 255)
        );
        let mut stream = flate2::Compress::new(flate2::Compression::fast(), true);
        for (mode, color) in [(1, [30, 44, 125]), (0, [201, 131, 90])] {
            let mut compressed = Vec::with_capacity(128);
            stream
                .compress_vec(
                    &color.repeat(8),
                    &mut compressed,
                    flate2::FlushCompress::Sync,
                )
                .unwrap();
            decoder
                .tight(&mut &packet(mode, &compressed)[..], &mut frame, rect)
                .unwrap();
            assert!(
                frame
                    .rgba
                    .chunks_exact(4)
                    .all(|p| p[..3] == color && p[3] == 255)
            );
        }
    }

    #[test]
    fn refuses_out_of_bounds_pixels_and_oversized_clipboard() {
        let mut frame = Frame {
            layout: None,
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            dirty: None,
        };
        let data = [0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0];
        assert_eq!(
            update(&mut &data[..], &mut frame, &mut Decoder::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(update(&mut &[3][..], &mut frame, &mut Decoder::default()).is_err());
        let mut excessive = vec![3, 0, 0, 0];
        excessive.extend(((MAX_IGNORED_CUT_TEXT + 1) as u32).to_be_bytes());
        assert_eq!(
            update(&mut &excessive[..], &mut frame, &mut Decoder::default())
                .unwrap_err()
                .to_string(),
            "Display clipboard message is too large"
        );
        assert_eq!(frame.rgba, [0; 4]);
    }

    #[test]
    fn tight_single_color_palette_keeps_following_rectangles_in_sync() {
        let mut frame = Frame {
            width: 9,
            height: 2,
            rgba: vec![0; 9 * 2 * 4],
            layout: None,
            dirty: None,
        };
        let mut wire = vec![0, 0, 255, 255];
        // A nine-pixel row uses two packed bytes, even with only one color.
        wire.extend([0, 0, 0, 0, 0, 9, 0, 2, 0, 0, 0, 7]);
        wire.extend([0x40, 1, 0, 70, 80, 90, 0, 0, 0, 0]);
        // A second rectangle in the same update, and then a new update.
        wire.extend([0, 8, 0, 1, 0, 1, 0, 1, 0, 0, 0, 7, 0x80, 1, 2, 3]);
        wire.extend([0; 8]);
        wire.extend(LAST_RECT.to_be_bytes());
        wire.extend([0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 9, 8, 7, 0]);
        let mut input = &wire[..];
        let mut decoder = Decoder::default();
        update(&mut input, &mut frame, &mut decoder).unwrap();
        assert!(
            frame.rgba[..68]
                .chunks_exact(4)
                .all(|p| p == [70, 80, 90, 255])
        );
        assert_eq!(&frame.rgba[68..], &[1, 2, 3, 255]);
        update(&mut input, &mut frame, &mut decoder).unwrap();
        assert_eq!(&frame.rgba[..4], &[7, 8, 9, 255]);
        assert!(input.is_empty());
    }

    #[test]
    fn skips_clipboard_messages_without_losing_the_next_frame() {
        let mut frame = Frame {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            layout: None,
            dirty: None,
        };
        let mut messages = vec![3, 0, 0, 0, 0, 0, 0, 3, b'x', 0, b'y', 2, 3, 0, 0, 0];
        messages.extend((-4i32).to_be_bytes());
        messages.extend([1, 0, 0, 0]);
        messages.extend([
            0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 7, 0x80, 40, 50, 60,
        ]);
        update(&mut &messages[..], &mut frame, &mut Decoder::default()).unwrap();
        assert_eq!(frame.rgba, [40, 50, 60, 255]);
    }

    #[test]
    fn keeps_4k_and_portrait_sizes_and_limits_oversized_displays() {
        assert_eq!(Resolution::fit(3840.0, 2160.0), Resolution(3840, 2160));
        assert_eq!(Resolution::fit(2160.0, 3840.0), Resolution(2160, 3840));
        assert_eq!(Resolution::fit(7680.0, 4320.0), Resolution(3965, 2230));
        assert_eq!(Resolution::fit(f32::NAN, 2160.0), Resolution::FALLBACK);
        assert!(!Resolution(4096, 4096).valid());
        assert_eq!(Resolution(3840, 2160).video_size(), Resolution(1920, 1080));
        assert_eq!(Resolution(2160, 3840).video_size(), Resolution(1080, 1920));
        assert_eq!(Resolution(960, 540).video_size(), Resolution(960, 540));
    }
}
