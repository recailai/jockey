use rusqlite::{params, Connection};

/// Clears what the v17 ACP migration left behind. Old `claude-native` runtime rows kept the
/// provider handle their `claude-code` successor took over, so the session watcher imported
/// those transcripts twice; and ACP turns recorded before the transcript fix stored usage frames
/// and a second copy of the reply as segments.
pub(super) fn migrate(conn: &Connection) -> Result<(), String> {
    super::in_tx(conn, |conn| {
        conn.execute_batch(
            "DELETE FROM app_session_messages
             WHERE client_id LIKE 'import:claude-native:%'
               AND EXISTS (SELECT 1 FROM app_session_messages c
                 WHERE c.session_id = app_session_messages.session_id
                   AND c.client_id = 'import:claude-code:' || substr(app_session_messages.client_id, 22));
             UPDATE app_session_messages
             SET client_id = 'import:claude-code:' || substr(client_id, 22)
             WHERE client_id LIKE 'import:claude-native:%';",
        )
        .map_err(|e| e.to_string())?;
        for (table, keys) in [
            (
                "app_session_role_runtime_configs",
                "app_session_id, role_name",
            ),
            ("project_agent_runtime_configs", "project_id, role_name"),
            ("agent_lifecycle", "app_session_id, role_name"),
        ] {
            conn.execute(
                &format!(
                    "DELETE FROM {table} WHERE runtime_kind = 'claude-native'
                     AND ({keys}) IN (SELECT {keys} FROM {table} WHERE runtime_kind = 'claude-code')"
                ),
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                &format!(
                    "UPDATE {table} SET runtime_kind = 'claude-code' WHERE runtime_kind = 'claude-native'"
                ),
                [],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute(
            "UPDATE app_session_role_runtime_configs SET runtime_profile_id = 'acp:claude-code'
             WHERE runtime_kind = 'claude-code' AND runtime_profile_id IS NOT 'acp:claude-code'",
            [],
        )
        .map_err(|e| e.to_string())?;
        repair_message_segments(conn)
    })
}

fn repair_message_segments(conn: &Connection) -> Result<(), String> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, payload FROM app_session_messages
                 WHERE payload IS NOT NULL AND payload LIKE '%\"segments\"%'",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    for (id, payload) in rows {
        let Ok(mut msg) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        if crate::engine::transcript::repair_stored_segments(&mut msg) {
            conn.execute(
                "UPDATE app_session_messages SET payload = ?1 WHERE id = ?2",
                params![msg.to_string(), id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn legacy_rows_and_duplicate_imports_are_folded_into_claude_code() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO app_sessions(id,title,active_role,runtime_kind,created_at,last_active_at)
               VALUES ('s','S','Dev','claude-code',1,1);
             INSERT INTO app_session_role_runtime_configs(app_session_id,role_name,runtime_kind,acp_session_id,updated_at)
               VALUES ('s','Dev','claude-code','h',1),('s','Dev','claude-native','h',1),
                      ('s','Old','claude-native','h2',1);
             INSERT INTO app_session_messages(session_id,role_name,content,created_at,client_id) VALUES
               ('s','user','q',1,'import:claude-code:h:0'),
               ('s','user','q',1,'import:claude-native:h:0'),
               ('s','Dev','a',2,'import:claude-native:h:1'),
               ('s','Dev','x',3,'turn-1');",
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            count(&conn, "SELECT count(*) FROM app_session_messages WHERE client_id LIKE 'import:claude-native:%'"),
            0
        );
        assert_eq!(count(&conn, "SELECT count(*) FROM app_session_messages"), 3);
        assert_eq!(
            count(&conn, "SELECT count(*) FROM app_session_messages WHERE client_id = 'import:claude-code:h:1'"),
            1
        );
        assert_eq!(
            count(&conn, "SELECT count(*) FROM app_session_role_runtime_configs WHERE runtime_kind = 'claude-native'"),
            0
        );
        assert_eq!(
            count(&conn, "SELECT count(*) FROM app_session_role_runtime_configs WHERE role_name = 'Old' AND runtime_profile_id = 'acp:claude-code'"),
            1
        );
        migrate(&conn).unwrap();
    }
}
