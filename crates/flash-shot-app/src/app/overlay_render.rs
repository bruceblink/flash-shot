//! GPUI render composition for the capture overlay.

use super::*;

impl Render for CaptureOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.topmost_requested
            && let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.topmost_requested = true;
            let hwnd = handle.hwnd.get();
            // Keep the active capture overlay above ordinary application windows so its
            // selection controls remain available until the capture is completed or cancelled.
            cx.defer(move |_| {
                if let Err(error) = crate::platform::window_visibility::make_topmost(hwnd) {
                    log::warn!(target: "flash_shot::overlay", "overlay_topmost_failed error={error}");
                }
            });
        }
        let display_bounds = self.display.physical_bounds;
        let app = self.app.read(cx);
        let colors = app.colors;
        let workspace_colors = ThemeColors::for_mode(ThemeMode::Dark);
        let locale = app.settings.locale;
        // The session owns the committed selection. Keep rendering it after the
        // drag has ended, even if a late pointer event clears transient UI state.
        let selection = visible_selection(app.selection_drag, app.session.selection());
        let inspection_target = app.inspection_target;
        let annotations = app
            .annotation_document
            .as_ref()
            .map(|document| document.annotations().to_vec())
            .unwrap_or_default();
        let layer_annotations = annotations.clone();
        let annotation_preview = app
            .annotation_document
            .as_ref()
            .and_then(|document| app.annotation_editor.preview(document.canvas_bounds()));
        let text_edit = app.text_edit().cloned();
        let text_edit_annotation = app.text_edit_annotation();
        let selected_annotation = app.selected_annotation;
        let selected_annotation_object =
            selected_annotation.and_then(|id| app.annotation_document.as_ref()?.annotation(id));
        let can_delete = selected_annotation.is_some();
        let selected_tool = app.annotation_tool;
        let can_edit_text = selected_annotation_object.is_some_and(|annotation| {
            matches!(
                annotation.kind,
                AnnotationKind::Text { .. } | AnnotationKind::Watermark { .. }
            )
        });
        let can_rotate =
            selected_annotation_object.is_some_and(Annotation::supports_clockwise_rotation);
        let style_capabilities =
            annotation_style_capabilities(selected_tool, selected_annotation_object);
        let selected_number = selected_annotation.and_then(|id| {
            app.annotation_document
                .as_ref()?
                .annotation(id)
                .and_then(|annotation| match annotation.kind {
                    AnnotationKind::Number { value, .. } => Some(value),
                    _ => None,
                })
        });
        let annotation_color = app.annotation_style.stroke_rgba;
        let annotation_width = app.annotation_style.stroke_width;
        let annotation_font_size = app.annotation_style.text_font_size;
        let annotation_opacity = (app.annotation_style.stroke_rgba & 0xFF) as u8;
        let fill_enabled = app.annotation_style.fill_rgba.is_some();
        let can_undo = app.annotation_history.undo_len() > 0;
        let can_redo = app.annotation_history.redo_len() > 0;
        let status = app.status.clone();
        let show_more_actions = app.overlay_more_actions;
        let recognition_result = app.recognition_result.clone();
        let recognition_retry = app.recognition_retry;
        let recognition_in_flight = app.recognition_in_flight;
        let mut visible_secondary_actions = ALWAYS_VISIBLE_SECONDARY_ACTIONS.to_vec();
        if recognition_retry.is_some() {
            visible_secondary_actions.push(SecondaryAction::RetryRecognition);
        }
        if recognition_result.is_some() {
            visible_secondary_actions.extend([
                SecondaryAction::CopyRecognition,
                SecondaryAction::ClearRecognition,
            ]);
        }
        let secondary_navigation = SecondaryActionNavigation {
            current: SecondaryAction::SaveAnnotations,
            visible_actions: visible_secondary_actions,
            focus_handles: self.secondary_action_focus_handles.clone(),
        };
        let more_actions_navigation = secondary_navigation.clone();
        let hover_pixel = app.hover_pixel;
        let frame = app.frame.clone();
        let viewport = local_viewport(window);
        self.last_workspace_scale_factor = window.scale_factor();
        let show_inline_secondary_actions =
            view_rect(viewport).width >= INLINE_SECONDARY_ACTIONS_MIN_VIEWPORT_WIDTH;
        self.annotation_arrange_actions_for =
            arrange_context_for_selection(self.annotation_arrange_actions_for, selected_annotation);
        let show_annotation_arrange_actions = self.annotation_arrange_actions_for.is_some();
        let annotation_toolbar_items = annotation_toolbar_items(
            can_delete,
            can_edit_text,
            selected_number.is_some(),
            can_rotate,
            show_annotation_arrange_actions,
        );
        let transform = self.transform(viewport);
        let selected_on_display =
            selection.and_then(|selection| intersect(selection, display_bounds));
        let annotation_tool_width = if locale == Locale::SimplifiedChinese {
            ThemeMetrics::WORKSPACE_TOOL_CELL_WIDTH_COMPACT
        } else {
            ANNOTATION_TOOL_ESTIMATED_WIDTH
        };
        let owns_action_toolbar =
            selection.is_some_and(|selection| owns_selection_toolbar(selection, display_bounds));
        let show_annotation_controls =
            annotation_controls_visible(app.overlay_annotation_controls, selection, display_bounds);
        let annotation_tool_group = app.annotation_tool_group.filter(|_| {
            app.annotation_tool_group_owner.as_deref() == Some(self.display.id.as_str())
        });
        let show_annotation_tool_group_dismiss = app.annotation_tool_group.is_some();
        // Show only renderer-backed controls for the active tool. This keeps the style row
        // contextual while matching Snow Shot's selected-tool workspace without opening More.
        let show_annotation_style = show_annotation_controls && style_capabilities.has_controls();
        let visible_style_capabilities = if show_annotation_style {
            style_capabilities
        } else {
            AnnotationStyleCapabilities::EMPTY
        };
        let mut layout_snapshot = workspace_layout_snapshot(WorkspaceLayoutInput {
            selection,
            display_bounds,
            transform,
            viewport,
            hover_pixel,
            inspection_target,
            show_annotation_controls,
            annotation_toolbar_items,
            annotation_style_capabilities: visible_style_capabilities,
            annotation_tool_group,
            annotation_tool_width,
            has_recognition_result: recognition_result.is_some(),
            has_recognition_retry: recognition_retry.is_some(),
            recognition_in_flight,
        });
        if self
            .manual_toolbar_position
            .is_some_and(|position| Some(position.selection) != selection)
        {
            self.reset_toolbar_position();
        }
        if let Some(position) = self.manual_toolbar_position {
            apply_manual_toolbar_position(&mut layout_snapshot, position, viewport);
        }
        if show_more_actions && self.manual_toolbar_position.is_none() {
            position_workspace_for_more_menu(&mut layout_snapshot, selection, viewport);
        }
        self.last_workspace_snapshot = Some(layout_snapshot);
        let action_layout = layout_snapshot.action_toolbar;
        let annotation_layout = layout_snapshot.annotation_toolbar;
        let annotation_layer_layout = layout_snapshot.annotation_layer;
        let show_annotation_layers = owns_action_toolbar
            && !layer_annotations.is_empty()
            && (!show_annotation_controls || annotation_layer_layout.is_some());
        let annotation_layer_top = annotation_layer_layout
            .map(|layout| layout.top)
            .unwrap_or(OVERLAY_EDGE_INSET);
        let annotation_layer_max_height = annotation_layer_layout
            .map(|layout| layout.max_height)
            .unwrap_or_else(|| {
                (view_rect(viewport).height - annotation_layer_top - OVERLAY_BOTTOM_SAFE_INSET)
                    .max(80.0)
            });
        let secondary_menu = layout_snapshot.secondary_menu;
        let dimension_layout = layout_snapshot.dimension;
        let status_inset = layout_snapshot.status_inset;
        let smart_target_hud = layout_snapshot.smart_target_hud;
        let target_on_display = selection
            .is_none()
            .then(|| inspection_target.and_then(|target| intersect(target.bounds, display_bounds)))
            .flatten();
        let has_selection = selection.is_some();
        let selection_copy_in_progress = app.selection_copy_is_active();
        // Copy owns a frozen snapshot, not this editor. Keep the normal action bar available so
        // large PNG/DIB encoding never turns a simple Copy into a workflow dead end.
        let can_export = has_selection && owns_action_toolbar;
        let show_action_toolbar = !has_selection || owns_action_toolbar;
        let selection_cursor = selection_cursor(
            selection,
            transform,
            view_point(window.mouse_position()),
            selected_tool.is_some(),
            app.selection_drag.is_moving(),
        );
        let text_input_focus = self.focus_handle.clone();
        let text_input_app = self.app.clone();
        let inline_secondary_action_buttons = WorkspaceInlineAction::catalog()
            .iter()
            .copied()
            .map(|action| self.inline_secondary_action_button(action, locale, workspace_colors, cx))
            .collect::<Vec<_>>();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(|this, event, window, cx| {
                if this.toolbar_drag.is_some() {
                    this.update_toolbar_drag(event, local_viewport(window), cx);
                    cx.stop_propagation();
                }
            }))
            .capture_any_mouse_up(cx.listener(
                |this, event: &MouseUpEvent, window, cx| {
                    if this.toolbar_drag.is_some() {
                        this.finish_toolbar_drag(event, local_viewport(window), cx);
                    }
                },
            ))
            .bg(colors.background)
            .child(
                img(self.preview.clone())
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .cursor_crosshair()
                    .when(selection_cursor == SelectionCursor::Move, |overlay| {
                        overlay.cursor_move()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNwse, |overlay| {
                        overlay.cursor_nwse_resize()
                    })
                    .when(selection_cursor == SelectionCursor::ResizeNesw, |overlay| {
                        overlay.cursor_nesw_resize()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.begin_selection(event, local_viewport(window), cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event, window, cx| {
                        if this.toolbar_drag.is_some() {
                            this.update_toolbar_drag(event, local_viewport(window), cx);
                            return;
                        }
                        let viewport = local_viewport(window);
                        this.update_selection(event, viewport, window, cx)
                    }))
                    .capture_any_mouse_up(cx.listener(
                        move |this, event: &MouseUpEvent, window, cx| {
                            if this.toolbar_drag.is_some() {
                                this.finish_toolbar_drag(event, local_viewport(window), cx);
                                return;
                            }
                            if event.button == MouseButton::Left && this.selection_pointer_active {
                                this.finish_selection(event, local_viewport(window), cx);
                            }
                        },
                    ))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                (bounds, transform, selected_on_display, target_on_display)
                            },
                            move |bounds, (_, transform, selection, target), window, _| {
                                paint_selection_mask(
                                    window, bounds, transform, selection, target, colors,
                                )
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        (
                            bounds,
                            transform,
                            annotations,
                            annotation_preview,
                            selected_annotation,
                            text_edit_annotation,
                            text_edit,
                        )
                    },
                    move |bounds,
                          (
                        _,
                        transform,
                        annotations,
                        preview,
                        selected_annotation,
                        text_edit_annotation,
                        text_edit,
                    ),
                          window,
                          cx| {
                        // GPUI only accepts native IME registration during paint. Keeping it in
                        // this canvas lets text and watermark edits receive input without
                        // triggering a paint-phase assertion from the Windows backend.
                        if text_edit.is_some() {
                            window.handle_input(
                                &text_input_focus,
                                ElementInputHandler::new(bounds, text_input_app.clone()),
                                cx,
                            );
                        }
                        paint_annotations(
                            window,
                            transform,
                            &annotations,
                            preview.as_ref(),
                            AnnotationPaintState {
                                selected: selected_annotation,
                                hidden: text_edit_annotation,
                            },
                            colors,
                            cx,
                        );
                        if let Some(edit) = text_edit {
                            paint_text_annotation(
                                window,
                                transform.unwrap_or_else(|| unreachable!()),
                                edit.origin,
                                &edit.content,
                                0xFFFFFFFF,
                                annotation_font_size,
                                cx,
                            );
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
            .child(
                canvas(
                    move |bounds, _, _| (bounds, transform, hover_pixel, frame),
                    move |viewport, (_, transform, hover_pixel, frame), window, _| {
                        paint_magnifier(window, viewport, transform, hover_pixel, frame.as_ref())
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
            .when_some(dimension_layout, |overlay, layout| {
                let selection = selected_on_display.expect("dimension layout requires selection");
                overlay.child(
                    div()
                        .id("overlay-selection-dimensions")
                        .occlude()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(OVERLAY_DIMENSION_LABEL_WIDTH))
                        .h(px(OVERLAY_DIMENSION_LABEL_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .bg(rgba(0x0B0D10E6))
                        .border_1()
                        .border_color(colors.accent)
                        .text_color(colors.overlay_text)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow_lg()
                        .child(selection_dimension_label(locale, selection)),
                )
            })
            .when_some(smart_target_hud, |overlay, layout| {
                // Keep this read-only HUD out of hit testing so selection gestures still reach
                // the full-screen canvas behind it.
                overlay.child(
                    div()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(layout.width))
                        .h(px(OVERLAY_SMART_TARGET_HUD_HEIGHT))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.accent)
                        .bg(rgba(0x0B0D10E6))
                        .text_color(colors.overlay_text)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow_lg()
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .text_ellipsis()
                                .child(smart_target_hud_label(locale, layout.target)),
                        ),
                )
            })
            .when_some(annotation_layout, |overlay, layout| {
                overlay.child(
                    div()
                        .id("overlay-marking-panel")
                        .occlude()
                        .absolute()
                        .left(px(layout.left))
                        .top(px(layout.top))
                        .w(px(layout.width))
                        .h(px(layout.height)),
                )
            })
            .when(show_annotation_layers, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-layers")
                        .occlude()
                        .absolute()
                        .when_some(annotation_layer_layout, |layers, layout| {
                            layers.left(px(layout.left))
                        })
                        .when(annotation_layer_layout.is_none(), |layers| {
                            layers.right(px(OVERLAY_EDGE_INSET))
                        })
                        .top(px(annotation_layer_top))
                        .w(px(ANNOTATION_LAYERS_WIDTH))
                        .max_h(px(annotation_layer_max_height))
                        .overflow_y_scroll()
                        .p_2()
                        .bg(workspace_colors.toolbar_surface)
                        .border_1()
                        .border_color(workspace_colors.toolbar_border)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(workspace_colors.overlay_muted)
                                .child(locale.text(UiText::OverlayLayers)),
                        )
                        .children(layer_annotations.iter().rev().enumerate().map(
                            |(reverse_index, annotation)| {
                                let id = annotation.id;
                                let position = layer_annotations.len() - reverse_index;
                                let is_selected = selected_annotation == Some(id);
                                div()
                                    .id(format!("overlay-layer-{}", id.value()))
                                    .px_2()
                                    .py_1()
                                    .bg(if is_selected {
                                        workspace_colors.accent
                                    } else {
                                        workspace_colors.toolbar_surface
                                    })
                                    .text_color(if is_selected {
                                        workspace_colors.background
                                    } else {
                                        workspace_colors.text
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let app = this.app.clone();
                                        cx.defer(move |cx| {
                                            app.update(cx, |app, cx| {
                                                app.select_annotation_layer(id, cx);
                                            });
                                        });
                                    }))
                                    .child(annotation_layer_entry_label(
                                        locale,
                                        position,
                                        &annotation.kind,
                                    ))
                            },
                        )),
                )
            })
            .when(show_annotation_tool_group_dismiss, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-annotation-tool-group-dismiss")
                        .occlude()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_annotation_tool_group_from_pointer(window, cx);
                            }),
                        ),
                )
            })
            .when(show_more_actions, |overlay| {
                overlay.child(
                    div()
                        .id("overlay-more-actions-dismiss")
                        .occlude()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.dismiss_more_actions_from_pointer(window, cx);
                            }),
                        ),
                )
            })
            .when(
                show_annotation_controls
                    && (annotation_tool_group.is_some() || selected_annotation.is_some()),
                |overlay| {
                overlay.child(
                    div()
                        .id("overlay-annotation-tools")
                        .absolute()
                        .when_some(annotation_layout, |tools, layout| {
                            tools
                                .left(px(layout.left))
                                .top(px(layout.tools_top))
                                .w(px(layout.tools_width))
                        })
                        .when(annotation_layout.is_none(), |tools| {
                            tools
                                .left(px(OVERLAY_EDGE_INSET))
                                .right(px(OVERLAY_EDGE_INSET))
                                .top(px(OVERLAY_EDGE_INSET))
                        })
                        .flex_col()
                        .gap(px(ANNOTATION_TOOL_GAP))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        // Reserve the same row height as the inline palette. The old standalone
                        // palette is intentionally absent so the reference toolbar stays one row;
                        // the child popover below still uses this anchor when a group is open.
                        .child(div().h(px(ANNOTATION_TOOL_PALETTE_HEIGHT)))
                        .when_some(annotation_tool_group, |tools, group| {
                            let group_tools = WorkspaceAnnotationToolSpec::group_tools(group);
                            let popup_width = layout_snapshot
                                .annotation_tool_group
                                .map(|layout| layout.width)
                                .unwrap_or_else(|| {
                                    annotation_tool_group_popover_width(
                                        annotation_layout
                                            .map(|layout| layout.tools_width)
                                            .unwrap_or_else(|| {
                                                (view_rect(viewport).width
                                                    - OVERLAY_EDGE_INSET * 2.0)
                                                    .clamp(1.0, ANNOTATION_TOOLBAR_MAX_WIDTH)
                                            }),
                                        group,
                                        annotation_tool_width,
                                    )
                                });
                            tools.child(
                                workspace_surface(workspace_colors, true)
                                    .id(format!(
                                        "overlay-tool-group-popover-{}",
                                        annotation_tool_group_key(group)
                                    ))
                                    .occlude()
                                    .w(px(popup_width))
                                    .p(px(ANNOTATION_TOOL_GROUP_POPUP_PADDING))
                                    .flex()
                                    .flex_wrap()
                                    .gap(px(ANNOTATION_TOOL_GAP))
                                    .children(group_tools.into_iter().flatten().enumerate().map(
                                        |(index, tool)| {
                                            let focus_handle = self
                                                .annotation_tool_group_item_focus_handle(
                                                    group, index,
                                                );
                                            let active = selected_tool == Some(tool);
                                            annotation_tool_button(
                                                format!(
                                                    "overlay-tool-group-{}-{}",
                                                    annotation_tool_group_key(group),
                                                    annotation_tool_key(tool)
                                                ),
                                                locale.text(annotation_tool_ui_text(tool)),
                                                workspace_colors,
                                                active,
                                                annotation_tool_width,
                                                None,
                                                cx.listener(move |this, _, window, cx| {
                                                    this.select_annotation_tool_from_group(
                                                        group, tool, window, cx,
                                                    );
                                                }),
                                            )
                                            .track_focus(&focus_handle)
                                            .on_key_down(cx.listener(
                                                move |this, event, window, cx| {
                                                    this.handle_annotation_tool_group_item_key_down(
                                                        group, tool, event, window, cx,
                                                    );
                                                },
                                            ))
                                        },
                                    )),
                            )
                        })
                        .when_some(selected_annotation, |tools, selected_id| {
                            tools.child(
                                div()
                                    .id("overlay-selection-context")
                                    .w_full()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .h(px(ANNOTATION_ACTION_HEIGHT))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .text_color(workspace_colors.overlay_muted)
                                            .text_xs()
                                            .child(locale.text(UiText::OverlaySelected)),
                                    )
                                    .child(annotation_action_button(
                                        "overlay-delete",
                                        locale.text(UiText::OverlayDelete),
                                        workspace_colors,
                                        AnnotationActionTone::Destructive,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.delete_selected_annotation(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .when(can_edit_text, |actions| {
                                        actions.child(annotation_action_button(
                                            "overlay-edit-text",
                                            locale.text(UiText::OverlayEditText),
                                            workspace_colors,
                                            AnnotationActionTone::Primary,
                                            true,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.edit_selected_text_annotation(cx);
                                                    });
                                                });
                                            }),
                                        ))
                                    })
                                    .when_some(selected_number, |actions, value| {
                                        actions
                                            .child(annotation_action_button(
                                                "overlay-number-decrement",
                                                "-",
                                                workspace_colors,
                                                AnnotationActionTone::Neutral,
                                                true,
                                                cx.listener(|this, _, _, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.adjust_selected_number(-1, cx);
                                                        });
                                                    });
                                                }),
                                            ))
                                            .child(
                                                div()
                                                    .h(px(ANNOTATION_ACTION_HEIGHT))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_md()
                                                    .bg(workspace_colors.toolbar_elevated)
                                                    .text_color(workspace_colors.text)
                                                    .child(annotation_number_value_label(
                                                        locale, value,
                                                    )),
                                            )
                                            .child(annotation_action_button(
                                                "overlay-number-increment",
                                                "+",
                                                workspace_colors,
                                                AnnotationActionTone::Neutral,
                                                true,
                                                cx.listener(|this, _, _, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.adjust_selected_number(1, cx);
                                                        });
                                                    });
                                                }),
                                            ))
                                    })
                                    .child(annotation_action_button(
                                        "overlay-duplicate",
                                        locale.text(UiText::OverlayDuplicate),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.duplicate_selected_annotation(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-selection-arrange-toggle",
                                        locale.text(UiText::OverlayArrange),
                                        workspace_colors,
                                        if show_annotation_arrange_actions {
                                            AnnotationActionTone::Primary
                                        } else {
                                            AnnotationActionTone::Neutral
                                        },
                                        true,
                                        cx.listener(move |this, _, _, cx| {
                                            this.annotation_arrange_actions_for = if this
                                                .annotation_arrange_actions_for
                                                == Some(selected_id)
                                            {
                                                None
                                            } else {
                                                Some(selected_id)
                                            };
                                            cx.notify();
                                        }),
                                    )),
                            )
                        })
                        .when(show_annotation_arrange_actions, |tools| {
                            tools.child(
                                div()
                                    .id("overlay-selection-arrange-context")
                                    .w_full()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .h(px(ANNOTATION_ACTION_HEIGHT))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .text_color(workspace_colors.overlay_muted)
                                            .text_xs()
                                            .child(locale.text(UiText::OverlayArrange)),
                                    )
                                    .when(can_rotate, |actions| {
                                        actions.child(annotation_action_button(
                                            "overlay-rotate-clockwise",
                                            locale.text(UiText::OverlayRotate90),
                                            workspace_colors,
                                            AnnotationActionTone::Neutral,
                                            true,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.rotate_selected_annotation_clockwise(
                                                            cx,
                                                        );
                                                    });
                                                });
                                            }),
                                        ))
                                    })
                                    .child(annotation_action_button(
                                        "overlay-bring-forward",
                                        locale.text(UiText::OverlayBringForward),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_forward(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-send-backward",
                                        locale.text(UiText::OverlaySendBackward),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_backward(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-bring-to-front",
                                        locale.text(UiText::OverlayBringToFront),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.bring_selected_annotation_to_front(cx);
                                                });
                                            });
                                        }),
                                    ))
                                    .child(annotation_action_button(
                                        "overlay-send-to-back",
                                        locale.text(UiText::OverlaySendToBack),
                                        workspace_colors,
                                        AnnotationActionTone::Neutral,
                                        true,
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.send_selected_annotation_to_back(cx);
                                                });
                                            });
                                        }),
                                    )),
                            )
                        }),
                )
            })
            .when(show_annotation_style, |overlay| {
                let style_row = workspace_surface(workspace_colors, false)
                    .id("overlay-annotation-style-row")
                    .occlude()
                    .absolute()
                    .when_some(annotation_layout, |style, layout| {
                        style
                            .left(px(layout.style_left))
                            .top(px(layout.style_top))
                            .w(px(layout.tools_width))
                            .h(px(layout.style_height))
                    })
                    .when(annotation_layout.is_none(), |style| {
                        style
                            .left(px(OVERLAY_EDGE_INSET))
                            .right(px(OVERLAY_EDGE_INSET))
                            .top(px(OVERLAY_EDGE_INSET))
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                    .p(px(ANNOTATION_STYLE_ROW_PADDING))
                    .children(
                        [
                            style_capabilities.color.then(|| {
                                div()
                                    .id("overlay-style-colors")
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_COLORS.into_iter().map(|color| {
                                        workspace_swatch(
                                            format!("overlay-color-{color:08x}"),
                                            gpui::Hsla::from(rgba(color)),
                                            color == annotation_color,
                                            locale.text(UiText::AnnotationColorSelected),
                                            locale.text(UiText::AnnotationColorSelected),
                                            workspace_colors,
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_color(color, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.fill.then(|| {
                                div()
                                    .id("overlay-style-fill-group")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .child(annotation_style_button(
                                        "overlay-fill",
                                        locale.text(UiText::OverlayFill),
                                        locale.text(UiText::OverlayFill),
                                        WorkspaceButtonConfig::text(
                                            Some(ANNOTATION_STYLE_FILL_WIDTH),
                                            ANNOTATION_STYLE_CONTROL_HEIGHT,
                                            workspace_colors,
                                            WorkspaceButtonTone::Neutral,
                                            fill_enabled,
                                            true,
                                            Some(locale.text(UiText::OverlayFill)),
                                        ),
                                        cx.listener(|this, _, _, cx| {
                                            let app = this.app.clone();
                                            cx.defer(move |cx| {
                                                app.update(cx, |app, cx| {
                                                    app.toggle_annotation_fill(cx)
                                                });
                                            });
                                        }),
                                    ))
                            }),
                            style_capabilities.width.then(|| {
                                div()
                                    .id("overlay-style-widths")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_WIDTHS.into_iter().map(|width| {
                                        let value = annotation_width_value_label(locale, width);
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationWidth,
                                            &[("width", &value)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-width-{width}"),
                                            value,
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_VALUE_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                width == annotation_width,
                                                true,
                                                Some(locale.text(UiText::AnnotationWidthLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_width(width, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.opacity.then(|| {
                                div()
                                    .id("overlay-style-opacity")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_OPACITIES.into_iter().map(|opacity| {
                                        let percent = (u16::from(opacity) * 100 / 255).to_string();
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationOpacity,
                                            &[("percent", &percent)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-opacity-{opacity}"),
                                            annotation_opacity_value_label(locale, opacity),
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_OPACITY_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                opacity == annotation_opacity,
                                                true,
                                                Some(locale.text(UiText::AnnotationOpacityLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_opacity(opacity, cx)
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                            style_capabilities.font_size.then(|| {
                                div()
                                    .id("overlay-style-font-sizes")
                                    .border_l_1()
                                    .border_color(workspace_colors.toolbar_border)
                                    .pl_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(ANNOTATION_STYLE_CONTROL_GAP))
                                    .children(ANNOTATION_FONT_SIZES.into_iter().map(|font_size| {
                                        let value = annotation_text_size_value_label(locale, font_size);
                                        let aria_label = locale.format_template(
                                            UiText::AnnotationTextSize,
                                            &[("size", &value)],
                                        );
                                        annotation_style_button(
                                            format!("overlay-font-size-{font_size}"),
                                            value,
                                            aria_label,
                                            WorkspaceButtonConfig::text(
                                                Some(ANNOTATION_STYLE_VALUE_WIDTH),
                                                ANNOTATION_STYLE_CONTROL_HEIGHT,
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                font_size == annotation_font_size,
                                                true,
                                                Some(locale.text(UiText::AnnotationTextSizeLabel)),
                                            ),
                                            cx.listener(move |this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_annotation_font_size(
                                                            font_size, cx,
                                                        )
                                                    });
                                                });
                                            }),
                                        )
                                    }))
                            }),
                        ]
                        .into_iter()
                        .flatten(),
                    );
                overlay.child(style_row)
            })
            .child(
                div()
                    .id("overlay-status")
                    .occlude()
                    .absolute()
                    .left(px(18.0))
                    .bottom(px(status_inset))
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgba(0xFFFFFF24))
                    .bg(rgba(0x0B0D10E6))
                    .text_color(colors.overlay_text)
                    .text_sm()
                    .shadow_lg()
                    .child(status),
            )
            .child(
                div()
                    // These controls sit over the full-screen selection canvas. Their hitbox must
                    // keep a command click from also starting or completing a selection underneath.
                    .id("overlay-actions")
                    .occlude()
                    .absolute()
                    .when(!show_action_toolbar, |actions| actions.hidden())
                    .when_some(action_layout, |actions, layout| {
                        actions
                            .left(px(layout.left))
                            .top(px(layout.top))
                            .w(px(layout.width))
                    })
                    .when(action_layout.is_none(), |actions| {
                        actions
                            .right(px(OVERLAY_EDGE_INSET))
                            .bottom(px(OVERLAY_BOTTOM_SAFE_INSET))
                    })
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(OVERLAY_ACTION_ITEM_GAP))
                    .px(px(OVERLAY_ACTION_BAR_HORIZONTAL_PADDING))
                    .py(px(OVERLAY_ACTION_BAR_PADDING))
                    .rounded(px(ThemeMetrics::default().radius_md))
                    .border_1()
                    .border_color(workspace_colors.toolbar_border)
                    .bg(workspace_colors.toolbar_surface)
                    .shadow_lg()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(can_export, |actions| {
                        let toolbar_layout_for_drag = action_layout;
                        let selection_for_drag = selection;
                        actions
                            .child(
                                workspace_drag_handle(workspace_colors).on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event, _, cx| {
                                        this.begin_toolbar_drag(
                                            event,
                                            toolbar_layout_for_drag,
                                            selection_for_drag,
                                            cx,
                                        );
                                    }),
                                ),
                            )
                            .when(show_annotation_controls, |actions| {
                                actions.child(annotation_toolbar_icon_button(
                                            "overlay-tool-selection",
                                            icon::MOVE,
                                            locale.text(UiText::OverlaySelect),
                                            locale.text(UiText::OverlaySelect),
                                            workspace_colors,
                                            selected_tool.is_none(),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.select_selection_tool(cx);
                                                    });
                                                });
                                            }),
                                        ))
                                    .child(workspace_separator(
                                        "overlay-tool-selection-separator",
                                        workspace_colors,
                                    ))
                                    .children(
                                            WorkspaceAnnotationToolSpec::catalog()
                                                .iter()
                                                .copied()
                                                .map(|spec| {
                                                let tool = spec.tool();
                                                let group = spec.group();
                                                let active = selected_tool == Some(tool);
                                                let expanded = group.is_some_and(|group| {
                                                    annotation_tool_group == Some(group)
                                                });
                                                let label = locale.text(spec.label());
                                                let tooltip = annotation_tool_tooltip(locale, spec);
                                                let button = annotation_toolbar_icon_button(
                                                    format!("overlay-tool-{}", spec.id()),
                                                    spec.icon(),
                                                    label,
                                                    tooltip,
                                                    workspace_colors,
                                                    active,
                                                    cx.listener(move |this, _, window, cx| {
                                                        if active && let Some(group) = group {
                                                            this
                                                                .toggle_annotation_tool_group_from_trigger(
                                                                    group, window, cx,
                                                                );
                                                        } else {
                                                            let app = this.app.clone();
                                                            cx.defer(move |cx| {
                                                                app.update(cx, |app, cx| {
                                                                    app.select_annotation_tool(
                                                                        tool, cx,
                                                                    );
                                                                });
                                                            });
                                                        }
                                                    }),
                                                )
                                                .aria_expanded(expanded);
                                                match group {
                                                    Some(group)
                                                        if WorkspaceAnnotationToolSpec::group_tools(
                                                            group,
                                                        )
                                                        .into_iter()
                                                        .flatten()
                                                        .next()
                                                            == Some(tool) =>
                                                    {
                                                        let focus_handle = self
                                                            .annotation_tool_group_trigger_focus_handle(
                                                                group,
                                                            );
                                                        button
                                                            .track_focus(&focus_handle)
                                                            .on_key_down(cx.listener(
                                                                move |this, event, window, cx| {
                                                                    this
                                                                        .handle_annotation_tool_group_trigger_key_down(
                                                                            group, event, window,
                                                                            cx,
                                                                        );
                                                                },
                                                            ))
                                                    }
                                                    Some(group) => button.on_key_down(cx.listener(
                                                        move |this, event, window, cx| {
                                                            this
                                                                .handle_annotation_tool_group_trigger_key_down(
                                                                    group, event, window, cx,
                                                                );
                                                        },
                                                    )),
                                                    None => button,
                                                }
                                            }),
                                        )
                            })
                            .when(show_annotation_controls, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-action-context-separator",
                                    workspace_colors,
                                ))
                            })
                            .when(show_annotation_controls, |actions| {
                                actions.child(
                                    div()
                                        .id("overlay-annotation-context-actions")
                                        .flex()
                                        .items_center()
                                        .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                        .child(workspace_icon_button(
                                            "overlay-undo",
                                            icon::UNDO,
                                            locale.text(UiText::OverlayUndo),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                can_undo,
                                                locale.text(UiText::OverlayUndo),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.undo_annotation(cx)
                                                    });
                                                });
                                            }),
                                        ))
                                        .child(workspace_icon_button(
                                            "overlay-redo",
                                            icon::REDO,
                                            locale.text(UiText::OverlayRedo),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                can_redo,
                                                locale.text(UiText::OverlayRedo),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.redo_annotation(cx)
                                                    });
                                                });
                                            }),
                                        )),
                                )
                            })
                            .when(!show_annotation_controls, |actions| actions.child(
                                div()
                                    .id("overlay-mode-actions")
                                    .flex()
                                    .items_center()
                                    .child(
                                        workspace_icon_button(
                                            "overlay-annotation-controls",
                                            icon::MARK,
                                            locale.text(UiText::OverlayMark),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                show_annotation_controls,
                                                true,
                                                primary_action_tooltip(locale, "draw"),
                                            ),
                                            cx.listener(|this, _, window, cx| {
                                                // Return focus to the overlay canvas after opening the
                                                // annotation palette so keyboard tools remain available
                                                // without requiring an extra click on the image.
                                                this.focus_handle.focus(window, cx);
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.toggle_overlay_annotation_controls(cx)
                                                    })
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                            ))
                            .when(show_annotation_controls && !show_inline_secondary_actions, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-action-result-separator",
                                    workspace_colors,
                                ))
                            })
                            .when(show_inline_secondary_actions, |actions| {
                                actions
                                    .child(workspace_separator(
                                        "overlay-inline-secondary-separator",
                                        workspace_colors,
                                    ))
                                    .child(
                                        div()
                                            .id("overlay-inline-secondary-actions")
                                            .flex()
                                            .items_center()
                                            .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                            .children(inline_secondary_action_buttons),
                                    )
                            })
                            .when(show_inline_secondary_actions, |actions| {
                                actions.child(workspace_separator(
                                    "overlay-inline-result-separator",
                                    workspace_colors,
                                ))
                            })
                            .child(
                                div()
                                    .id("overlay-result-actions")
                                    .flex()
                                    .items_center()
                                    .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                    .child(
                                        workspace_icon_button(
                                            WorkspaceResultAction::Pin.id(),
                                            WorkspaceResultAction::Pin.icon(),
                                            locale.text(UiText::OverlayPin),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                true,
                                                locale.text(UiText::OverlayPinTooltip),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.pin_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            WorkspaceResultAction::Save.id(),
                                            WorkspaceResultAction::Save.icon(),
                                            locale.text(UiText::OverlaySave),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                false,
                                                true,
                                                primary_action_tooltip(locale, "save"),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.save_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            WorkspaceResultAction::More.id(),
                                            WorkspaceResultAction::More.icon(),
                                            more_actions_button_label(locale, show_more_actions),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Neutral,
                                                show_more_actions,
                                                true,
                                                if show_more_actions {
                                                    locale.text(UiText::OverlayHideMoreTooltip)
                                                } else {
                                                    locale.text(UiText::OverlayShowMoreTooltip)
                                                },
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.toggle_overlay_more_actions(cx)
                                                    })
                                                });
                                            }),
                                        )
                                        .track_focus(&self.more_actions_focus_handle)
                                        .aria_keyshortcuts("Alt+M")
                                        .aria_expanded(show_more_actions)
                                        .on_key_down(
                                            move |event, window, cx| {
                                                handle_more_actions_key_down(
                                                    show_more_actions,
                                                    &more_actions_navigation,
                                                    event,
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            WorkspaceResultAction::Cancel.id(),
                                            WorkspaceResultAction::Cancel.icon(),
                                            locale.text(UiText::OverlayCancel),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Destructive,
                                                false,
                                                true,
                                                primary_action_tooltip(locale, "cancel"),
                                            ),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.reset(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    )
                                    .child(
                                        workspace_icon_button(
                                            WorkspaceResultAction::Copy.id(),
                                            WorkspaceResultAction::Copy.icon(),
                                            locale.text(UiText::OverlayCopy),
                                            WorkspaceButtonConfig::icon(
                                                workspace_colors,
                                                WorkspaceButtonTone::Primary,
                                                false,
                                                !selection_copy_in_progress,
                                                if selection_copy_in_progress {
                                                    locale.text(UiText::OverlayCopyingTooltip)
                                                } else {
                                                    primary_action_tooltip(locale, "copy")
                                                },
                                            ),
                                            cx.listener(|this, _, window, cx| {
                                                // Copy becomes disabled while its background worker runs.
                                                // Keep keyboard cancellation routed through the overlay
                                                // instead of leaving focus on the disabled toolbar button.
                                                this.focus_handle.focus(window, cx);
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| app.copy_selection(cx))
                                                });
                                            }),
                                        )
                                        .on_key_down(stop_overlay_action_key_propagation),
                                    ),
                            )
                            .when(show_more_actions, |actions| {
                                actions.child(
                                    workspace_surface(workspace_colors, false)
                                        .id("overlay-secondary-actions")
                                        .tab_group()
                                        .occlude()
                                        .absolute()
                                        .when_some(secondary_menu, |menu, layout| {
                                            menu.w(px(layout.width))
                                                .left(px(layout.left))
                                                .top(px(layout.top_offset))
                                        })
                                        .p(px(OVERLAY_ACTION_BAR_PADDING))
                                        .flex()
                                        .flex_wrap()
                                        .justify_end()
                                        .gap(px(OVERLAY_ACTION_ITEM_GAP))
                                        .child(secondary_action_button(
                                            SecondaryAction::SaveAnnotations.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveAnnotations),
                                            locale.text(UiText::OverlaySaveAnnotations),
                                            SecondaryAction::SaveAnnotations.width(),
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_annotation_document(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::SaveEditable.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::SaveEditable),
                                            locale.text(UiText::OverlaySaveEditable),
                                            SecondaryAction::SaveEditable.width(),
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.save_editable_project(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::OpenAnnotations.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::OpenAnnotations),
                                            locale.text(UiText::OverlayOpenAnnotations),
                                            SecondaryAction::OpenAnnotations.width(),
                                            workspace_colors,
                                            false,
                                            None,
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.open_annotation_document(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::QuickSave.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::QuickSave),
                                            locale.text(UiText::OverlayQuickSave),
                                            SecondaryAction::QuickSave.width(),
                                            workspace_colors,
                                            true,
                                            Some(locale.text(UiText::OverlayQuickSaveTooltip)),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.quick_save_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::ScrollShot.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::ScrollShot),
                                            locale.text(UiText::OverlayScrollShot),
                                            SecondaryAction::ScrollShot.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "scroll")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_manual_scroll(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::Qr.id(),
                                            secondary_navigation.for_action(SecondaryAction::Qr),
                                            locale.text(UiText::OverlayQr),
                                            SecondaryAction::Qr.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "qr")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_qr_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::Ocr.id(),
                                            secondary_navigation.for_action(SecondaryAction::Ocr),
                                            locale.text(UiText::OverlayOcr),
                                            SecondaryAction::Ocr.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "ocr")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.recognize_text_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::CopyColor.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::CopyColor),
                                            locale.text(UiText::OverlayCopyColor),
                                            SecondaryAction::CopyColor.width(),
                                            workspace_colors,
                                            false,
                                            Some(locale.text(UiText::OverlayCopyColorTooltip)),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.copy_hover_color(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::Translate.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::Translate),
                                            locale.text(UiText::OverlayTranslate),
                                            SecondaryAction::Translate.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "translate")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.translate_selection(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::RecordArea.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordArea),
                                            locale.text(UiText::OverlayRecordArea),
                                            SecondaryAction::RecordArea.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "record-area")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_region_recording(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .child(secondary_action_button(
                                            SecondaryAction::RecordWindow.id(),
                                            secondary_navigation
                                                .for_action(SecondaryAction::RecordWindow),
                                            locale.text(UiText::OverlayRecordWindow),
                                            SecondaryAction::RecordWindow.width(),
                                            workspace_colors,
                                            false,
                                            Some(secondary_action_tooltip(locale, "record-window")),
                                            cx.listener(|this, _, _, cx| {
                                                let app = this.app.clone();
                                                cx.defer(move |cx| {
                                                    app.update(cx, |app, cx| {
                                                        app.start_selected_window_recording(cx)
                                                    })
                                                });
                                            }),
                                        ))
                                        .when(recognition_in_flight, |actions| {
                                            actions.child(
                                                div()
                                                    .id("overlay-recognition-progress")
                                                    .w_full()
                                                    .h(px(OVERLAY_RECOGNITION_STATUS_HEIGHT))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .rounded_sm()
                                                    .bg(workspace_colors.toolbar_surface)
                                                    .text_xs()
                                                    .text_color(workspace_colors.overlay_muted)
                                                    .child(
                                                        locale.text(
                                                            UiText::OverlayRecognizingSelection,
                                                        ),
                                                    ),
                                            )
                                        })
                                        .when_some(recognition_retry, |actions, retry| {
                                            let retry_label =
                                                recognition_retry_label(locale, retry);
                                            actions.child(secondary_action_button(
                                                SecondaryAction::RetryRecognition.id(),
                                                secondary_navigation
                                                    .for_action(SecondaryAction::RetryRecognition),
                                                retry_label,
                                                SecondaryAction::RetryRecognition.width(),
                                                workspace_colors,
                                                true,
                                                Some(
                                                    locale.text(
                                                        UiText::OverlayRetryRecognitionTooltip,
                                                    ),
                                                ),
                                                cx.listener(move |this, _, window, cx| {
                                                    let app = this.app.clone();
                                                    cx.defer(move |cx| {
                                                        app.update(cx, |app, cx| {
                                                            app.retry_recognition(retry, cx)
                                                        })
                                                    });
                                                    this.focus_secondary_action_after_render(
                                                        SecondaryAction::SaveAnnotations,
                                                        window,
                                                        cx,
                                                    );
                                                }),
                                            ))
                                        })
                                        .when_some(recognition_result, |actions, result| {
                                            actions
                                                .child(
                                                    div()
                                                        .id("overlay-recognition-preview")
                                                        .w_full()
                                                        .h(px(OVERLAY_RECOGNITION_PREVIEW_HEIGHT))
                                                        .p_2()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .overflow_hidden()
                                                        .border_1()
                                                        .border_color(rgba(0xFFFFFF24))
                                                        .bg(rgba(0x0B0D10E6))
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(
                                                                    workspace_colors.overlay_muted,
                                                                )
                                                                .child(result.title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .text_color(
                                                                    workspace_colors.overlay_text,
                                                                )
                                                                .child(recognition_result_preview(
                                                                    &result.text,
                                                                )),
                                                        ),
                                                )
                                                .child(secondary_action_button(
                                                    SecondaryAction::CopyRecognition.id(),
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::CopyRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayCopyText),
                                                    SecondaryAction::CopyRecognition.width(),
                                                    workspace_colors,
                                                    false,
                                                    Some(
                                                        locale.text(UiText::OverlayCopyTextTooltip),
                                                    ),
                                                    cx.listener(|this, _, _, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.copy_recognition_result(cx)
                                                            })
                                                        });
                                                    }),
                                                ))
                                                .child(secondary_action_button(
                                                    SecondaryAction::ClearRecognition.id(),
                                                    secondary_navigation.for_action(
                                                        SecondaryAction::ClearRecognition,
                                                    ),
                                                    locale.text(UiText::OverlayClearResult),
                                                    SecondaryAction::ClearRecognition.width(),
                                                    workspace_colors,
                                                    false,
                                                    Some(
                                                        locale.text(
                                                            UiText::OverlayClearResultTooltip,
                                                        ),
                                                    ),
                                                    cx.listener(|this, _, window, cx| {
                                                        let app = this.app.clone();
                                                        cx.defer(move |cx| {
                                                            app.update(cx, |app, cx| {
                                                                app.clear_recognition_result(cx)
                                                            })
                                                        });
                                                        this.focus_secondary_action_after_render(
                                                            SecondaryAction::SaveAnnotations,
                                                            window,
                                                            cx,
                                                        );
                                                    }),
                                                ))
                                        }),
                                )
                            })
                    })
                    .when(!has_selection, |actions| {
                        actions.child(
                            div()
                                .id("overlay-cancel")
                                .px_3()
                                .py_2()
                                .bg(colors.panel)
                                .text_color(colors.text)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let app = this.app.clone();
                                    cx.defer(move |cx| app.update(cx, |app, cx| app.reset(cx)));
                                }))
                                .child(locale.text(UiText::OverlayCancel)),
                        )
                    }),
            )
    }
}
