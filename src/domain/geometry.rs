//! Physical-pixel geometry shared by capture and selection workflows.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub fn new(start: PhysicalPoint, end: PhysicalPoint) -> Self {
        Self {
            left: start.x.min(end.x),
            top: start.y.min(end.y),
            right: start.x.max(end.x),
            bottom: start.y.max(end.y),
        }
    }

    pub const fn width(self) -> u32 {
        self.right.saturating_sub(self.left) as u32
    }

    pub const fn height(self) -> u32 {
        self.bottom.saturating_sub(self.top) as u32
    }

    pub const fn contains(self, point: PhysicalPoint) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    pub const fn translate_to_local(self, point: PhysicalPoint) -> Option<PhysicalPoint> {
        if !self.contains(point) {
            return None;
        }
        Some(PhysicalPoint {
            x: point.x - self.left,
            y: point.y - self.top,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PhysicalPoint, PhysicalRect};

    #[test]
    fn normalizes_drag_direction_across_negative_coordinates() {
        let rect = PhysicalRect::new(
            PhysicalPoint { x: 300, y: 200 },
            PhysicalPoint { x: -1200, y: -100 },
        );

        assert_eq!(rect.left, -1200);
        assert_eq!(rect.top, -100);
        assert_eq!(rect.width(), 1500);
        assert_eq!(rect.height(), 300);
    }

    #[test]
    fn translates_virtual_desktop_point_to_monitor_pixels() {
        let monitor = PhysicalRect {
            left: -2560,
            top: 0,
            right: 0,
            bottom: 1440,
        };

        assert_eq!(
            monitor.translate_to_local(PhysicalPoint { x: -1280, y: 720 }),
            Some(PhysicalPoint { x: 1280, y: 720 })
        );
        assert_eq!(
            monitor.translate_to_local(PhysicalPoint { x: 0, y: 720 }),
            None
        );
    }
}
