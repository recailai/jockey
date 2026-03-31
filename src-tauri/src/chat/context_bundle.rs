use crate::db::get_state;
use crate::db::skill::load_skills_by_names;
use crate::fs_context::{attach_dir_context, attach_file_context};
use crate::resolve_chat_cwd;
use crate::types::{AppState, ParsedRouteInput, ATTACH_MAX_TOTAL_BYTES};
use tauri::State;

use super::{load_recent_role_chats, RecentRoleChat};

pub(super) struct ContextBundle {
    pub(super) cwd: String,
    pub(super) attachment_pairs: Vec<(String, String)>,
    pub(super) attach_notes: Vec<String>,
    pub(super) skill_pairs: Vec<(String, String)>,
    pub(super) recent_chats: Vec<RecentRoleChat>,
}

pub(super) async fn build_context_bundle(
    state: &State<'_, AppState>,
    app_session_id: &str,
    routed: &ParsedRouteInput,
) -> ContextBundle {
    let cwd = crate::db::app_session::get_app_session_cwd(get_state(state), app_session_id)
        .unwrap_or_else(resolve_chat_cwd);

    let mut attachment_pairs: Vec<(String, String)> = Vec::new();
    let mut attach_budget = ATTACH_MAX_TOTAL_BYTES;
    let mut attach_notes = Vec::new();

    {
        let per_file_budget =
            attach_budget / (routed.file_refs.len() + routed.dir_refs.len()).max(1);
        let per_file_budget = per_file_budget.min(ATTACH_MAX_TOTAL_BYTES);
        let file_futs: Vec<_> = routed
            .file_refs
            .iter()
            .map(|r| attach_file_context(cwd.clone(), r.clone(), per_file_budget))
            .collect();
        let dir_futs: Vec<_> = routed
            .dir_refs
            .iter()
            .map(|r| attach_dir_context(cwd.clone(), r.clone(), per_file_budget))
            .collect();
        let (file_results, dir_results) = tokio::join!(
            futures::future::join_all(file_futs),
            futures::future::join_all(dir_futs)
        );
        for result in file_results.into_iter().chain(dir_results) {
            if attach_budget == 0 {
                attach_notes.push("attachment budget reached; some files skipped".to_string());
                break;
            }
            match result {
                Ok((key, value, used)) => {
                    attachment_pairs.push((key, value));
                    attach_budget = attach_budget.saturating_sub(used);
                }
                Err(e) => attach_notes.push(e),
            }
        }
    }

    let db_pool = get_state(state).db.clone();
    let shared_ctx = get_state(state).shared_context.clone();
    let skill_refs = routed.skill_refs.clone();
    let skill_pool = db_pool.clone();
    let skill_ctx = shared_ctx.clone();
    let skill_pairs: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
        let tmp_state = AppState {
            db: skill_pool,
            shared_context: skill_ctx,
        };
        load_skills_by_names(&tmp_state, &skill_refs)
    })
    .await
    .unwrap_or_default()
    .into_iter()
    .filter(|s| !s.content.is_empty())
    .map(|s| (format!("skill:{}", s.name), s.content))
    .collect();

    let pre_pool = db_pool.clone();
    let pre_ctx = shared_ctx.clone();
    let pre_app_session_id = app_session_id.to_string();
    let recent_chats: Vec<RecentRoleChat> = tokio::task::spawn_blocking(move || {
        let tmp = AppState {
            db: pre_pool,
            shared_context: pre_ctx,
        };
        load_recent_role_chats(&tmp, &pre_app_session_id)
    })
    .await
    .unwrap_or_default();

    ContextBundle {
        cwd,
        attachment_pairs,
        attach_notes,
        skill_pairs,
        recent_chats,
    }
}
