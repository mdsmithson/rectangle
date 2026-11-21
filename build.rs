#[path = "src/artwork.rs"]
mod artwork;

#[path = "src/mark.rs"]
mod mark;

fn main() {
    use std::{fs, io::Write, path::PathBuf, process::Command};
    println!("cargo:rerun-if-changed=src/mark.rs");
    println!("cargo:rerun-if-changed=src/artwork.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("icon.rgba"), artwork::icon(64)).unwrap();
    let mut texture = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    texture.write_all(&artwork::marble()).unwrap();
    fs::write(out.join("marble.rgba.zlib"), texture.finish().unwrap()).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let mut images = Vec::new();
    for size in [16_u32, 32, 48, 64, 128] {
        let mut dib = Vec::new();
        for field in [40, size, size * 2] {
            dib.extend(field.to_le_bytes());
        }
        dib.extend(1_u16.to_le_bytes());
        dib.extend(32_u16.to_le_bytes());
        dib.extend([0_u8; 24]);
        let rgba = artwork::icon(size);
        for row in rgba.chunks_exact(size as usize * 4).rev() {
            for p in row.chunks_exact(4) {
                dib.extend([p[2], p[1], p[0], p[3]]);
            }
        }
        dib.resize(dib.len() + (size.div_ceil(32) * 4 * size) as usize, 0);
        images.push((size, dib));
    }
    let mut ico = vec![0, 0, 1, 0, images.len() as u8, 0];
    let mut offset = (6 + 16 * images.len()) as u32;
    for (size, dib) in &images {
        ico.extend([*size as u8, *size as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend((dib.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += dib.len() as u32;
    }
    for (_, dib) in images {
        ico.extend(dib);
    }
    fs::write(out.join("rectangle.ico"), ico).unwrap();
    fs::write(out.join("rectangle.rc"), "1 ICON \"rectangle.ico\"\n").unwrap();
    let mut candidates = vec![PathBuf::from("rc.exe")];
    if let Some(base) = std::env::var_os("ProgramFiles(x86)") {
        let bin = PathBuf::from(base).join("Windows Kits/10/bin");
        if let Ok(versions) = fs::read_dir(bin) {
            let mut versions: Vec<_> = versions.flatten().map(|p| p.path()).collect();
            versions.sort();
            candidates.extend(versions.into_iter().rev().map(|p| p.join("x64/rc.exe")));
        }
    }
    if candidates.into_iter().any(|path| {
        Command::new(path)
            .current_dir(&out)
            .args(["/nologo", "/fo", "rectangle.res", "rectangle.rc"])
            .status()
            .is_ok_and(|s| s.success())
    }) {
        println!(
            "cargo:rustc-link-arg-bin=rectangle={}",
            out.join("rectangle.res").display()
        );
    } else {
        println!(
            "cargo:warning=Windows resource compiler unavailable; the in-app icon is still set"
        );
    }
}
