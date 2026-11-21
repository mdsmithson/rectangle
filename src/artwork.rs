use crate::mark::{COLORS, COUNT, STROKE, square};

fn hash(x: i32, y: i32) -> f32 {
    let mut v = (x as u32).wrapping_mul(0x6d2b79f5) ^ (y as u32).wrapping_mul(0x27d4eb2d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x85ebca6b);
    v ^= v >> 13;
    (v & 65535) as f32 / 65535.0
}

fn noise(x: f32, y: f32) -> f32 {
    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let smooth = |v: f32| v * v * (3.0 - 2.0 * v);
    let (a, b) = (smooth(x - x.floor()), smooth(y - y.floor()));
    let mix = |x, y, t| x + (y - x) * t;
    mix(
        mix(hash(i, j), hash(i + 1, j), a),
        mix(hash(i, j + 1), hash(i + 1, j + 1), a),
        b,
    )
}

fn stone(x: f32, y: f32) -> [u8; 4] {
    let warp = noise(x * 3.1 + 3.0, y * 3.1) * 0.62
        + noise(x * 7.1, y * 7.1) * 0.22
        + noise(x * 16.0, y * 16.0) * 0.06;
    let path = x * 0.79 + y * 1.5 + warp - 0.23;
    let ridge = ((path * std::f32::consts::TAU * 1.45).sin()).abs();
    let vein = (1.0 - ridge).powi(35) * 29.0 + (1.0 - ridge).powi(6) * 11.0;
    let glint = (1.0 - ((x - 0.21) * 0.8 + y * 1.65).abs()).max(0.0).powi(3) * 17.0;
    let grain = noise(x * 219.0, y * 219.0) * 3.0;
    let base = (9.0 + glint + noise(x * 5.0, y * 5.0) * 8.0 + vein + grain) as u8;
    [base, base + 1, base + 4, 255]
}

pub const WIDTH: usize = 768;
pub const HEIGHT: usize = 512;

pub fn marble() -> Vec<u8> {
    (0..HEIGHT)
        .flat_map(|y| {
            (0..WIDTH).flat_map(move |x| stone(x as f32 / WIDTH as f32, y as f32 / HEIGHT as f32))
        })
        .collect()
}

pub fn icon(size: u32) -> Vec<u8> {
    let squares = std::array::from_fn::<_, COUNT, _>(|i| square(i, 0.0));
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0_u32; 4];
            for yy in 0..4 {
                for xx in 0..4 {
                    let u = (x as f32 + (xx as f32 + 0.5) / 4.0) / size as f32;
                    let v = (y as f32 + (yy as f32 + 0.5) / 4.0) / size as f32;
                    let corner = ((u - 0.5).abs() - 0.30)
                        .max(0.0)
                        .hypot(((v - 0.5).abs() - 0.30).max(0.0));
                    if corner > 0.17 {
                        continue;
                    }
                    let gloss = ((1.0 - v) * 25.0) as u8;
                    let mut rgb = [9 + gloss / 2, 18 + gloss, 29 + gloss];
                    let p = [(u - 0.5) / 0.72, (v - 0.5) / 0.72];
                    for (i, corners) in squares.iter().enumerate() {
                        for j in 0..4 {
                            let [ax, ay] = corners[j];
                            let [bx, by] = corners[(j + 1) % 4];
                            let (dx, dy) = (bx - ax, by - ay);
                            let t = (((p[0] - ax) * dx + (p[1] - ay) * dy) / (dx * dx + dy * dy))
                                .clamp(0.0, 1.0);
                            if (p[0] - ax - t * dx).hypot(p[1] - ay - t * dy) < STROKE * 0.68 {
                                rgb = COLORS[i % 2];
                            }
                        }
                    }
                    for i in 0..3 {
                        sum[i] += u32::from(rgb[i]);
                    }
                    sum[3] += 1;
                }
            }
            let a = sum[3];
            rgba.extend([
                if a > 0 { (sum[0] / a) as u8 } else { 0 },
                if a > 0 { (sum[1] / a) as u8 } else { 0 },
                if a > 0 { (sum[2] / a) as u8 } else { 0 },
                (a * 255 / 16) as u8,
            ]);
        }
    }
    rgba
}
