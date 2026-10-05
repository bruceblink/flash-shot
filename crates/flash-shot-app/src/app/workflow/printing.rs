//! Capture and Pin printing lifecycle.

use super::*;

impl FlashShotApp {
    /// Opens the Windows printer chooser for the current annotated selection.
    ///
    /// The immutable image snapshot is prepared off-thread. Cancel and failure return the session
    /// to Selecting; a submitted job completes and closes only the capture generation that opened
    /// the dialog. Capture and Pin windows restore their input and z-order after every outcome.
    pub(in crate::app) fn print_selection(&mut self, owner_hwnd: isize, cx: &mut Context<Self>) {
        let locale = self.settings.locale;
        if self.print_in_flight {
            self.status = locale.text(crate::i18n::UiText::PrintBusy).to_owned();
            cx.notify();
            return;
        }

        let selection = match self.session.start_export() {
            Ok(selection) => selection,
            Err(error) => {
                let detail = error.to_string();
                self.status = locale
                    .format_template(crate::i18n::UiText::PrintStartFailed, &[("error", &detail)]);
                cx.notify();
                return;
            }
        };
        if !self.try_begin_print_job() {
            let _ = self.session.export_cancelled();
            self.status = locale.text(crate::i18n::UiText::PrintBusy).to_owned();
            cx.notify();
            return;
        }
        let Some((frame, document)) = self.export_source() else {
            let _ = self.session.export_cancelled();
            self.print_in_flight = false;
            self.status = locale.format_template(
                crate::i18n::UiText::PrintStartFailed,
                &[(
                    "error",
                    locale.text(crate::i18n::UiText::PrintSnapshotUnavailable),
                )],
            );
            cx.notify();
            return;
        };
        if owner_hwnd == 0 {
            let _ = self.session.export_cancelled();
            self.print_in_flight = false;
            self.status = locale.format_template(
                crate::i18n::UiText::PrintFailed,
                &[(
                    "error",
                    locale.text(crate::i18n::UiText::PrintWindowHandleUnavailable),
                )],
            );
            cx.notify();
            return;
        }
        let dialog_windows = match self.demote_print_windows_for_dialog(owner_hwnd, None, cx) {
            Ok(handles) => handles,
            Err(error) => {
                let _ = self.session.export_cancelled();
                self.print_in_flight = false;
                let detail = error.to_string();
                self.status =
                    locale.format_template(crate::i18n::UiText::PrintFailed, &[("error", &detail)]);
                cx.notify();
                return;
            }
        };

        self.status = locale
            .text(crate::i18n::UiText::PrintSelectionInProgress)
            .to_owned();
        let generation = self.operation_generation;
        cx.notify();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        frame
                            .composite_annotations(&document)?
                            .crop(selection)
                            .and_then(|snapshot| {
                                crate::platform::printing::print_image(owner_hwnd, snapshot)
                            })
                    })
                    .await;
                Self::restore_print_windows_after_dialog(&dialog_windows);
                if let Some(this) = this.upgrade() {
                    this.update(&mut cx, |this, cx| {
                        this.finish_print_selection(result, generation, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// Reserves the app-wide printer slot shared by Capture and every Pin window.
    pub(in crate::app) fn try_begin_print_job(&mut self) -> bool {
        if self.print_in_flight || self.delayed_capture_generation.is_some() {
            return false;
        }
        self.print_in_flight = true;
        true
    }

    /// Releases the shared printer slot when a Pin-owned print dialog has completed.
    pub(in crate::app) fn finish_print_job(&mut self, cx: &mut Context<Self>) {
        self.print_in_flight = false;
        cx.notify();
    }

    fn finish_print_selection(
        &mut self,
        result: std::io::Result<crate::platform::printing::PrintOutcome>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        self.print_in_flight = false;
        if !is_current_operation(self.operation_generation, generation) {
            cx.notify();
            return;
        }

        let locale = self.settings.locale;
        match result {
            Ok(crate::platform::printing::PrintOutcome::Submitted) => {
                self.status = match self.session.export_completed() {
                    Ok(()) => locale.text(crate::i18n::UiText::PrintSubmitted).to_owned(),
                    Err(error) => {
                        let detail = error.to_string();
                        locale.format_template(
                            crate::i18n::UiText::PrintSubmittedTransitionFailed,
                            &[("error", &detail)],
                        )
                    }
                };
                self.close_capture_overlays(cx);
                self.return_to_background();
            }
            Ok(crate::platform::printing::PrintOutcome::Cancelled) => {
                if let Err(error) = self.session.export_cancelled() {
                    let detail = error.to_string();
                    self.status = locale
                        .format_template(crate::i18n::UiText::PrintFailed, &[("error", &detail)]);
                } else {
                    self.status = locale.text(crate::i18n::UiText::PrintCancelled).to_owned();
                }
            }
            Err(error) => {
                let detail = error.to_string();
                self.status =
                    locale.format_template(crate::i18n::UiText::PrintFailed, &[("error", &detail)]);
                if let Err(transition) = self.session.export_cancelled() {
                    self.status.push_str("; ");
                    self.status.push_str(&transition.to_string());
                }
            }
        }
        cx.notify();
    }

    /// Lowers and disables every owned Capture/Pin window except the modal-dialog owner.
    ///
    /// A print dialog is owned by one surface, while the other Flash Shot surfaces are independent
    /// top-level windows. Temporarily returning all of them to ordinary z-order and disabling
    /// sibling input prevents them from covering the chooser or accepting actions mid-print.
    #[cfg(windows)]
    pub(in crate::app) fn demote_print_windows_for_dialog(
        &self,
        owner_hwnd: isize,
        current_pin: Option<gpui::WindowHandle<PinnedImage>>,
        cx: &mut Context<Self>,
    ) -> std::io::Result<super::super::PrintWindowState> {
        if owner_hwnd == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "print owner window handle is null",
            ));
        }

        let mut handles = self.capture_overlay_native_handles(cx)?;
        for pinned in &self.pinned_windows {
            if current_pin
                .as_ref()
                .is_some_and(|current| pinned == current)
            {
                continue;
            }
            let handle = pinned
                .update(cx, |_, window, _| {
                    crate::app::pinned::native_window_handle(window).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            "pinned window does not expose a Win32 HWND",
                        )
                    })
                })
                .map_err(|error| std::io::Error::other(error.to_string()))??;
            handles.push(handle);
        }
        if !handles.contains(&owner_hwnd) {
            handles.push(owner_hwnd);
        }
        handles.dedup();

        let mut state = super::super::PrintWindowState::default();
        for handle in handles {
            if let Err(error) = window_visibility::make_not_topmost(handle) {
                Self::restore_print_windows_after_dialog(&state);
                return Err(error);
            }
            state.topmost_windows.push(handle);
        }
        for handle in state.topmost_windows.iter().copied() {
            if handle == owner_hwnd {
                continue;
            }
            match window_visibility::is_enabled(handle) {
                Ok(true) => {
                    if let Err(error) = window_visibility::set_enabled(handle, false) {
                        Self::restore_print_windows_after_dialog(&state);
                        return Err(error);
                    }
                    state.previously_enabled_windows.push(handle);
                }
                Ok(false) => {}
                Err(error) => {
                    Self::restore_print_windows_after_dialog(&state);
                    return Err(error);
                }
            }
        }
        Ok(state)
    }

    /// Leaves non-Windows builds with no native handles to restore.
    #[cfg(not(windows))]
    pub(in crate::app) fn demote_print_windows_for_dialog(
        &self,
        _owner_hwnd: isize,
        _current_pin: Option<gpui::WindowHandle<PinnedImage>>,
        _cx: &mut Context<Self>,
    ) -> std::io::Result<super::super::PrintWindowState> {
        Ok(super::super::PrintWindowState::default())
    }

    /// Restores sibling input and topmost ordering after the system printer call has returned.
    pub(in crate::app) fn restore_print_windows_after_dialog(
        state: &super::super::PrintWindowState,
    ) {
        for handle in &state.topmost_windows {
            if let Err(error) = window_visibility::make_topmost(*handle) {
                log::warn!(target: "flash_shot::overlay", "print_window_topmost_restore_failed error={error}");
            }
        }
        for handle in &state.previously_enabled_windows {
            if let Err(error) = window_visibility::set_enabled(*handle, true) {
                log::warn!(target: "flash_shot::overlay", "print_window_input_restore_failed error={error}");
            }
        }
    }
}
