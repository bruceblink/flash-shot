//! Capture overlay input event orchestration and focus restoration.

use super::*;

impl CaptureOverlay {
    pub(in crate::app) fn new(
        app: Entity<FlashShotApp>,
        display: DisplayInfo,
        preview: Arc<RenderImage>,
        operation_generation: u64,
        allow_cross_display_drag: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app,
            display,
            preview,
            allow_cross_display_drag,
            selection_pointer_active: false,
            operation_generation,
            selection_updates: FrameInputBatch::default(),
            focus_handle: cx.focus_handle(),
            more_actions_focus_handle: cx.focus_handle().tab_stop(false),
            secondary_action_focus_handles: std::array::from_fn(|index| {
                cx.focus_handle().tab_stop(true).tab_index(index as isize)
            }),
            annotation_tool_group_trigger_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            annotation_tool_group_item_focus_handles: std::array::from_fn(|_| {
                std::array::from_fn(|_| cx.focus_handle())
            }),
            toolbar_drag: None,
            manual_toolbar_position: None,
            last_workspace_snapshot: None,
            last_workspace_scale_factor: 1.0,
            topmost_requested: false,
            annotation_arrange_actions_for: None,
            _app_observation: observation,
        }
    }

    /// Clears the manual toolbar position before a fresh selection gesture begins.
    pub(super) fn reset_toolbar_position(&mut self) {
        self.toolbar_drag = None;
        self.manual_toolbar_position = None;
    }

    /// Exposes the rendered toolbar bounds to the native acceptance bridge.
    ///
    /// Layout values are local to this borderless display-sized window, so translate them by the
    /// display origin before returning physical desktop coordinates.
    pub(in crate::app) fn action_toolbar_bounds_for_acceptance(&self) -> Option<PhysicalRect> {
        let toolbar = self.last_workspace_snapshot?.action_toolbar?;
        let origin = self.display.physical_bounds;
        let scale = self.last_workspace_scale_factor.max(0.01);
        Some(PhysicalRect {
            left: origin.left + (toolbar.left * scale).round() as i32,
            top: origin.top + (toolbar.top * scale).round() as i32,
            right: origin.left + ((toolbar.left + toolbar.width) * scale).round() as i32,
            bottom: origin.top + ((toolbar.top + toolbar.height) * scale).round() as i32,
        })
    }

    /// Exposes client-relative More panel bounds so native acceptance can add the HWND origin.
    pub(in crate::app) fn secondary_menu_bounds_for_acceptance(&self) -> Option<PhysicalRect> {
        let snapshot = self.last_workspace_snapshot?;
        let toolbar = snapshot.action_toolbar?;
        let menu = snapshot.secondary_menu?;
        let scale = self.last_workspace_scale_factor.max(0.01);
        let left = toolbar.left + menu.left;
        let top = toolbar.top + menu.top_offset;
        Some(PhysicalRect {
            left: (left * scale).round() as i32,
            top: (top * scale).round() as i32,
            right: ((left + menu.width) * scale).round() as i32,
            bottom: ((top + menu.height) * scale).round() as i32,
        })
    }

    /// Starts a toolbar move without allowing the full-screen selection surface to see the click.
    pub(super) fn begin_toolbar_drag(
        &mut self,
        event: &MouseDownEvent,
        toolbar: Option<ActionToolbarLayout>,
        selection: Option<PhysicalRect>,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let (Some(toolbar), Some(selection)) = (toolbar, selection) else {
            return;
        };
        self.toolbar_drag = Some(ToolbarDragGesture {
            pointer_start: view_point(event.position),
            toolbar_start: toolbar,
            selection,
        });
        self.manual_toolbar_position = Some(ManualToolbarPosition {
            selection,
            left: toolbar.left,
            top: toolbar.top,
        });
        cx.stop_propagation();
        cx.notify();
    }

    /// Applies one movement sample while the toolbar owns the pointer gesture.
    pub(super) fn update_toolbar_drag(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            self.toolbar_drag = None;
            return;
        }
        let Some(gesture) = self.toolbar_drag else {
            return;
        };
        self.manual_toolbar_position = Some(toolbar_position_for_pointer(
            gesture,
            view_point(event.position),
            viewport,
        ));
        cx.stop_propagation();
        cx.notify();
    }

    /// Commits the final pointer sample and releases the toolbar gesture.
    pub(super) fn finish_toolbar_drag(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.toolbar_drag.is_none() {
            return;
        }
        if let Some(gesture) = self.toolbar_drag {
            self.manual_toolbar_position = Some(toolbar_position_for_pointer(
                gesture,
                view_point(event.position),
                viewport,
            ));
        }
        self.toolbar_drag = None;
        cx.stop_propagation();
        cx.notify();
    }

    /// Builds one inline action from the shared catalog while keeping existing workflow handlers.
    pub(super) fn inline_secondary_action_button(
        &self,
        action: WorkspaceInlineAction,
        locale: Locale,
        colors: ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = locale.text(match action {
            WorkspaceInlineAction::Scroll => UiText::OverlayScrollShot,
            WorkspaceInlineAction::Qr => UiText::OverlayQr,
            WorkspaceInlineAction::Ocr => UiText::OverlayOcr,
            WorkspaceInlineAction::Translate => UiText::OverlayTranslate,
            WorkspaceInlineAction::RecordArea => UiText::OverlayRecordArea,
            WorkspaceInlineAction::RecordWindow => UiText::OverlayRecordWindow,
        });
        let config = WorkspaceButtonConfig::icon(
            colors,
            WorkspaceButtonTone::Neutral,
            false,
            true,
            secondary_action_tooltip(locale, action.tooltip_key()),
        );
        let button = match action {
            WorkspaceInlineAction::Scroll => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.start_manual_scroll(cx)));
                }),
            ),
            WorkspaceInlineAction::Qr => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.recognize_qr_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::Ocr => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.recognize_text_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::Translate => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.translate_selection(cx)));
                }),
            ),
            WorkspaceInlineAction::RecordArea => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| app.update(cx, |app, cx| app.start_region_recording(cx)));
                }),
            ),
            WorkspaceInlineAction::RecordWindow => workspace_icon_button(
                action.id(),
                action.icon(),
                label,
                config,
                cx.listener(|this, _, _, cx| {
                    let app = this.app.clone();
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| app.start_selected_window_recording(cx))
                    });
                }),
            ),
        };
        button.on_key_down(stop_overlay_action_key_propagation)
    }

    pub(super) fn transform(&self, viewport: Bounds<Pixels>) -> Option<PreviewTransform> {
        PreviewTransform::contain(self.display.physical_bounds, view_rect(viewport))
    }

    pub(super) fn begin_selection(
        &mut self,
        event: &MouseDownEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            return;
        }
        self.reset_toolbar_position();
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let screen_pointer = self
            .allow_cross_display_drag
            .then(|| cursor::position().ok())
            .flatten();
        let Some(point) = self.transform(viewport).and_then(|transform| {
            selection_point_from_view_or_screen(
                transform,
                event.position,
                self.allow_cross_display_drag,
                screen_pointer,
                frame_bounds,
            )
        }) else {
            return;
        };
        self.selection_pointer_active = true;
        // A new gesture must not consume a hover or drag sample that belonged to the prior one.
        self.selection_updates.invalidate();
        let resize_handle = self
            .app
            .read(cx)
            .selection_drag
            .selection()
            .and_then(|selection| {
                self.transform(viewport)?.resize_handle_at(
                    selection,
                    view_point(event.position),
                    10.0,
                )
            });
        let annotation_resize_handle = self.app.read(cx).selected_annotation.and_then(|id| {
            let annotation = self
                .app
                .read(cx)
                .annotation_document
                .as_ref()?
                .annotation(id)?;
            self.transform(viewport)?.resize_handle_at(
                annotation.bounds(),
                view_point(event.position),
                10.0,
            )
        });
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                app.begin_overlay_selection(point, resize_handle, annotation_resize_handle);
                cx.notify();
            })
        });
    }

    pub(super) fn update_selection(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            self.selection_updates.invalidate();
            return;
        }
        let Some(transform) = self.transform(viewport) else {
            return;
        };
        let view = view_point(event.position);
        let point = transform.view_to_pixel(view);
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let dragging_point = event
            .dragging()
            .then(|| {
                let screen_pointer = self
                    .allow_cross_display_drag
                    .then(|| cursor::position().ok())
                    .flatten();
                selection_point_from_view_or_screen(
                    transform,
                    event.position,
                    self.allow_cross_display_drag,
                    screen_pointer,
                    frame_bounds,
                )
            })
            .flatten();
        let update = PendingSelectionUpdate {
            operation_generation,
            hover_point: point,
            dragging_point,
            preserve_aspect_ratio,
            resize_from_center,
        };
        let Some(frame_generation) = self.selection_updates.push(update) else {
            return;
        };
        // This is the only deferred callback for all moves received before the next paint. The
        // callback applies the most recent sample, not the first one that happened to arrive.
        cx.on_next_frame(window, move |this, _, cx| {
            if let Some(update) = this.selection_updates.take(frame_generation) {
                this.apply_selection_update(update, cx);
            }
        });
        // `on_next_frame` needs a paint to occur. GPUI deduplicates same-entity notifications in
        // this event cycle, so repeated raw moves keep one queued callback and one invalidation.
        cx.notify();
    }

    /// Applies one frame's newest pointer sample to the shared capture model.
    ///
    /// The overlay owns scheduling while `FlashShotApp` owns the actual selection and hover state;
    /// checking both generations keeps a closed overlay from changing a later capture session.
    pub(super) fn apply_selection_update(
        &mut self,
        update: PendingSelectionUpdate,
        cx: &mut Context<Self>,
    ) {
        if !accepts_overlay_input(
            update.operation_generation,
            self.app.read(cx).operation_generation,
        ) {
            return;
        }
        let app = self.app.clone();
        app.update(cx, |app, cx| {
            if !accepts_overlay_input(update.operation_generation, app.operation_generation) {
                return;
            }
            app.update_overlay_hover(update.hover_point, cx);
            if let Some(point) = update.dragging_point {
                app.update_overlay_selection(
                    point,
                    update.preserve_aspect_ratio,
                    update.resize_from_center,
                    cx,
                );
            }
        });
    }

    pub(super) fn finish_selection(
        &mut self,
        event: &MouseUpEvent,
        viewport: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.selection_pointer_active = false;
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            self.selection_updates.invalidate();
            return;
        }
        // The final mouse position wins over any queued frame update from this gesture, so it
        // cannot overwrite the selection after the button is released.
        self.selection_updates.invalidate();
        let frame_bounds = self
            .app
            .read(cx)
            .frame
            .as_ref()
            .map(|frame| frame.bounds)
            .unwrap_or(self.display.physical_bounds);
        let point = self.transform(viewport).and_then(|transform| {
            let screen_pointer = self
                .allow_cross_display_drag
                .then(|| cursor::position().ok())
                .flatten();
            selection_point_from_view_or_screen(
                transform,
                event.position,
                self.allow_cross_display_drag,
                screen_pointer,
                frame_bounds,
            )
        });
        let Some(point) = point else { return };
        let app = self.app.clone();
        let preserve_aspect_ratio = event.modifiers.shift;
        let resize_from_center = event.modifiers.alt;
        let copy_on_double_click = capture_double_click(event.click_count);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                app.finish_overlay_selection(
                    point,
                    preserve_aspect_ratio,
                    resize_from_center,
                    copy_on_double_click,
                    cx,
                )
            })
        });
    }

    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        if !accepts_overlay_input(operation_generation, self.app.read(cx).operation_generation) {
            return;
        }
        // Copy cancellation has priority over More and text-editor Escape handling. The copied
        // pixels are already frozen, so this leaves the user in their current editing context.
        if event.keystroke.key == "escape"
            && !event.keystroke.modifiers.modified()
            && self.app.read(cx).selection_copy_owns_escape()
        {
            self.app.update(cx, |app, cx| app.cancel_selection_copy(cx));
            cx.stop_propagation();
            return;
        }
        if smart_target_toggle_shortcut(&event.keystroke)
            && self.app.update(cx, |app, cx| app.toggle_smart_target(cx))
        {
            cx.stop_propagation();
            return;
        }
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_more_actions_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if more_actions_shortcut(&event.keystroke)
            && self.open_more_actions_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        let app = self.app.clone();
        let event = event.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation) {
                    return;
                }
                if !app.handle_text_edit_key(&event.keystroke, cx) {
                    app.handle_key_down(&event, cx);
                }
            })
        });
    }

    pub(super) fn annotation_tool_group_trigger_focus_handle(
        &self,
        group: AnnotationToolGroup,
    ) -> FocusHandle {
        self.annotation_tool_group_trigger_focus_handles[group.index()].clone()
    }

    pub(super) fn annotation_tool_group_item_focus_handle(
        &self,
        group: AnnotationToolGroup,
        index: usize,
    ) -> FocusHandle {
        self.annotation_tool_group_item_focus_handles[group.index()][index].clone()
    }

    /// Opens or closes a tool group and moves focus after the next render, so the new child exists.
    pub(super) fn toggle_annotation_tool_group_from_trigger(
        &mut self,
        group: AnnotationToolGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        let owner = self.display.id.clone();
        let app = self.app.clone();
        let is_open = {
            let app = app.read(cx);
            app.annotation_tool_group == Some(group)
                && app.annotation_tool_group_owner.as_deref() == Some(owner.as_str())
        };
        let focus_handle = if is_open {
            self.annotation_tool_group_trigger_focus_handle(group)
        } else {
            self.annotation_tool_group_item_focus_handle(group, 0)
        };
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.toggle_annotation_tool_group(&owner, group, cx);
                }
            });
        });
        cx.on_next_frame(window, move |_, window, cx| focus_handle.focus(window, cx));
    }

    /// Selects one materialized child exactly once, closes its owner, and returns focus to the
    /// trigger so the next keyboard action stays in the screenshot workspace.
    pub(super) fn select_annotation_tool_from_group(
        &mut self,
        group: AnnotationToolGroup,
        tool: AnnotationTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation_generation = self.operation_generation;
        let owner = self.display.id.clone();
        let app = self.app.clone();
        let trigger_focus = self.annotation_tool_group_trigger_focus_handle(group);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if !accepts_overlay_input(operation_generation, app.operation_generation)
                    || app.annotation_tool_group != Some(group)
                    || app.annotation_tool_group_owner.as_deref() != Some(owner.as_str())
                {
                    return;
                }
                app.select_annotation_tool(tool, cx);
                app.close_annotation_tool_group();
            });
        });
        cx.on_next_frame(window, move |_, window, cx| trigger_focus.focus(window, cx));
    }

    /// Closes the one group owned by this overlay and restores trigger focus for Escape handling.
    pub(super) fn close_annotation_tool_group_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (group, owner) = {
            let app = self.app.read(cx);
            (
                app.annotation_tool_group,
                app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str()),
            )
        };
        let Some(group) = group.filter(|_| owner) else {
            return false;
        };
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        let trigger_focus = self.annotation_tool_group_trigger_focus_handle(group);
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation)
                    && app.annotation_tool_group == Some(group)
                {
                    app.close_annotation_tool_group();
                    cx.notify();
                }
            });
        });
        cx.on_next_frame(window, move |_, window, cx| trigger_focus.focus(window, cx));
        true
    }

    /// Handles a child button's local arrows and Escape without leaking them to canvas shortcuts.
    pub(super) fn handle_annotation_tool_group_item_key_down(
        &mut self,
        group: AnnotationToolGroup,
        current: AnnotationTool,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if close_more_actions_shortcut(&event.keystroke)
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        let navigation = AnnotationToolGroupNavigation {
            tools: WorkspaceAnnotationToolSpec::group_tools(group),
            current,
            focus_handles: self.annotation_tool_group_item_focus_handles[group.index()].clone(),
        };
        navigation.handle_key_down(event, window, cx);
    }

    /// Lets a focused group trigger enter its children with arrows while keeping activation local.
    pub(super) fn handle_annotation_tool_group_trigger_key_down(
        &mut self,
        group: AnnotationToolGroup,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = {
            let app = self.app.read(cx);
            app.annotation_tool_group == Some(group)
                && app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str())
        };
        if close_more_actions_shortcut(&event.keystroke)
            && is_open
            && self.close_annotation_tool_group_from_keyboard(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        if is_open && let Some(direction) = annotation_tool_group_focus_direction(&event.keystroke)
        {
            let tools = WorkspaceAnnotationToolSpec::group_tools(group);
            let target = match direction {
                AnnotationToolGroupFocusDirection::Next => tools.iter().flatten().next().copied(),
                AnnotationToolGroupFocusDirection::Previous => {
                    tools.iter().flatten().last().copied()
                }
            };
            if let Some(target) = target
                && let Some(focus_handle) = self.annotation_tool_group_item_focus_handles
                    [group.index()]
                .iter()
                .zip(tools.iter().flatten())
                .find_map(|(focus_handle, candidate)| {
                    (*candidate == target).then(|| focus_handle.clone())
                })
            {
                focus_handle.focus(window, cx);
            }
            cx.stop_propagation();
            return;
        }
        stop_overlay_action_key_propagation(event, window, cx);
    }

    /// Dismisses a group before a canvas gesture can start and returns focus to that canvas.
    pub(super) fn dismiss_annotation_tool_group_from_pointer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.close_annotation_tool_group();
                    cx.notify();
                }
            });
        });
    }

    /// Dismisses More before a canvas gesture can start and returns focus to that canvas.
    pub(super) fn dismiss_more_actions_from_pointer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let operation_generation = self.operation_generation;
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, |app, cx| {
                if accepts_overlay_input(operation_generation, app.operation_generation) {
                    app.close_overlay_more_actions();
                    cx.notify();
                }
            });
        });
    }

    /// Opens More from the keyboard and waits one frame before focusing its first rendered action.
    pub(super) fn open_more_actions_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let owns_toolbar = {
            let app = self.app.read(cx);
            app.text_edit().is_none()
                && app.session.selection().is_some_and(|selection| {
                    owns_selection_toolbar(selection, self.display.physical_bounds)
                })
        };
        if !owns_toolbar {
            return false;
        }

        self.app.update(cx, |app, cx| {
            if !app.overlay_more_actions {
                app.toggle_overlay_more_actions(cx);
            }
        });
        let first_action = self.secondary_action_focus_handle(SecondaryAction::SaveAnnotations);
        cx.on_next_frame(window, move |_, window, cx| first_action.focus(window, cx));
        true
    }

    /// Closes More before Escape reaches the capture-wide cancellation shortcut.
    pub(super) fn close_more_actions_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let can_close = {
            let app = self.app.read(cx);
            app.text_edit().is_none() && app.overlay_more_actions
        };
        if !can_close {
            return false;
        }

        self.app
            .update(cx, |app, cx| app.toggle_overlay_more_actions(cx));
        let more_actions = self.more_actions_focus_handle.clone();
        cx.on_next_frame(window, move |_, window, cx| more_actions.focus(window, cx));
        true
    }

    /// Returns the persistent focus handle for a menu action across conditional re-renders.
    pub(super) fn secondary_action_focus_handle(&self, action: SecondaryAction) -> FocusHandle {
        self.secondary_action_focus_handles[action.index()].clone()
    }

    /// Restores focus to a persistent More action after a transient action removes itself.
    pub(super) fn focus_secondary_action_after_render(
        &self,
        action: SecondaryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action_focus = self.secondary_action_focus_handle(action);
        cx.on_next_frame(window, move |_, window, cx| action_focus.focus(window, cx));
    }
}
