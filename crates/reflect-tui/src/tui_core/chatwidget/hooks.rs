use std::path::PathBuf;

use super::ChatWidget;
use crate::app_server_protocol::HooksListEntry;
use crate::app_server_protocol::HooksListResponse;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::bottom_pane::HooksBrowserView;
use crate::tui_core::hooks_rpc::hooks_list_entry_for_cwd;

impl ChatWidget {
    pub(crate) fn add_hooks_output(&mut self) {
        self.app_event_tx.send(AppEvent::FetchHooksList {
            cwd: self.config.cwd.to_path_buf(),
        });
    }

    pub(crate) fn on_hooks_loaded(
        &mut self,
        cwd: PathBuf,
        result: Result<HooksListResponse, String>,
    ) {
        if self.config.cwd.as_path() != cwd.as_path() {
            return;
        }

        match result {
            Ok(response) => {
                self.open_hooks_browser(hooks_list_entry_for_cwd(response, &cwd));
            }
            Err(err) => self.add_error_message(format!("Failed to load hooks: {err}")),
        }
    }

    pub(crate) fn open_hooks_browser(&mut self, entry: HooksListEntry) {
        self.bottom_pane
            .show_view(Box::new(HooksBrowserView::from_entry(
                entry,
                self.app_event_tx.clone(),
                self.bottom_pane.list_keymap(),
            )));
        self.request_redraw();
    }
}
