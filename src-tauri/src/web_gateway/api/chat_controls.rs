use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

use super::{bad_request, internal, ApiResult, ProjectFilter};
use crate::{commands::runtime_cmd as runtime, types::AppState, web_gateway::ServerContext};

pub(super) fn router() -> Router<ServerContext> {
    Router::new()
        .route("/agent/config", get(config).post(set_config))
        .route("/agent/pins", get(pins))
        .route("/agent/runtime", post(set_runtime))
        .route("/sessions/:id/agent/bind", post(bind))
        .route("/sessions/:id/agent/discover", post(discover))
        .route("/sessions/:id/agent/commands", get(commands))
        .route("/sessions/:id/agent/mode", post(mode))
        .route("/sessions/:id/mentions", get(mentions))
        .route("/skills", get(skills))
        .route("/roles/:id/rules", get(role_rules))
        .route("/roles/:id/skills", get(role_skills))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRequest {
    project_id: Option<String>,
    role_name: String,
    runtime_kind: Option<String>,
    force: Option<bool>,
}

async fn config(
    State(ctx): State<ServerContext>,
    Query(body): Query<AgentRequest>,
) -> ApiResult<Value> {
    runtime::get_project_agent_config_cmd(
        ctx.app.state::<AppState>(),
        body.project_id,
        body.role_name,
    )
    .map(Json)
    .map_err(bad_request)
}

async fn pins(
    State(ctx): State<ServerContext>,
    Query(filter): Query<ProjectFilter>,
) -> ApiResult<Value> {
    runtime::list_project_agent_pins_cmd(ctx.app.state::<AppState>(), filter.project_id)
        .map(Json)
        .map_err(bad_request)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigRequest {
    project_id: Option<String>,
    role_name: String,
    runtime_kind: Option<String>,
    config_id: String,
    value: String,
}

async fn set_config(
    State(ctx): State<ServerContext>,
    Json(body): Json<ConfigRequest>,
) -> ApiResult<Value> {
    runtime::set_project_agent_config_cmd(
        ctx.app.state::<AppState>(),
        body.project_id,
        body.role_name,
        body.config_id,
        body.value,
        body.runtime_kind,
    )
    .map(Json)
    .map_err(bad_request)
}

async fn set_runtime(
    State(ctx): State<ServerContext>,
    Json(body): Json<AgentRequest>,
) -> ApiResult<Value> {
    runtime::set_project_agent_runtime_cmd(
        ctx.app.state::<AppState>(),
        body.project_id,
        body.role_name,
        body.runtime_kind
            .ok_or_else(|| bad_request("runtimeKind required"))?,
    )
    .map(Json)
    .map_err(bad_request)
}

async fn bind(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<AgentRequest>,
) -> ApiResult<String> {
    runtime::bind_session_agent_cmd(
        ctx.app.state::<AppState>(),
        id,
        body.role_name,
        body.runtime_kind
            .ok_or_else(|| bad_request("runtimeKind required"))?,
    )
    .map(Json)
    .map_err(bad_request)
}

async fn discover(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<AgentRequest>,
) -> ApiResult<Value> {
    runtime::prewarm_role_config_cmd(
        ctx.app.state::<AppState>(),
        body.role_name,
        id,
        body.project_id,
        body.force,
        body.runtime_kind,
    )
    .await
    .map(Json)
    .map_err(bad_request)
}

async fn commands(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Query(body): Query<AgentRequest>,
) -> ApiResult<Vec<Value>> {
    runtime::list_available_commands_cmd(
        ctx.app.state::<AppState>(),
        body.role_name,
        id,
        body.project_id,
    )
    .await
    .map(Json)
    .map_err(bad_request)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModeRequest {
    role_name: String,
    mode_id: String,
}

async fn mode(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Json(body): Json<ModeRequest>,
) -> ApiResult<()> {
    runtime::set_acp_mode(
        ctx.app.state::<AppState>(),
        body.role_name,
        body.mode_id,
        id,
    )
    .await
    .map(Json)
    .map_err(bad_request)
}

#[derive(Deserialize)]
struct MentionQuery {
    query: String,
    limit: Option<usize>,
}

async fn mentions(
    State(ctx): State<ServerContext>,
    Path(id): Path<String>,
    Query(body): Query<MentionQuery>,
) -> ApiResult<Value> {
    let rows = crate::commands::completion::complete_mentions(
        ctx.app.state::<AppState>(),
        body.query,
        body.limit,
        Some(id),
    )
    .await
    .map_err(bad_request)?;
    serde_json::to_value(rows)
        .map(Json)
        .map_err(|error| internal(error.to_string()))
}

async fn skills(State(ctx): State<ServerContext>) -> ApiResult<Value> {
    let rows = crate::db::skill::list_app_skills(ctx.app.state::<AppState>()).map_err(internal)?;
    serde_json::to_value(rows)
        .map(Json)
        .map_err(|error| internal(error.to_string()))
}

async fn role_rules(State(ctx): State<ServerContext>, Path(id): Path<String>) -> ApiResult<Value> {
    let rows = crate::db::rule::list_all_rules_for_role_cmd(ctx.app.state::<AppState>(), id)
        .map_err(internal)?;
    serde_json::to_value(rows)
        .map(Json)
        .map_err(|error| internal(error.to_string()))
}

async fn role_skills(State(ctx): State<ServerContext>, Path(id): Path<String>) -> ApiResult<Value> {
    let rows = crate::db::skill::list_all_skills_for_role_cmd(ctx.app.state::<AppState>(), id)
        .map_err(internal)?;
    serde_json::to_value(rows)
        .map(Json)
        .map_err(|error| internal(error.to_string()))
}
