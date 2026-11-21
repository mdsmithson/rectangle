use crate::mark;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};

const UV: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
pub const EDGE: Color32 = Color32::from_rgb(51, 55, 65);
pub const SURFACE: Color32 = Color32::from_rgb(16, 17, 21);
pub const ACCENT: Color32 = Color32::from_rgb(176, 224, 230);

pub fn theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(9, 10, 12);
    v.window_fill = SURFACE;
    v.window_corner_radius = 16.into();
    v.window_stroke = Stroke::new(1.0, EDGE);
    v.override_text_color = Some(crate::INK);
    v.selection.bg_fill = Color32::from_rgb(42, 66, 76);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.faint_bg_color = Color32::from_rgb(20, 22, 27);
    v.extreme_bg_color = Color32::from_rgb(8, 9, 12);
    for (widget, fill) in [
        (&mut v.widgets.noninteractive, SURFACE),
        (&mut v.widgets.inactive, Color32::from_rgb(30, 32, 38)),
        (&mut v.widgets.hovered, Color32::from_rgb(44, 48, 56)),
        (&mut v.widgets.active, Color32::from_rgb(50, 64, 70)),
        (&mut v.widgets.open, Color32::from_rgb(37, 43, 50)),
    ] {
        widget.corner_radius = 8.into();
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, EDGE);
    }
    ctx.set_visuals(v);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.button_padding = Vec2::new(10.0, 5.0);
            style.spacing.interact_size.y = 29.0;
        });
    }
}

pub fn marble(ctx: &egui::Context) -> egui::TextureHandle {
    use std::io::Read;
    let mut pixels = Vec::with_capacity(768 * 512 * 4);
    flate2::read::ZlibDecoder::new(
        &include_bytes!(concat!(env!("OUT_DIR"), "/marble.rgba.zlib"))[..],
    )
    .read_to_end(&mut pixels)
    .expect("build-generated marble texture");
    ctx.load_texture(
        "Rectangle black marble",
        egui::ColorImage::from_rgba_unmultiplied([768, 512], &pixels),
        egui::TextureOptions::LINEAR,
    )
}

pub fn background(ui: &egui::Ui, texture: &egui::TextureHandle, rect: Rect) {
    ui.painter().add(
        egui::epaint::RectShape::filled(rect, 19, Color32::WHITE).with_texture(texture.id(), UV),
    );
    ui.painter()
        .rect_stroke(rect, 19, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
}

pub fn frame(fill: Color32, radius: u8, margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, EDGE))
        .corner_radius(radius)
        .inner_margin(egui::Margin::same(margin))
}

pub fn spiral(painter: &egui::Painter, center: Pos2, size: f32, elapsed: f32) {
    for i in 0..mark::COUNT {
        let path = mark::square(i, elapsed).map(|[x, y]| center + Vec2::new(x, y) * size);
        let [r, g, b] = mark::COLORS[i % 2];
        painter.add(Shape::closed_line(
            path.to_vec(),
            Stroke::new((size * mark::STROKE).max(0.65), Color32::from_rgb(r, g, b)),
        ));
    }
}

pub fn loading(ui: &mut egui::Ui, title: &str, elapsed: f32) {
    let (pane, response) = ui.allocate_exact_size(
        ui.available_size().max(Vec2::splat(1.0)),
        egui::Sense::hover(),
    );
    let response = response.on_hover_text("Connecting over SSH and starting the remote browser");
    let side = (pane.width() - 32.0)
        .min((pane.height() - 99.0).max(60.0))
        .clamp(60.0, 264.0);
    let center = pane.center() - Vec2::new(0.0, 32.0);
    let card = Rect::from_center_size(center, Vec2::splat(side));
    let p = ui.painter_at(pane);
    for i in (0..18).rev() {
        p.rect_filled(
            card.expand(i as f32 * 1.3),
            27,
            Color32::from_rgba_unmultiplied(80, 150, 200, 3),
        );
    }
    let top = Color32::from_rgb(33, 56, 75);
    let bottom = Color32::from_rgb(12, 18, 28);
    let mut mesh = egui::epaint::Mesh::default();
    let mut border = Vec::new();
    let radius = 23.0;
    for (corner, start) in [
        (card.right_top() + Vec2::new(-radius, radius), -90.0_f32),
        (card.right_bottom() - Vec2::splat(radius), 0.0),
        (card.left_bottom() + Vec2::new(radius, -radius), 90.0),
        (card.left_top() + Vec2::splat(radius), 180.0),
    ] {
        for i in 0..=8 {
            let a = (start + i as f32 * 90.0 / 8.0).to_radians();
            border.push(corner + Vec2::new(a.cos(), a.sin()) * radius);
        }
    }
    mesh.colored_vertex(center, top.lerp_to_gamma(bottom, 0.5));
    for point in &border {
        let t = (point.y - card.top()) / card.height();
        mesh.colored_vertex(*point, top.lerp_to_gamma(bottom, t));
    }
    for i in 0..border.len() {
        mesh.add_triangle(0, (i + 1) as u32, ((i + 1) % border.len() + 1) as u32);
    }
    p.add(mesh);
    p.rect_stroke(
        card,
        23,
        Stroke::new(1.0, Color32::from_rgb(55, 85, 105)),
        egui::StrokeKind::Inside,
    );
    spiral(&p, center, side * 0.67, elapsed);
    let y = card.bottom() + 16.0;
    p.text(
        Pos2::new(center.x, y),
        Align2::CENTER_TOP,
        "Starting browser session",
        FontId::proportional(20.0),
        crate::INK,
    );
    let name = if title.chars().count() > 38 {
        format!("{}…", title.chars().take(37).collect::<String>())
    } else {
        title.into()
    };
    p.text(
        Pos2::new(center.x, y + 31.0),
        Align2::CENTER_TOP,
        name,
        FontId::proportional(13.0),
        crate::MUTED,
    );
    response
        .ctx
        .request_repaint_after(std::time::Duration::from_millis(16));
}

pub fn remote_cursor(ui: &egui::Ui, rect: Rect, hotspot: Pos2) {
    let scale = 3.0 / ui.ctx().pixels_per_point();
    let points = [
        (0.0, 0.0),
        (0.0, 17.0),
        (4.4, 12.7),
        (8.1, 19.1),
        (11.0, 17.5),
        (7.4, 11.4),
        (13.1, 11.4),
    ]
    .map(|(x, y)| hotspot + Vec2::new(x, y) * scale);
    let mut mesh = egui::Mesh::default();
    for point in points {
        mesh.colored_vertex(point, Color32::from_rgb(240, 38, 53));
    }
    for [a, b, c] in [[0, 1, 2], [0, 2, 5], [0, 5, 6], [2, 3, 4], [2, 4, 5]] {
        mesh.add_triangle(a, b, c);
    }
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::closed_line(
        points.to_vec(),
        Stroke::new(1.3, Color32::from_rgb(65, 7, 15)),
    ));
}

pub fn fit_desktop(pane: Rect, [width, height]: [u16; 2]) -> Rect {
    let size = Vec2::new(f32::from(width), f32::from(height));
    let scale = (pane.width() / size.x).min(pane.height() / size.y);
    Rect::from_center_size(pane.center(), size * scale)
}

pub fn display(ui: &egui::Ui, id: egui::TextureId, rect: Rect) {
    ui.painter()
        .add(egui::epaint::RectShape::filled(rect, 12, Color32::WHITE).with_texture(id, UV));
    ui.painter()
        .rect_stroke(rect, 12, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside);
}
