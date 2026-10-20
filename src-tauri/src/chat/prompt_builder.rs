/// User-authored role prompt plus enabled rules, for the runtime's native system-prompt
/// channel. The user's message itself is never wrapped: it reaches every runtime verbatim.
pub(crate) fn build_system_prompt(
    role_system_prompt: Option<&str>,
    enabled_rules: &[(String, String)],
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(sp) = role_system_prompt.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(sp.to_string());
    }
    for (name, content) in enabled_rules {
        parts.push(format!("Rule: {name}\n{content}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Loads the role's prompt and enabled rules for launch-time delivery. Prewarm and turns
/// must agree on this text, since it is part of the provider process's launch contract.
pub(crate) fn role_instructions(
    state: &crate::types::AppState,
    role_name: &str,
    project_id: Option<&str>,
) -> Option<String> {
    let role = crate::db::role::load_role_scoped(state, role_name, project_id)
        .ok()
        .flatten()?;
    let rules = crate::db::rule::get_enabled_rules_for_role(state, &role.id).unwrap_or_default();
    build_system_prompt(Some(role.system_prompt.as_str()), &rules)
}

pub(super) fn with_command_suggestion(output: String) -> String {
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_carries_role_prompt_and_rules() {
        let rules = vec![("style".to_string(), "be terse".to_string())];
        let text = build_system_prompt(Some("You are Dev."), &rules).expect("system text");
        assert_eq!(text, "You are Dev.\n\nRule: style\nbe terse");
    }

    #[test]
    fn blank_prompt_and_no_rules_yield_nothing() {
        assert_eq!(build_system_prompt(Some("  "), &[]), None);
        assert_eq!(build_system_prompt(None, &[]), None);
    }

    #[test]
    fn rules_alone_are_enough() {
        let rules = vec![("a".to_string(), "b".to_string())];
        assert_eq!(
            build_system_prompt(None, &rules).as_deref(),
            Some("Rule: a\nb")
        );
    }
}
