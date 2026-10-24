use rusqlite::Connection;

pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    super::in_tx(conn, |conn| {
        // Preserve both old configuration snapshots. Only the active native binding
        // replaces a pre-existing ACP snapshot, so resume follows the conversation
        // the user was actually using rather than an older ACP experiment.
        conn.execute_batch(
            "INSERT INTO app_session_role_runtime_configs
             (app_session_id,role_name,runtime_kind,runtime_profile_id,acp_session_id,model_override,mode_override,config_options_json,updated_at,transcript_cursor)
             SELECT app_session_id,role_name,'claude-code','acp:claude-code',acp_session_id,model_override,mode_override,config_options_json,updated_at,transcript_cursor
             FROM app_session_role_runtime_configs WHERE runtime_kind='claude-native'
             ON CONFLICT(app_session_id,role_name,runtime_kind) DO UPDATE SET
               acp_session_id=excluded.acp_session_id, model_override=excluded.model_override,
               mode_override=excluded.mode_override, config_options_json=excluded.config_options_json,
               updated_at=excluded.updated_at, transcript_cursor=excluded.transcript_cursor
             WHERE EXISTS (SELECT 1 FROM app_session_roles r
               WHERE r.app_session_id=excluded.app_session_id AND r.role_name=excluded.role_name AND r.runtime_kind='claude-native');
             INSERT OR IGNORE INTO project_agent_runtime_configs
             (project_id,role_name,runtime_kind,config_options_json,updated_at)
             SELECT project_id,role_name,'claude-code',config_options_json,updated_at
             FROM project_agent_runtime_configs WHERE runtime_kind='claude-native';
             UPDATE app_sessions SET external_session_id=NULL
             WHERE runtime_kind='claude-native' AND external_session_id IS NOT NULL
               AND EXISTS(SELECT 1 FROM app_sessions c WHERE c.runtime_kind='claude-code' AND c.external_session_id=app_sessions.external_session_id);",
        ).map_err(|e| e.to_string())?;
        for table in [
            "roles",
            "app_sessions",
            "app_session_roles",
            "project_agent_configs",
        ] {
            conn.execute(&format!("UPDATE {table} SET runtime_kind='claude-code' WHERE runtime_kind='claude-native'"), []).map_err(|e| e.to_string())?;
        }
        for table in [
            "roles",
            "app_sessions",
            "app_session_roles",
            "app_session_role_runtime_configs",
        ] {
            conn.execute(&format!("UPDATE {table} SET runtime_profile_id=CASE runtime_kind WHEN 'claude-code' THEN 'acp:claude-code' WHEN 'codex-cli' THEN 'acp:codex' WHEN 'pi-cli' THEN 'acp:pi' WHEN 'antigravity-cli' THEN 'acp:agy' ELSE runtime_profile_id END WHERE runtime_kind IN ('claude-code','codex-cli','pi-cli','antigravity-cli')"), []).map_err(|e| e.to_string())?;
        }
        // Previous role create/edit UI hardcoded autoApprove=true, so it was not
        // an explicit grant. The control-center migration restores manual decisions.
        conn.execute("UPDATE roles SET auto_approve=0", [])
            .map_err(|e| e.to_string())?;
        conn.execute("UPDATE project_role_overrides SET overrides_json=json_set(overrides_json, '$.autoApprove', json('false')) WHERE json_valid(overrides_json) AND json_extract(overrides_json, '$.autoApprove')=1", []).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "DELETE FROM import_session_catalog; DELETE FROM import_session_catalog_scans;",
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_keeps_transcript_and_active_provider_handle() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute_batch("INSERT INTO app_sessions(id,title,active_role,runtime_kind,created_at,last_active_at) VALUES ('s','Session','Developer','claude-native',1,1);
            INSERT INTO app_session_roles(app_session_id,role_name,runtime_kind,acp_session_id) VALUES ('s','Developer','claude-native','active-handle');
            INSERT INTO app_session_role_runtime_configs(app_session_id,role_name,runtime_kind,acp_session_id,updated_at) VALUES ('s','Developer','claude-native','active-handle',20),('s','Developer','claude-code','older-acp-handle',10);
            INSERT INTO app_session_messages(session_id,role_name,content,created_at) VALUES ('s','user','preserve me',1);").unwrap();
        conn.execute("UPDATE roles SET auto_approve=1", []).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM roles WHERE auto_approve=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT runtime_kind FROM app_sessions WHERE id='s'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "claude-code"
        );
        assert_eq!(conn.query_row("SELECT acp_session_id FROM app_session_role_runtime_configs WHERE app_session_id='s' AND runtime_kind='claude-code'", [], |r| r.get::<_,String>(0)).unwrap(), "active-handle");
        assert_eq!(
            conn.query_row(
                "SELECT content FROM app_session_messages WHERE session_id='s'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preserve me"
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM app_session_role_runtime_configs WHERE app_session_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        migrate(&conn).unwrap();
    }
}
