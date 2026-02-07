//! Split-pane management on App.

use super::*;

impl App {
    /// Update the pane state caches from the session manager.
    pub fn update_pane_state_caches(&mut self) {
        for pane_id in [TerminalPaneId::Primary, TerminalPaneId::Secondary] {
            let idx = pane_id.index();
            if let Some(ref session_id) = self.panes[idx].session_id {
                self.pane_state_caches[idx] =
                    self.session_manager.get_session_state(session_id);
            } else {
                self.pane_state_caches[idx] = None;
            }
        }
    }
}
