use eframe::egui::{Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Snap {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Right,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::TopLeft => "tl",
            Self::TopRight => "tr",
            Self::BottomLeft => "bl",
            Self::BottomRight => "br",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.code() == code)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left half",
            Self::Right => "Right half",
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }

    fn mask(self) -> u8 {
        match self {
            Self::Left => 5,
            Self::Right => 10,
            Self::TopLeft => 1,
            Self::TopRight => 2,
            Self::BottomLeft => 4,
            Self::BottomRight => 8,
        }
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.mask() & other.mask() != 0
    }

    pub fn vacant(occupied: impl Iterator<Item = Self>) -> impl Iterator<Item = Self> {
        let used = occupied.fold(0, |mask, slot| mask | slot.mask());
        [Self::Left, Self::Right]
            .into_iter()
            .flat_map(move |half| {
                let [top, bottom] = if half == Self::Left {
                    [Self::TopLeft, Self::BottomLeft]
                } else {
                    [Self::TopRight, Self::BottomRight]
                };
                if used & half.mask() == 0 {
                    [Some(half), None]
                } else {
                    [top, bottom].map(|slot| (used & slot.mask() == 0).then_some(slot))
                }
            })
            .flatten()
    }

    pub fn rect(self, area: Rect) -> Rect {
        let m = self.mask();
        let x0 = if m & 5 != 0 { 0.0 } else { 0.5 };
        let x1 = if m & 10 != 0 { 1.0 } else { 0.5 };
        let y0 = if m & 3 != 0 { 0.0 } else { 0.5 };
        let y1 = if m & 12 != 0 { 1.0 } else { 0.5 };
        Rect::from_min_max(
            area.min + area.size() * Vec2::new(x0, y0),
            area.min + area.size() * Vec2::new(x1, y1),
        )
        .shrink(3.0)
    }

    pub fn target(area: Rect, point: Pos2) -> Option<Self> {
        if !area.contains(point) {
            return None;
        }
        let p = (point - area.min) / area.size();
        match (p.x < 0.24, p.x > 0.76, p.y < 0.29, p.y > 0.71) {
            (true, _, true, _) => Some(Self::TopLeft),
            (_, true, true, _) => Some(Self::TopRight),
            (true, _, _, true) => Some(Self::BottomLeft),
            (_, true, _, true) => Some(Self::BottomRight),
            (true, _, _, _) => Some(Self::Left),
            (_, true, _, _) => Some(Self::Right),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Snap::*;

    #[test]
    fn vacant_tiles_combine_empty_halves_and_leave_occupied_panes_alone() {
        for (occupied, expected) in [
            (vec![Left], vec![Right]),
            (vec![TopLeft], vec![BottomLeft, Right]),
            (vec![Left, TopRight], vec![BottomRight]),
            (vec![TopLeft, BottomRight], vec![BottomLeft, TopRight]),
            (vec![Left, TopRight, BottomRight], vec![]),
        ] {
            assert_eq!(
                super::Snap::vacant(occupied.into_iter()).collect::<Vec<_>>(),
                expected
            );
        }
    }
}
