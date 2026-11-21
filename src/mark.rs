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
