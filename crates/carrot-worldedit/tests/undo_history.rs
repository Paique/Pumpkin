//! Undo regression loop for the reported symptom: `//set dirt` applies
//! 575k blocks, `//undo` answers "Desfeitas 0 edição(ões)".
//!
//! Reproduces the exact production wiring: `WeState::configure` on the
//! module data folder, one session, `history.record` with the screenshot's
//! change volume, then `pop_undo` — both on the live session and on a
//! freshly opened one (server restart keeps history on disk).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use carrot_worldedit::history::Change;
use carrot_worldedit::session::WeState;
use carrot_worldedit::states::parse_state_str;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// The `//set dirt` from the report: 575,282 applied blocks over the
/// screenshot's region (x -5429..-5358, z 3091..3111, y -64..319).
const CHANGE_COUNT: usize = 575_282;

fn screenshot_changes() -> Vec<Change> {
    let before: BlockStateId = parse_state_str("stone").expect("valid state");
    let after: BlockStateId = parse_state_str("dirt").expect("valid state");
    let mut changes = Vec::with_capacity(CHANGE_COUNT);
    for x in -5429..=-5358 {
        for z in 3091..=3111 {
            for y in -64..=319 {
                if changes.len() == CHANGE_COUNT {
                    return changes;
                }
                changes.push((BlockPos(Vector3::new(x, y, z)), before, after));
            }
        }
    }
    changes
}

#[test]
fn undo_serves_a_575k_edit_on_the_live_session() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let state = Arc::new(WeState::new());
    state.configure(&temp.path().join("modules").join("worldedit"));

    let uuid = uuid::Uuid::new_v4();
    let changes = screenshot_changes();
    let session = state.session(uuid);
    {
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .history
            .record(&changes)
            .expect("history record must succeed");
    }

    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(guard.history.undo_depth(), 1, "edit must be undoable");
    let popped = guard
        .history
        .pop_undo()
        .expect("undo entry must exist after a recorded edit");
    assert_eq!(popped.len(), CHANGE_COUNT);
    assert_eq!(popped[0], changes[0]);
}

#[test]
fn undo_serves_a_575k_edit_after_reopening_the_session() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let data_folder = temp.path().join("modules").join("worldedit");
    let uuid = uuid::Uuid::new_v4();
    let changes = screenshot_changes();

    {
        let state = Arc::new(WeState::new());
        state.configure(&data_folder);
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .history
            .record(&changes)
            .expect("history record must succeed");
        // `WeState` dropped here — the server restarted.
    }

    let state = Arc::new(WeState::new());
    state.configure(&data_folder);
    let session = state.session(uuid);
    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let popped = guard
        .history
        .pop_undo()
        .expect("disk history must survive a restart");
    assert_eq!(popped.len(), CHANGE_COUNT);
    assert_eq!(popped[0], changes[0]);
}

#[test]
fn undo_survives_a_module_reenable() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let data_folder = temp.path().join("modules").join("worldedit");
    let uuid = uuid::Uuid::new_v4();
    let changes = screenshot_changes();

    let state = Arc::new(WeState::new());
    state.configure(&data_folder);
    {
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .history
            .record(&changes)
            .expect("history record must succeed");
    }

    // `/carrot modules worldedit disable` + `enable`: sessions are dropped
    // and rebuilt, history files stay on disk.
    state.clear_transient();
    let session = state.session(uuid);
    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let popped = guard
        .history
        .pop_undo()
        .expect("history must survive a module re-enable");
    assert_eq!(popped.len(), CHANGE_COUNT);
}
