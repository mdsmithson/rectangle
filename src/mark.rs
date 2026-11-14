pub const COLORS: [[u8; 3]; 2] = [[176, 224, 230], [255, 255, 255]];
pub const COUNT: usize = 8;
pub const STROKE: f32 = 1.0 / 30.0;

pub fn square(index: usize, elapsed: f32) -> [[f32; 2]; 4] {
    let phase = elapsed.rem_euclid(5.4);
    let t = phase.min(5.4 - phase);
    let progress = ((t - index as f32 * 0.1) / 2.0).clamp(0.0, 1.0);
    let ease = if progress < 0.5 {
        4.0 * progress.powi(3)
    } else {
        1.0 - (-2.0 * progress + 2.0).powi(3) * 0.5
    };
    let angle = (index as f32 / 12.0 + ease) * std::f32::consts::PI;
    let (s, c) = angle.sin_cos();
    let half = 0.5 * 0.8_f32.powi(index as i32);
    [[-half, -half], [half, -half], [half, half], [-half, half]]
        .map(|[x, y]| [x * c - y * s, x * s + y * c])
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
