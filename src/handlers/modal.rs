use anyhow::Result;
use crossterm::event::KeyEvent;

use crate::app::App;
use crate::ui::modal::ModalKeyResult;

/// Forward a key event to the currently open modal
pub(crate) fn forward_key_to_modal(app: &mut App, key: KeyEvent) -> Result<()> {
    let result = if let Some(modal) = app.modal_state.as_modal_mut() {
        modal.handle_key_modal(key)
    } else {
        return Ok(());
    };

    match result {
        ModalKeyResult::Continue => {}
        ModalKeyResult::Close => {
            app.close_modal();
        }
        ModalKeyResult::PathSelected(path) => {
            app.confirm_new_project(&path)?;
        }
        ModalKeyResult::SearchSelected(session_id) => {
            app.navigate_to_conversation(&session_id)?;
        }
        ModalKeyResult::SearchQueryChanged => {
            app.perform_search();
        }
        ModalKeyResult::BranchSelected(branch_name) => {
            app.confirm_worktree(&branch_name)?;
        }
        ModalKeyResult::WorktreeSearchConfirmed {
            project_path,
            branch_name,
        } => {
            app.confirm_worktree_search(&project_path, &branch_name)?;
        }
        ModalKeyResult::WorktreeSearchQueryChanged => {}
    }
    Ok(())
}
