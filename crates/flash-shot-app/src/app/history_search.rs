//! Native text-input state for filtering retained screenshots without affecting annotation text.

use std::ops::Range;

use gpui::{Context, Keystroke};

use super::{FlashShotApp, SettingsSection};
use crate::i18n::UiText;

/// Identifies guarded history shortcuts before they reach the existing batch-action flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryKeyboardCommand {
    SelectAllFiltered,
    Escape,
    DeleteSelection,
    MoveFocus { forward: bool },
    FocusBoundary { last: bool },
    ToggleFocused,
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
            HistoryKeyboardCommand::MoveFocus { forward } => self.move_history_focus(forward, cx),
            HistoryKeyboardCommand::FocusBoundary { last } => self.focus_history_boundary(last, cx),
            HistoryKeyboardCommand::ToggleFocused => self.toggle_focused_history(cx),
        }
        true
    }

    /// Moves the keyboard focus through the current filtered history without changing selection.
    fn move_history_focus(&mut self, forward: bool, cx: &mut Context<Self>) {
        let paths = self.filtered_history_paths();
        self.history_keyboard_focus =
            history_focus_index(&paths, self.history_keyboard_focus.as_ref(), forward)
                .and_then(|index| paths.get(index).cloned());
        if self.history_keyboard_focus.is_some() {
            // Keyboard navigation may move beyond the five-row preview, so reveal the full list
            // only after the user explicitly starts navigating it.
            self.history_expanded = true;
        }
        cx.notify();
    }

    /// Jumps to the first or last filtered capture so long lists remain keyboard reachable.
    fn focus_history_boundary(&mut self, last: bool, cx: &mut Context<Self>) {
        let paths = self.filtered_history_paths();
        self.history_keyboard_focus = if last {
            paths.last().cloned()
        } else {
            paths.first().cloned()
        };
        if self.history_keyboard_focus.is_some() {
            self.history_expanded = true;
        }
        cx.notify();
    }

    /// Toggles the focused capture, choosing the first filtered row when focus is not set yet.
    fn toggle_focused_history(&mut self, cx: &mut Context<Self>) {
        let paths = self.filtered_history_paths();
        let Some(index) = history_focus_index(&paths, self.history_keyboard_focus.as_ref(), true)
        else {
            self.status = self
                .settings
                .locale
                .text(UiText::HistoryNoMatches)
                .to_owned();
            cx.notify();
            return;
        };
        let path = paths[index].clone();
        self.history_keyboard_focus = Some(path.clone());
        self.history_expanded = true;
        self.toggle_history_selection(path, cx);
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
        "down" => Some(HistoryKeyboardCommand::MoveFocus { forward: true }),
        "up" => Some(HistoryKeyboardCommand::MoveFocus { forward: false }),
        "home" => Some(HistoryKeyboardCommand::FocusBoundary { last: false }),
        "end" => Some(HistoryKeyboardCommand::FocusBoundary { last: true }),
        "space" => Some(HistoryKeyboardCommand::ToggleFocused),
        _ => None,
    }
}

/// Returns the next filtered row index, wrapping at either end for uninterrupted keyboard use.
fn history_focus_index(
    paths: &[std::path::PathBuf],
    focused: Option<&std::path::PathBuf>,
    forward: bool,
) -> Option<usize> {
    if paths.is_empty() {
        return None;
    }
    let current = focused.and_then(|path| paths.iter().position(|candidate| candidate == path));
    Some(match current {
        Some(index) if forward => (index + 1) % paths.len(),
        Some(0) => paths.len() - 1,
        Some(index) => index - 1,
        None if forward => 0,
        None => paths.len() - 1,
    })
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
    use super::{HistoryKeyboardCommand, history_focus_index, history_keyboard_command};
    use std::path::PathBuf;

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
            history_keyboard_command(&key("down", Default::default())),
            Some(HistoryKeyboardCommand::MoveFocus { forward: true })
        );
        assert_eq!(
            history_keyboard_command(&key("up", Default::default())),
            Some(HistoryKeyboardCommand::MoveFocus { forward: false })
        );
        assert_eq!(
            history_keyboard_command(&key("home", Default::default())),
            Some(HistoryKeyboardCommand::FocusBoundary { last: false })
        );
        assert_eq!(
            history_keyboard_command(&key("end", Default::default())),
            Some(HistoryKeyboardCommand::FocusBoundary { last: true })
        );
        assert_eq!(
            history_keyboard_command(&key("space", Default::default())),
            Some(HistoryKeyboardCommand::ToggleFocused)
        );
        assert_eq!(
            history_keyboard_command(&key(
                "space",
                gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                }
            )),
            None
        );
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

    #[test]
    fn history_focus_wraps_and_recovers_when_the_current_row_is_filtered_out() {
        let paths = vec![
            PathBuf::from("F:/captures/first.png"),
            PathBuf::from("F:/captures/second.png"),
            PathBuf::from("F:/captures/third.png"),
        ];

        assert_eq!(history_focus_index(&paths, None, true), Some(0));
        assert_eq!(history_focus_index(&paths, None, false), Some(2));
        assert_eq!(history_focus_index(&paths, Some(&paths[0]), false), Some(2));
        assert_eq!(history_focus_index(&paths, Some(&paths[2]), true), Some(0));
        assert_eq!(
            history_focus_index(
                &paths,
                Some(&PathBuf::from("F:/captures/missing.png")),
                true
            ),
            Some(0)
        );
        assert_eq!(history_focus_index(&[], None, true), None);
    }
}
