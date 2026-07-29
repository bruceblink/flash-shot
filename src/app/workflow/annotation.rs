//! Annotation editing, keyboard commands, and inspection workflows.

use super::*;
use crate::app::TextEdit;

impl FlashShotApp {
    pub(in crate::app) fn select_rectangle_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Rectangle, cx);
    }

    pub(in crate::app) fn select_watermark_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Watermark, cx);
    }

    pub(in crate::app) fn select_text_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Text, cx);
    }

    pub(in crate::app) fn select_highlight_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Highlight, cx);
    }

    pub(in crate::app) fn select_mosaic_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Mosaic, cx);
    }

    pub(in crate::app) fn select_blur_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Blur, cx);
    }

    pub(in crate::app) fn select_number_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Number, cx);
    }

    pub(in crate::app) fn adjust_selected_number(
        &mut self,
        delta: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id).cloned() else {
            self.selected_annotation = None;
            return false;
        };
        let AnnotationKind::Number { center, value } = existing.kind.clone() else {
            return false;
        };
        let value = adjusted_number_value(value, delta);
        if value
            == match existing.kind {
                AnnotationKind::Number { value, .. } => value,
                _ => unreachable!(),
            }
        {
            return true;
        }
        let replacement = Annotation {
            kind: AnnotationKind::Number { center, value },
            ..existing
        };
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Replace(replacement))
        {
            Ok(()) => {
                self.status = format!("Number marker: {value}");
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn select_ellipse_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Ellipse, cx);
    }

    pub(in crate::app) fn select_line_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Line, cx);
    }

    pub(in crate::app) fn select_arrow_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Arrow, cx);
    }

    pub(in crate::app) fn select_freehand_tool(&mut self, cx: &mut Context<Self>) {
        self.select_annotation_tool(AnnotationTool::Freehand, cx);
    }

    pub(in crate::app) fn select_annotation_color(&mut self, color: u32, cx: &mut Context<Self>) {
        let opacity = self.annotation_style.stroke_rgba as u8;
        self.annotation_style.stroke_rgba = with_alpha(color, opacity);
        if self.selected_annotation.is_some() {
            self.annotation_style.fill_rgba =
                self.annotation_style.fill_rgba.map(|_| fill_color(color));
        }
        self.replace_selected_annotation_style(cx);
        self.status = "Annotation color selected".to_owned();
        cx.notify();
    }

    pub(in crate::app) fn select_annotation_width(&mut self, width: u32, cx: &mut Context<Self>) {
        self.annotation_style.stroke_width = width.max(1);
        self.replace_selected_annotation_style(cx);
        self.status = format!(
            "Annotation width: {} px",
            self.annotation_style.stroke_width
        );
        cx.notify();
    }

    pub(in crate::app) fn select_annotation_font_size(
        &mut self,
        font_size: u32,
        cx: &mut Context<Self>,
    ) {
        self.annotation_style.text_font_size = font_size.max(1);
        self.replace_selected_annotation_style(cx);
        self.status = format!("Text size: {} px", self.annotation_style.text_font_size);
        cx.notify();
    }

    pub(in crate::app) fn select_annotation_opacity(
        &mut self,
        opacity: u8,
        cx: &mut Context<Self>,
    ) {
        self.annotation_style.stroke_rgba = with_alpha(self.annotation_style.stroke_rgba, opacity);
        if let Some(fill) = self.annotation_style.fill_rgba {
            self.annotation_style.fill_rgba = Some(with_alpha(fill, fill_alpha(opacity)));
        }
        self.replace_selected_annotation_style(cx);
        self.status = format!("Annotation opacity: {}%", u16::from(opacity) * 100 / 255);
        cx.notify();
    }

    pub(in crate::app) fn toggle_annotation_fill(&mut self, cx: &mut Context<Self>) {
        let supported = self
            .selected_annotation
            .and_then(|id| self.annotation_document.as_ref()?.annotation(id))
            .is_some_and(Annotation::supports_fill)
            || self
                .annotation_tool
                .is_some_and(AnnotationTool::supports_fill);
        if !supported {
            self.status = "Fill is available for rectangles and ellipses".to_owned();
            cx.notify();
            return;
        }
        self.annotation_style.fill_rgba = self
            .annotation_style
            .fill_rgba
            .is_none()
            .then(|| fill_color(self.annotation_style.stroke_rgba));
        self.replace_selected_annotation_style(cx);
        self.status = if self.annotation_style.fill_rgba.is_some() {
            "Shape fill enabled"
        } else {
            "Shape fill disabled"
        }
        .to_owned();
        cx.notify();
    }

    fn replace_selected_annotation_style(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id).cloned() else {
            self.selected_annotation = None;
            return false;
        };
        let replacement = crate::domain::annotation::Annotation {
            style: self.annotation_style,
            ..existing.clone()
        };
        if replacement == existing {
            return false;
        }
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Replace(replacement))
        {
            Ok(()) => true,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                false
            }
        }
    }

    pub(in crate::app) fn select_selection_tool(&mut self, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
        self.text_edit = None;
        self.text_edit_annotation = None;
        self.annotation_tool = None;
        self.selected_annotation = None;
        self.status = "Selection tool selected".to_owned();
        cx.notify();
    }

    fn select_annotation_tool(&mut self, tool: AnnotationTool, cx: &mut Context<Self>) {
        self.annotation_editor.cancel();
        self.text_edit = None;
        self.text_edit_annotation = None;
        self.annotation_tool = Some(tool);
        self.selected_annotation = None;
        self.status = tool_selected_status(tool).to_owned();
        cx.notify();
    }

    pub(super) fn begin_annotation(&mut self, point: crate::domain::geometry::PhysicalPoint) {
        let (Some(document), Some(tool)) =
            (self.annotation_document.as_ref(), self.annotation_tool)
        else {
            return;
        };
        let id = AnnotationId::new(self.next_annotation_id);
        if matches!(tool, AnnotationTool::Text | AnnotationTool::Watermark) {
            self.annotation_editor.cancel();
            self.text_edit_annotation = None;
            self.text_edit = Some(if tool == AnnotationTool::Watermark {
                TextEdit::with_content(
                    point,
                    crate::domain::annotation::WATERMARK_CONTENT.to_owned(),
                    true,
                )
            } else {
                TextEdit::new(point)
            });
            self.status = if tool == AnnotationTool::Watermark {
                "Type watermark, then press Enter".to_owned()
            } else {
                "Type text, then press Enter".to_owned()
            };
            return;
        }
        let started = if tool == AnnotationTool::Number {
            self.annotation_editor.begin_number(
                document,
                id,
                style_for_tool(tool, self.annotation_style),
                point,
                self.next_sequence_number,
            )
        } else {
            self.annotation_editor.begin(
                document,
                id,
                tool,
                style_for_tool(tool, self.annotation_style),
                point,
            )
        };
        if started.is_ok() {
            self.next_annotation_id = self.next_annotation_id.saturating_add(1);
            self.status = drawing_status(tool).to_owned();
        }
    }

    pub(super) fn finish_annotation(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.annotation_document.as_mut() else {
            return;
        };
        let tool = self.annotation_tool;
        let moving = self.annotation_editor.moving().is_some();
        let resizing = self.annotation_editor.resizing().is_some();
        let committed = match self
            .annotation_editor
            .commit(document, &mut self.annotation_history)
        {
            Ok(true) if moving => {
                self.status = "Annotation moved".to_owned();
                false
            }
            Ok(true) if resizing => {
                self.status = "Annotation resized".to_owned();
                false
            }
            Ok(true) => {
                self.status = annotation_added_status(tool).to_owned();
                tool == Some(AnnotationTool::Number)
            }
            Ok(false) if moving => {
                self.status = "Annotation move cancelled".to_owned();
                false
            }
            Ok(false) if resizing => {
                self.status = "Annotation resize cancelled".to_owned();
                false
            }
            Ok(false) => {
                self.status = annotation_cancelled_status(tool).to_owned();
                false
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        };
        if committed {
            self.next_sequence_number = self.next_sequence_number.saturating_add(1);
        }
        cx.notify();
    }

    pub(in crate::app) fn commit_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.take() else {
            return false;
        };
        let target = self.text_edit_annotation.take();
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        if let Some(id) = target {
            let Some(existing) = document.annotation(id).cloned() else {
                self.status = "Text annotation no longer exists".to_owned();
                cx.notify();
                return true;
            };
            let Some(replacement) = text_annotation_with_content(existing, edit.content) else {
                self.status = "Selected annotation cannot be edited as text".to_owned();
                cx.notify();
                return true;
            };
            match self
                .annotation_history
                .apply(document, AnnotationCommand::Replace(replacement))
            {
                Ok(()) => self.status = "Text annotation updated".to_owned(),
                Err(error) => self.status = error.to_string(),
            }
            cx.notify();
            return true;
        }
        let id = AnnotationId::new(self.next_annotation_id);
        let started = if self.annotation_tool == Some(AnnotationTool::Watermark) {
            self.annotation_editor.begin_watermark(
                document,
                id,
                self.annotation_style,
                edit.origin,
                edit.content,
            )
        } else {
            self.annotation_editor.begin_text(
                document,
                id,
                self.annotation_style,
                edit.origin,
                edit.content,
            )
        };
        if let Err(error) = started {
            self.status = error.to_string();
            cx.notify();
            return true;
        }
        self.next_annotation_id = self.next_annotation_id.saturating_add(1);
        self.finish_annotation(cx);
        true
    }

    pub(in crate::app) fn cancel_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.text_edit.take().is_none() {
            return false;
        }
        self.text_edit_annotation = None;
        self.status = "Text cancelled".to_owned();
        cx.notify();
        true
    }

    pub(in crate::app) fn text_edit(&self) -> Option<&TextEdit> {
        self.text_edit.as_ref()
    }

    pub(in crate::app) fn text_edit_annotation(&self) -> Option<AnnotationId> {
        self.text_edit_annotation
    }

    pub(in crate::app) fn edit_selected_text_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(annotation) = self
            .annotation_document
            .as_ref()
            .and_then(|document| document.annotation(id))
        else {
            self.selected_annotation = None;
            return false;
        };
        let (origin, content) = match &annotation.kind {
            AnnotationKind::Text { origin, content }
            | AnnotationKind::Watermark { origin, content } => (*origin, content.clone()),
            _ => return false,
        };
        self.annotation_editor.cancel();
        self.annotation_tool = None;
        self.text_edit = Some(TextEdit::with_content(origin, content, true));
        self.text_edit_annotation = Some(id);
        self.status = "Edit text, then press Enter".to_owned();
        cx.notify();
        true
    }

    pub(in crate::app) fn replace_text_edit(
        &mut self,
        replacement_range_utf16: Option<Range<usize>>,
        text: &str,
        marked_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        let range = replacement_range_utf16
            .as_ref()
            .map(|range| range_from_utf16(&edit.content, range))
            .or(edit.marked_range.clone())
            .unwrap_or(edit.selected_range.clone());
        edit.content.replace_range(range.clone(), text);
        let cursor = range.start + text.len();
        edit.selected_range = marked_range_utf16
            .as_ref()
            .map(|range| range_from_utf16(text, range))
            .map(|selection| range.start + selection.start..range.start + selection.end)
            .unwrap_or(cursor..cursor);
        edit.marked_range = marked_range_utf16.map(|_| range.start..cursor);
        self.status = "Editing text...".to_owned();
        cx.notify();
        true
    }

    pub(in crate::app) fn unmark_text_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        edit.marked_range = None;
        cx.notify();
        true
    }

    pub(in crate::app) fn handle_text_edit_key(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.text_edit.is_none() || keystroke.modifiers.shift || keystroke.modifiers.control {
            return false;
        }
        match keystroke.key.as_str() {
            "enter" => self.commit_text_edit(cx),
            "escape" => self.cancel_text_edit(cx),
            "backspace" => self.delete_text_edit(true, cx),
            "delete" => self.delete_text_edit(false, cx),
            "left" => self.move_text_cursor(false, cx),
            "right" => self.move_text_cursor(true, cx),
            _ => false,
        }
    }

    fn delete_text_edit(&mut self, backwards: bool, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_ref() else {
            return false;
        };
        let range = if edit.selected_range.is_empty() {
            let cursor = edit.selected_range.end;
            if backwards {
                previous_char_boundary(&edit.content, cursor)..cursor
            } else {
                cursor..next_char_boundary(&edit.content, cursor)
            }
        } else {
            edit.selected_range.clone()
        };
        self.replace_text_edit(Some(range_to_utf16(&edit.content, &range)), "", None, cx)
    }

    fn move_text_cursor(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        let cursor = if forward {
            next_char_boundary(&edit.content, edit.selected_range.end)
        } else {
            previous_char_boundary(&edit.content, edit.selected_range.start)
        };
        edit.selected_range = cursor..cursor;
        edit.marked_range = None;
        cx.notify();
        true
    }

    pub(in crate::app) fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        // Printable keystrokes belong to the active native text editor, not
        // the annotation shortcut map.
        if self.text_edit.is_some() {
            return;
        }
        let Some(command) = keyboard_command(&event.keystroke) else {
            return;
        };
        let handled = match command {
            KeyboardCommand::Undo => self.undo_annotation(cx),
            KeyboardCommand::Redo => self.redo_annotation(cx),
            KeyboardCommand::Duplicate => self.duplicate_selected_annotation(cx),
            KeyboardCommand::BringForward => self.bring_selected_annotation_forward(cx),
            KeyboardCommand::SendBackward => self.send_selected_annotation_backward(cx),
            KeyboardCommand::RotateClockwise => self.rotate_selected_annotation_clockwise(cx),
            KeyboardCommand::SelectNextAnnotation => self.select_adjacent_annotation(false, cx),
            KeyboardCommand::SelectPreviousAnnotation => self.select_adjacent_annotation(true, cx),
            KeyboardCommand::Delete => self.delete_selected_annotation(cx),
            KeyboardCommand::Cancel => self.cancel_editor_or_capture(cx),
            KeyboardCommand::Copy => {
                if self.session.state() == CaptureSessionState::Selecting
                    && self.session.selection().is_some()
                {
                    self.copy_selection(cx);
                    true
                } else {
                    false
                }
            }
            KeyboardCommand::QuickSave => {
                if self.session.state() == CaptureSessionState::Selecting
                    && self.session.selection().is_some()
                {
                    self.quick_save_selection(cx);
                    true
                } else {
                    false
                }
            }
            KeyboardCommand::CopyColor => {
                if hovered_color(
                    self.frame.as_ref(),
                    self.hover_pixel,
                    ColorFormat::from_setting(self.settings.color_format),
                )
                .is_some()
                {
                    self.copy_hover_color(cx);
                    true
                } else {
                    false
                }
            }
            KeyboardCommand::MoveColorCursor(delta_x, delta_y) => {
                self.move_color_cursor(delta_x, delta_y, cx)
            }
            KeyboardCommand::Nudge(delta_x, delta_y) => {
                self.nudge_selected_annotation(delta_x, delta_y, cx)
                    || self.nudge_selection(delta_x, delta_y, cx)
            }
            KeyboardCommand::SelectTool(Some(tool)) => {
                self.select_annotation_tool(tool, cx);
                true
            }
            KeyboardCommand::SelectTool(None) => {
                self.select_selection_tool(cx);
                true
            }
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// Moves the color sampler by physical pixels while leaving the current selection unchanged.
    fn move_color_cursor(&mut self, delta_x: i32, delta_y: i32, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.frame.as_ref() else {
            return false;
        };
        let Some(point) = self.hover_pixel else {
            return false;
        };
        let target = clamp_physical_point(
            PhysicalPoint {
                x: point.x.saturating_add(delta_x),
                y: point.y.saturating_add(delta_y),
            },
            frame.bounds,
        );
        if target == point {
            return true;
        }
        if let Err(error) = crate::platform::cursor::move_to(target) {
            self.status = format!("Could not move color sampler: {error}");
            cx.notify();
            return true;
        }
        self.hover_pixel = Some(target);
        self.update_status_for_hover();
        cx.notify();
        true
    }

    fn cancel_editor_or_capture(&mut self, cx: &mut Context<Self>) -> bool {
        if self.cancel_text_edit(cx) {
            return true;
        }
        if self.annotation_editor.cancel() {
            self.status = "Annotation edit cancelled".to_owned();
            cx.notify();
            return true;
        }
        if self.selected_annotation.take().is_some() {
            self.status = "Annotation deselected".to_owned();
            cx.notify();
            return true;
        }
        if matches!(
            self.session.state(),
            CaptureSessionState::Capturing
                | CaptureSessionState::Selecting
                | CaptureSessionState::Completed
                | CaptureSessionState::Failed
        ) {
            self.reset(cx);
            return true;
        }
        false
    }

    pub(in crate::app) fn undo_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self.annotation_history.undo(document) {
            Ok(true) => {
                self.status = "Annotation undone".to_owned();
                cx.notify();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn redo_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self.annotation_history.redo(document) {
            Ok(true) => {
                self.status = "Annotation redone".to_owned();
                cx.notify();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn delete_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        self.annotation_editor.cancel();
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Delete(id))
        {
            Ok(()) => {
                self.selected_annotation = None;
                self.status = "Annotation deleted".to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn duplicate_selected_annotation(&mut self, cx: &mut Context<Self>) -> bool {
        const DUPLICATE_OFFSET_PIXELS: i32 = 12;

        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id) else {
            self.selected_annotation = None;
            return false;
        };
        let duplicate_id = AnnotationId::new(self.next_annotation_id);
        let duplicate = existing.duplicated(
            duplicate_id,
            document.canvas_bounds(),
            DUPLICATE_OFFSET_PIXELS,
        );
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Insert(duplicate))
        {
            Ok(()) => {
                self.next_annotation_id = self.next_annotation_id.saturating_add(1);
                self.selected_annotation = Some(duplicate_id);
                self.status = "Annotation duplicated".to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn rotate_selected_annotation_clockwise(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id).cloned() else {
            self.selected_annotation = None;
            return false;
        };
        let Some(rotated) = existing.rotated_clockwise_within(document.canvas_bounds()) else {
            self.status = "Rotation is not supported for text or number annotations".to_owned();
            cx.notify();
            return true;
        };
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Replace(rotated))
        {
            Ok(()) => {
                self.status = "Annotation rotated clockwise".to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(in crate::app) fn bring_selected_annotation_to_front(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        self.reorder_selected_annotation(usize::MAX, "Annotation brought to front", cx)
    }

    pub(in crate::app) fn select_annotation_layer(
        &mut self,
        id: AnnotationId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.annotation_document.as_ref() else {
            return false;
        };
        let Some(annotation) = document.annotation(id) else {
            return false;
        };
        let position = document
            .annotations()
            .iter()
            .position(|candidate| candidate.id == id)
            .map_or(0, |index| index + 1);
        self.annotation_editor.cancel();
        self.annotation_tool = None;
        self.selected_annotation = Some(id);
        self.annotation_style = annotation.style;
        self.status = format!(
            "Selected annotation {position} of {}",
            document.annotations().len()
        );
        cx.notify();
        true
    }

    fn select_adjacent_annotation(&mut self, reverse: bool, cx: &mut Context<Self>) -> bool {
        let Some(document) = self.annotation_document.as_ref() else {
            return false;
        };
        let ids = document
            .annotations()
            .iter()
            .map(|annotation| annotation.id)
            .collect::<Vec<_>>();
        let Some(id) = next_annotation_selection(&ids, self.selected_annotation, reverse) else {
            return false;
        };
        let Some(annotation) = document.annotation(id) else {
            return false;
        };
        self.annotation_editor.cancel();
        self.annotation_tool = None;
        self.selected_annotation = Some(id);
        self.annotation_style = annotation.style;
        self.status = format!(
            "Selected annotation {} of {}",
            annotation_position(&ids, id),
            ids.len()
        );
        cx.notify();
        true
    }

    pub(in crate::app) fn send_selected_annotation_to_back(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        self.reorder_selected_annotation(0, "Annotation sent to back", cx)
    }

    pub(in crate::app) fn bring_selected_annotation_forward(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self.selected_annotation_index() else {
            return false;
        };
        self.reorder_selected_annotation(index.saturating_add(1), "Annotation brought forward", cx)
    }

    pub(in crate::app) fn send_selected_annotation_backward(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self.selected_annotation_index() else {
            return false;
        };
        self.reorder_selected_annotation(index.saturating_sub(1), "Annotation sent backward", cx)
    }

    fn selected_annotation_index(&self) -> Option<usize> {
        let id = self.selected_annotation?;
        self.annotation_document
            .as_ref()?
            .annotations()
            .iter()
            .position(|annotation| annotation.id == id)
    }

    fn reorder_selected_annotation(
        &mut self,
        index: usize,
        status: &'static str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let target = index.min(document.annotations().len().saturating_sub(1));
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Reorder { id, index: target })
        {
            Ok(()) => {
                self.status = status.to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    fn nudge_selection(&mut self, delta_x: i32, delta_y: i32, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.frame.as_ref() else {
            return false;
        };
        let Some(selection) = self.selection_drag.nudge(frame.bounds, delta_x, delta_y) else {
            return false;
        };
        if self.session.select(selection).is_ok() {
            self.status = selection_status(selection);
            cx.notify();
            true
        } else {
            false
        }
    }

    fn nudge_selected_annotation(
        &mut self,
        delta_x: i32,
        delta_y: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.selected_annotation else {
            return false;
        };
        let Some(document) = self.annotation_document.as_mut() else {
            return false;
        };
        let Some(existing) = document.annotation(id).cloned() else {
            self.selected_annotation = None;
            return false;
        };
        let replacement = existing.translated_within(document.canvas_bounds(), delta_x, delta_y);
        if replacement == existing {
            return true;
        }
        match self
            .annotation_history
            .apply(document, AnnotationCommand::Replace(replacement))
        {
            Ok(()) => {
                self.status = "Annotation moved".to_owned();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
                true
            }
        }
    }

    pub(super) fn update_status_for_hover(&mut self) {
        if let Some((point, color)) = self.hover_pixel.and_then(|point| {
            self.frame
                .as_ref()?
                .pixel_at(point)
                .map(|color| (point, color))
        }) {
            self.status = if let Some(selection) = self.selection_drag.selection() {
                format!(
                    "{} x {} px | ({}, {}) {}",
                    selection.width(),
                    selection.height(),
                    point.x,
                    point.y,
                    color.hex_rgb()
                )
            } else if let Some(target) = self
                .inspection_target
                .filter(|target| target.bounds.contains(point))
            {
                smart_target_status(target, point, color.hex_rgb())
            } else {
                format!("({}, {}) {}", point.x, point.y, color.hex_rgb())
            };
        } else if let Some(selection) = self.selection_drag.selection() {
            self.status = selection_status(selection);
        } else if let Some(frame) = self.frame.as_ref() {
            self.status = format!("{} x {} physical pixels", frame.width, frame.height);
        }
    }

    pub(super) fn request_inspection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        cx: &mut Context<Self>,
    ) {
        self.inspection_request = Some(point);
        if self.inspection_in_flight {
            return;
        }
        self.start_inspection(cx);
    }

    fn start_inspection(&mut self, cx: &mut Context<Self>) {
        let Some(point) = self.inspection_request.take() else {
            return;
        };
        self.inspection_in_flight = true;
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move { SystemWindowInspector.target_at(point) })
                    .await;
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_inspection(point, result, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn finish_inspection(
        &mut self,
        point: crate::domain::geometry::PhysicalPoint,
        result: std::io::Result<Option<InspectionTarget>>,
        cx: &mut Context<Self>,
    ) {
        self.inspection_in_flight = false;
        match result {
            Ok(target) if self.hover_pixel == Some(point) => {
                self.inspection_target = target.and_then(|target| {
                    let bounds = intersect_rect(target.bounds, self.frame.as_ref()?.bounds)?;
                    Some(InspectionTarget {
                        bounds,
                        kind: target.kind,
                    })
                });
                self.update_status_for_hover();
                cx.notify();
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(target: "flash_shot::inspection", "window_inspection_failed error={error}");
            }
        }
        if self.inspection_request.is_some() {
            self.start_inspection(cx);
        }
    }
}
