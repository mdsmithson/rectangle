// Reuse the application's artwork; sips/iconutil convert this lossless TGA.
#[allow(dead_code)]
#[path = "../src/artwork.rs"]
mod artwork;
#[path = "../src/mark.rs"]
mod mark;

fn main() -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(std::env::args_os().nth(1).expect("output TGA path"))?;
    let size = 1024_u16;
    let mut header = [0; 18];
    header[2] = 2; // Uncompressed true-color, BGRA, top-left origin.
    header[12..14].copy_from_slice(&size.to_le_bytes());
    header[14..16].copy_from_slice(&size.to_le_bytes());
    header[16] = 32;
    header[17] = 0x28;
    file.write_all(&header)?;
    let mut pixels = artwork::icon(u32::from(size));
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    file.write_all(&pixels)
}
