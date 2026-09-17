use crate::db::with_db;
use crate::error::AppError;
use crate::now_ms;
use crate::types::{AppState, Project};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

pub(crate) fn list_projects_internal(state: &AppState) -> Result<Vec<Project>, String> {
    with_db(state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, root_path, created_at, updated_at \
                 FROM projects ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })
}

pub(crate) fn get_project_internal(state: &AppState, id: &str) -> Result<Option<Project>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, name, root_path, created_at, updated_at \
             FROM projects WHERE id = ?1",
            params![id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn find_project_by_path_internal(
    state: &AppState,
    root_path: &str,
) -> Result<Option<Project>, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id, name, root_path, created_at, updated_at \
             FROM projects WHERE root_path = ?1",
            params![root_path],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

pub(crate) fn create_project_internal(
    state: &AppState,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let name = name.trim().to_string();
    let root_path = root_path.trim().to_string();
    if name.is_empty() {
        return Err("project name cannot be empty".to_string());
    }
    if root_path.is_empty() {
        return Err("project root path cannot be empty".to_string());
    }

    // Canonicalize path if it exists
    let canonical_path = std::fs::canonicalize(&root_path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| root_path.clone());

    let now = now_ms();
    let existing = find_project_by_path_internal(state, &canonical_path)?;
    if let Some(mut proj) = existing {
        // Update updated_at and return existing
        with_db(state, |conn| {
            conn.execute(
                "UPDATE projects SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![&name, now, &proj.id],
            )
            .map_err(|e| e.to_string())
        })?;
        proj.name = name;
        proj.updated_at = now;
        return Ok(proj);
    }

    let id = Uuid::new_v4().to_string();
    let project = Project {
        id: id.clone(),
        name: name.clone(),
        root_path: canonical_path.clone(),
        created_at: now,
        updated_at: now,
    };

    with_db(state, |conn| {
        conn.execute(
            "INSERT INTO projects (id, name, root_path, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, canonical_path, now, now],
        )
        .map_err(|e| AppError::db(e.to_string()).to_string())?;
        Ok(())
    })?;

    Ok(project)
}

pub(crate) fn delete_project_internal(state: &AppState, id: &str) -> Result<(), String> {
    with_db(state, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE app_sessions SET project_id = NULL WHERE project_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM project_agent_runtime_configs WHERE project_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM project_agent_configs WHERE project_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM import_session_catalog WHERE project_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM import_session_catalog_scans WHERE project_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM roles WHERE project_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        let deleted = tx
            .execute("DELETE FROM projects WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        if deleted == 0 {
            return Err(format!("project not found: {id}"));
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })?;
    state.role_cache.clear();
    Ok(())
}

pub(crate) fn ensure_project_exists(
    conn: &rusqlite::Connection,
    project_id: Option<&str>,
) -> Result<(), String> {
    let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let table_exists: bool = conn
        .query_row(
            "SELECT count(1) FROM sqlite_master WHERE type='table' AND name='projects'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !table_exists {
        return Ok(());
    }
    let exists: i64 = conn
        .query_row(
            "SELECT count(1) FROM projects WHERE id = ?1",
            params![pid],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err(format!("project not found: {pid}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_db, pool::DbPool};
    use dashmap::DashMap;
    use std::sync::Arc;

    fn test_state(dir: &tempfile::TempDir) -> AppState {
        let pool = DbPool::new(
            dir.path().join("test.sqlite3"),
            2,
            "PRAGMA foreign_keys = ON;",
        )
        .expect("pool");
        {
            let conn = pool.get().expect("conn");
            init_db(&conn).expect("init_db");
        }
        AppState {
            db: pool,
            role_cache: Arc::new(DashMap::new()),
        }
    }

    #[test]
    fn deleting_a_project_removes_scoped_configuration_and_detaches_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let project =
            create_project_internal(&state, "Project".to_string(), "/tmp/project".to_string())
                .expect("create project");
        let session = crate::db::app_session::create_app_session_internal(
            &state,
            Some("session"),
            Some(&project.id),
            None,
            None,
            None,
        )
        .expect("create project session");

        crate::db::with_db(&state, |conn| {
            conn.execute(
                "INSERT INTO global_mcp_servers (name, config_json, is_builtin, created_at, updated_at)
                 VALUES ('project-test', '{}', 0, 1, 1)",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO roles (id, role_name, runtime_kind, system_prompt, project_id, created_at, updated_at)
                 VALUES ('project-role', 'Developer', 'claude-native', '', ?1, 1, 1)",
                params![&project.id],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO role_mcp_servers (role_id, mcp_server_name, enabled)
                 VALUES ('project-role', 'project-test', 1)",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO project_agent_runtime_configs (project_id, role_name, runtime_kind, updated_at)
                 VALUES (?1, 'Developer', 'claude-native', 1)",
                params![&project.id],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO import_session_catalog_scans (project_id, scanned_at) VALUES (?1, 1)",
                params![&project.id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .expect("seed project data");

        delete_project_internal(&state, &project.id).expect("delete project");

        crate::db::with_db(&state, |conn| {
            let session_project: Option<String> = conn
                .query_row(
                    "SELECT project_id FROM app_sessions WHERE id = ?1",
                    params![&session.id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            assert_eq!(session_project, None);
            for (table, sql, param) in [
                (
                    "roles",
                    "SELECT count(1) FROM roles WHERE project_id = ?1",
                    project.id.as_str(),
                ),
                (
                    "role_mcp_servers",
                    "SELECT count(1) FROM role_mcp_servers WHERE role_id = ?1",
                    "project-role",
                ),
                (
                    "project_agent_runtime_configs",
                    "SELECT count(1) FROM project_agent_runtime_configs WHERE project_id = ?1",
                    project.id.as_str(),
                ),
                (
                    "import_session_catalog_scans",
                    "SELECT count(1) FROM import_session_catalog_scans WHERE project_id = ?1",
                    project.id.as_str(),
                ),
            ] {
                let count: i64 = conn
                    .query_row(sql, params![param], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                assert_eq!(count, 0, "{table} should not retain project data");
            }
            Ok(())
        })
        .expect("verify cleanup");
    }

    #[test]
    fn ensure_project_exists_validates_correctly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = test_state(&dir);
        let project = create_project_internal(
            &state,
            "Project".to_string(),
            "/tmp/project_val".to_string(),
        )
        .expect("create project");

        crate::db::with_db(&state, |conn| {
            assert!(ensure_project_exists(conn, None).is_ok());
            assert!(ensure_project_exists(conn, Some("")).is_ok());
            assert!(ensure_project_exists(conn, Some("  ")).is_ok());
            assert!(ensure_project_exists(conn, Some(&project.id)).is_ok());
            assert!(ensure_project_exists(conn, Some("non-existent-uuid")).is_err());
            Ok(())
        })
        .expect("checks pass");
    }
}
