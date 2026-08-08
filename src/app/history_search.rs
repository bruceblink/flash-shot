//! Native text-input state for filtering retained screenshots without affecting annotation text.

use std::ops::Range;

use gpui::{Context, Keystroke};

use super::{FlashShotApp, SettingsSection};

/// Identifies guarded history shortcuts before they reach the existing batch-action flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryKeyboardCommand {
    SelectAllFiltered,
    Escape,
    DeleteSelection,
}

impl FlashShotApp {
    pub(super) fn history_search_query(&self) -> &str {
        &self.history_search.content
    }

    pub(super) fn history_search_is_active(&self) -> bool {
        self.history_search.active
    }

    pub(super) fn activate_history_search(&mut self, cx: &mut Context<Self>) {
        self.history_search.active = true;
        let cursor = self.history_search.content.len();
        self.history_search.selected_range = cursor..cursor;
        self.history_search.marked_range = None;
        cx.notify();
    }

    pub(super) fn clear_history_search(&mut self, cx: &mut Context<Self>) {
        if self.history_search.content.is_empty() {
            return;
        }
        self.history_search.content.clear();
        self.history_search.selected_range = 0..0;
        self.history_search.marked_range = None;
        self.history_expanded = false;
        cx.notify();
    }

    /// Routes history keys while keeping the active search box's normal text-editing behavior first.
    /// Delete intentionally reuses the confirmation flow, so a shortcut cannot remove saved files directly.
    pub(super) fn handle_history_key(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.handle_history_search_key(keystroke, cx) {
            return true;
        }
        if self.settings_section != SettingsSection::Files {
            return false;
        }
        let Some(command) = history_keyboard_command(keystroke) else {
            return false;
        };
        match command {
            HistoryKeyboardCommand::SelectAllFiltered => self.select_filtered_history(cx),
            HistoryKeyboardCommand::Escape => {
                // Escape abandons a destructive confirmation before it changes the saved selection.
                if self.history_clear_confirmation {
                    self.cancel_history_clear(cx);
                } else {
                    self.clear_history_selection(cx);
                }
            }
            HistoryKeyboardCommand::DeleteSelection => self.request_selected_history_clear(cx),
        }
        true
    }

    pub(super) fn handle_history_search_key(
        &mut self,
        keystroke: &Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.history_search.active {
            return false;
        }
        if keystroke.modifiers.secondary() && keystroke.key == "a" {
            self.history_search.selected_range = 0..self.history_search.content.len();
            self.history_search.marked_range = None;
            cx.notify();
            return true;
        }
        if keystroke.modifiers.secondary() && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.replace_history_search(None, &text.replace(['\r', '\n'], " "), None, cx);
            }
            return true;
        }
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.function {
            return false;
        }
        match keystroke.key.as_str() {
            "escape" | "enter" => {
                self.history_search.active = false;
                self.history_search.marked_range = None;
                cx.notify();
                true
            }
            "backspace" => self.delete_history_search(true, cx),
            "delete" => self.delete_history_search(false, cx),
            "left" => self.move_history_search_cursor(false, cx),
            "right" => self.move_history_search_cursor(true, cx),
            "home" => {
                self.history_search.selected_range = 0..0;
                self.history_search.marked_range = None;
                cx.notify();
                true
            }
            "end" => {
                let end = self.history_search.content.len();
                self.history_search.selected_range = end..end;
                self.history_search.marked_range = None;
                cx.notify();
                true
            }
            _ => false,
        }
    }

    pub(super) fn replace_history_search(
        &mut self,
        replacement_range_utf16: Option<Range<usize>>,
        text: &str,
        marked_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.history_search.active {
            return false;
        }
        let range = replacement_range_utf16
            .as_ref()
            .map(|range| super::utf16_range_to_byte_range(&self.history_search.content, range))
            .or(self.history_search.marked_range.clone())
            .unwrap_or(self.history_search.selected_range.clone());
        self.history_search
            .content
            .replace_range(range.clone(), text);
        let cursor = range.start + text.len();
        self.history_search.selected_range = marked_range_utf16
            .as_ref()
            .map(|range| super::utf16_range_to_byte_range(text, range))
            .map(|selection| range.start + selection.start..range.start + selection.end)
            .unwrap_or(cursor..cursor);
        self.history_search.marked_range = marked_range_utf16.map(|_| range.start..cursor);
        self.history_expanded = false;
        cx.notify();
        true
    }

    pub(super) fn unmark_history_search(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.history_search.active {
            return false;
        }
        self.history_search.marked_range = None;
        cx.notify();
        true
    }

    fn delete_history_search(&mut self, backwards: bool, cx: &mut Context<Self>) -> bool {
        let range = if self.history_search.selected_range.is_empty() {
            let cursor = self.history_search.selected_range.end;
            if backwards {
                previous_char_boundary(&self.history_search.content, cursor)..cursor
            } else {
                cursor..next_char_boundary(&self.history_search.content, cursor)
            }
        } else {
            self.history_search.selected_range.clone()
        };
        if range.is_empty() {
            return true;
        }
        let range_utf16 = super::byte_range_to_utf16_range(&self.history_search.content, &range);
        self.replace_history_search(Some(range_utf16), "", None, cx)
    }

    fn move_history_search_cursor(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let cursor = if forward {
            next_char_boundary(
                &self.history_search.content,
                self.history_search.selected_range.end,
            )
        } else {
            previous_char_boundary(
                &self.history_search.content,
                self.history_search.selected_range.start,
            )
        };
        self.history_search.selected_range = cursor..cursor;
        self.history_search.marked_range = None;
        cx.notify();
        true
    }
}

/// Maps only familiar, unambiguous shortcuts to batch history actions.
fn history_keyboard_command(keystroke: &Keystroke) -> Option<HistoryKeyboardCommand> {
    let modifiers = keystroke.modifiers;
    if modifiers.secondary() && modifiers.number_of_modifiers() == 1 && keystroke.key == "a" {
        return Some(HistoryKeyboardCommand::SelectAllFiltered);
    }
    if modifiers.modified() {
        return None;
    }
    match keystroke.key.as_str() {
        "escape" => Some(HistoryKeyboardCommand::Escape),
        "delete" => Some(HistoryKeyboardCommand::DeleteSelection),
        _ => None,
    }
}

fn previous_char_boundary(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(text: &str, offset: usize) -> usize {
    text[offset..]
        .char_indices()
        .nth(1)
        .map(|(index, _)| offset + index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::{HistoryKeyboardCommand, history_keyboard_command};

    fn key(key: &str, modifiers: gpui::Modifiers) -> gpui::Keystroke {
        gpui::Keystroke {
            key: key.into(),
            modifiers,
            key_char: None,
        }
    }

    #[test]
    fn history_batch_shortcuts_require_unambiguous_keys() {
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(
            history_keyboard_command(&key("a", control)),
            Some(HistoryKeyboardCommand::SelectAllFiltered)
        );
        assert_eq!(
            history_keyboard_command(&key("escape", Default::default())),
            Some(HistoryKeyboardCommand::Escape)
        );
        assert_eq!(
            history_keyboard_command(&key("delete", Default::default())),
            Some(HistoryKeyboardCommand::DeleteSelection)
        );
        assert_eq!(
            history_keyboard_command(&key("a", Default::default())),
            None
        );
        assert_eq!(
            history_keyboard_command(&key(
                "a",
                gpui::Modifiers {
                    control: true,
                    shift: true,
                    ..Default::default()
                }
            )),
            None
        );
        assert_eq!(history_keyboard_command(&key("delete", control)), None);
        assert_eq!(
            history_keyboard_command(&key(
                "a",
                gpui::Modifiers {
                    control: true,
                    platform: true,
                    ..Default::default()
                }
            )),
            None
        );
    }
}
