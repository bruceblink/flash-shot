//! Viewport-to-physical-pixel mapping for contained screenshot previews.

use super::geometry::{PhysicalPoint, PhysicalRect};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewRect {
    pub const fn right(self) -> f32 {
        self.left + self.width
    }

    pub const fn bottom(self) -> f32 {
        self.top + self.height
    }

    pub fn contains(self, point: ViewPoint) -> bool {
        point.x >= self.left
            && point.x <= self.right()
            && point.y >= self.top
            && point.y <= self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewTransform {
    image_bounds: PhysicalRect,
    fitted_view: ViewRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PreviewTransform {
    pub fn contain(image_bounds: PhysicalRect, viewport: ViewRect) -> Option<Self> {
        let image_width = image_bounds.width() as f32;
        let image_height = image_bounds.height() as f32;
        if image_width == 0.0
            || image_height == 0.0
            || viewport.width <= 0.0
            || viewport.height <= 0.0
        {
            return None;
        }
        let scale = (viewport.width / image_width).min(viewport.height / image_height);
        let width = image_width * scale;
        let height = image_height * scale;
        Some(Self {
            image_bounds,
            fitted_view: ViewRect {
                left: viewport.left + (viewport.width - width) / 2.0,
                top: viewport.top + (viewport.height - height) / 2.0,
                width,
                height,
            },
        })
    }

    pub const fn fitted_view(self) -> ViewRect {
        self.fitted_view
    }

    pub fn view_to_physical(self, point: ViewPoint) -> Option<PhysicalPoint> {
        if !self.fitted_view.contains(point) {
            return None;
        }
        let normalized_x =
            ((point.x - self.fitted_view.left) / self.fitted_view.width).clamp(0.0, 1.0);
        let normalized_y =
            ((point.y - self.fitted_view.top) / self.fitted_view.height).clamp(0.0, 1.0);
        Some(PhysicalPoint {
            x: self.image_bounds.left
                + (normalized_x * self.image_bounds.width() as f32).round() as i32,
            y: self.image_bounds.top
                + (normalized_y * self.image_bounds.height() as f32).round() as i32,
        })
    }

    pub fn physical_to_view(self, point: PhysicalPoint) -> ViewPoint {
        let normalized_x =
            (point.x - self.image_bounds.left) as f32 / self.image_bounds.width().max(1) as f32;
        let normalized_y =
            (point.y - self.image_bounds.top) as f32 / self.image_bounds.height().max(1) as f32;
        ViewPoint {
            x: self.fitted_view.left + normalized_x * self.fitted_view.width,
            y: self.fitted_view.top + normalized_y * self.fitted_view.height,
        }
    }

    pub fn resize_handle_at(
        self,
        selection: PhysicalRect,
        point: ViewPoint,
        hit_radius: f32,
    ) -> Option<ResizeHandle> {
        let handles = [
            (
                ResizeHandle::TopLeft,
                PhysicalPoint {
                    x: selection.left,
                    y: selection.top,
                },
            ),
            (
                ResizeHandle::TopRight,
                PhysicalPoint {
                    x: selection.right,
                    y: selection.top,
                },
            ),
            (
                ResizeHandle::BottomLeft,
                PhysicalPoint {
                    x: selection.left,
                    y: selection.bottom,
                },
            ),
            (
                ResizeHandle::BottomRight,
                PhysicalPoint {
                    x: selection.right,
                    y: selection.bottom,
                },
            ),
        ];

        handles.into_iter().find_map(|(handle, physical)| {
            let view = self.physical_to_view(physical);
            ((view.x - point.x).abs() <= hit_radius && (view.y - point.y).abs() <= hit_radius)
                .then_some(handle)
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectionDrag {
    anchor: Option<PhysicalPoint>,
    current: Option<PhysicalPoint>,
}

impl SelectionDrag {
    pub fn begin(&mut self, point: PhysicalPoint) {
        self.anchor = Some(point);
        self.current = Some(point);
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        if self.anchor.is_some() {
            self.current = Some(point);
        }
    }

    pub fn begin_resize(&mut self, selection: PhysicalRect, handle: ResizeHandle) {
        let (anchor, current) = match handle {
            ResizeHandle::TopLeft => (
                PhysicalPoint {
                    x: selection.right,
                    y: selection.bottom,
                },
                PhysicalPoint {
                    x: selection.left,
                    y: selection.top,
                },
            ),
            ResizeHandle::TopRight => (
                PhysicalPoint {
                    x: selection.left,
                    y: selection.bottom,
                },
                PhysicalPoint {
                    x: selection.right,
                    y: selection.top,
                },
            ),
            ResizeHandle::BottomLeft => (
                PhysicalPoint {
                    x: selection.right,
                    y: selection.top,
                },
                PhysicalPoint {
                    x: selection.left,
                    y: selection.bottom,
                },
            ),
            ResizeHandle::BottomRight => (
                PhysicalPoint {
                    x: selection.left,
                    y: selection.top,
                },
                PhysicalPoint {
                    x: selection.right,
                    y: selection.bottom,
                },
            ),
        };
        self.anchor = Some(anchor);
        self.current = Some(current);
    }

    pub fn selection(self) -> Option<PhysicalRect> {
        Some(PhysicalRect::new(self.anchor?, self.current?))
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewTransform, ResizeHandle, SelectionDrag, ViewPoint, ViewRect};
    use crate::domain::geometry::{PhysicalPoint, PhysicalRect};

    fn image_bounds() -> PhysicalRect {
        PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        }
    }

    #[test]
    fn contained_preview_letterboxes_without_changing_pixel_mapping() {
        let transform = PreviewTransform::contain(
            image_bounds(),
            ViewRect {
                left: 100.0,
                top: 50.0,
                width: 800.0,
                height: 800.0,
            },
        )
        .unwrap();

        assert_eq!(transform.fitted_view().left, 100.0);
        assert_eq!(transform.fitted_view().width, 800.0);
        assert_eq!(transform.fitted_view().height, 450.0);
        assert_eq!(
            transform.view_to_physical(ViewPoint { x: 500.0, y: 450.0 }),
            Some(PhysicalPoint { x: -960, y: 540 })
        );
    }

    #[test]
    fn letterbox_click_is_not_a_physical_pixel() {
        let transform = PreviewTransform::contain(
            image_bounds(),
            ViewRect {
                left: 0.0,
                top: 0.0,
                width: 800.0,
                height: 800.0,
            },
        )
        .unwrap();

        assert_eq!(
            transform.view_to_physical(ViewPoint { x: 400.0, y: 10.0 }),
            None
        );
    }

    #[test]
    fn reverse_drag_produces_normalized_physical_selection() {
        let mut drag = SelectionDrag::default();
        drag.begin(PhysicalPoint { x: -100, y: 900 });
        drag.update(PhysicalPoint { x: -1000, y: 100 });

        assert_eq!(
            drag.selection(),
            Some(PhysicalRect {
                left: -1000,
                top: 100,
                right: -100,
                bottom: 900,
            })
        );
    }

    #[test]
    fn resize_handle_hit_testing_uses_preview_coordinates() {
        let transform = PreviewTransform::contain(
            image_bounds(),
            ViewRect {
                left: 0.0,
                top: 0.0,
                width: 960.0,
                height: 540.0,
            },
        )
        .unwrap();
        let selection = PhysicalRect {
            left: -1600,
            top: 200,
            right: -800,
            bottom: 800,
        };

        let top_right = transform.physical_to_view(PhysicalPoint {
            x: selection.right,
            y: selection.top,
        });
        assert_eq!(
            transform.resize_handle_at(
                selection,
                ViewPoint {
                    x: top_right.x + 5.0,
                    y: top_right.y - 4.0,
                },
                8.0,
            ),
            Some(ResizeHandle::TopRight)
        );
        assert_eq!(
            transform.resize_handle_at(selection, ViewPoint { x: 480.0, y: 270.0 }, 8.0),
            None
        );
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_fixed_when_crossing_it() {
        let selection = PhysicalRect {
            left: 100,
            top: 200,
            right: 500,
            bottom: 600,
        };
        let mut drag = SelectionDrag::default();
        drag.begin_resize(selection, ResizeHandle::TopLeft);
        drag.update(PhysicalPoint { x: 700, y: 800 });

        assert_eq!(
            drag.selection(),
            Some(PhysicalRect {
                left: 500,
                top: 600,
                right: 700,
                bottom: 800,
            })
        );
    }
}
