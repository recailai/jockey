pub(crate) fn build_prepared_prompt(
    role_system_prompt: Option<&str>,
    enabled_rules: &[(String, String)],
    context_pairs: &[(String, String)],
    message: &str,
) -> String {
    // Slash commands must be passed raw to CLI/ACP so native commands like /cost, /compact execute directly.
    if message.starts_with('/') {
        return message.to_string();
    }

    let has_system = role_system_prompt
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_rules = !enabled_rules.is_empty();
    let has_context = !context_pairs.is_empty();

    // Native first: if no system prompt, rules, or extra context, pass raw message directly.
    if !has_system && !has_rules && !has_context {
        return message.to_string();
    }

    let mut prepared = String::new();
    if let Some(sp) = role_system_prompt {
        let trimmed = sp.trim();
        if !trimmed.is_empty() {
            prepared.push_str("System:\n");
            prepared.push_str(trimmed);
        }
    }

    for (name, content) in enabled_rules {
        if !prepared.is_empty() {
            prepared.push_str("\n\n");
        }
        prepared.push_str("Rule: ");
        prepared.push_str(name);
        prepared.push('\n');
        prepared.push_str(content);
    }

    if has_context {
        if !prepared.is_empty() {
            prepared.push_str("\n\n");
        }
        prepared.push_str("Context:\n");
        for (i, (k, v)) in context_pairs.iter().enumerate() {
            if i > 0 {
                prepared.push('\n');
            }
            prepared.push_str(k);
            prepared.push_str(": ");
            prepared.push_str(v);
        }
    }

    if !prepared.is_empty() {
        prepared.push_str("\n\nUser:\n");
    }
    prepared.push_str(message);
    prepared
}

pub(super) fn with_command_suggestion(output: String) -> String {
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slash_command_passes_through_raw() {
        let rules = vec![("rule1".to_string(), "content".to_string())];
        let context = vec![("cwd".to_string(), "/tmp".to_string())];
        let result = build_prepared_prompt(
            Some("You are a senior developer."),
            &rules,
            &context,
            "/cost",
        );
        assert_eq!(result, "/cost");
    }

    #[test]
    fn test_empty_system_prompt_emits_no_system_block() {
        let rules = vec![("formatting".to_string(), "be concise".to_string())];
        let context = vec![("cwd".to_string(), "/workspace".to_string())];
        let result = build_prepared_prompt(None, &rules, &context, "hello");
        assert!(!result.contains("System:"));
        assert!(result.contains("Rule: formatting\nbe concise"));
        assert!(result.contains("Context:\ncwd: /workspace"));
        assert!(result.ends_with("User:\nhello"));
    }

    #[test]
    fn test_raw_message_when_no_extras() {
        let result = build_prepared_prompt(None, &[], &[], "hello world");
        assert_eq!(result, "hello world");
    }
}
