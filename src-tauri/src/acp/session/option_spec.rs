//! What a runtime's parameters are, and how each one reaches that runtime.
//!
//! The four CLIs do not share a parameter set. Claude has effort plus two boolean knobs
//! (1M context, fast mode); Codex has six reasoning levels plus a service tier; Pi has seven
//! thinking levels and no boolean knobs; Antigravity has three effort levels and bakes a
//! second axis into its model ids. They also differ in *how* a value is delivered — a turn
//! parameter, a model-id suffix, a `--settings` JSON blob, or a separate RPC.
//!
//! Discovery used to emit a bag of loosely-typed options and every consumer then guessed:
//! the UI matched option names against `includes("reasoning")`, and the send path matched ids
//! against `["effort", "reasoning_effort", "reasoningEffort"]`. Declaring and then guessing is
//! why each newly-learned fact had to be threaded through several places by hand, and why a
//! runtime's real capability could sit in the catalog while a consumer silently ignored it.
//!
//! So the runtime declares, and consumers obey: `kind` says how to render it, `wire` says how
//! to deliver it, `default` says what happens when the user picks nothing. Adding a runtime
//! means writing one discovery function and nothing else.

use serde::Serialize;
use serde_json::{json, Value};

/// How a chosen value reaches the CLI. This is the part that genuinely differs per runtime,
/// so it travels as data rather than as a branch in the send path.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::acp) enum Wire {
    /// A field on the turn/prompt request, e.g. Codex's `effort` and `serviceTier`.
    TurnParam {
        name: String,
        /// For a toggle: the value to send when it is on. Absent for selects, which send the
        /// chosen value itself.
        #[serde(skip_serializing_if = "Option::is_none")]
        on_value: Option<String>,
    },
    /// A command-line flag on the process itself, e.g. Claude's `--effort high`. Headless
    /// runtimes have no per-turn channel — the value is fixed when the process is launched —
    /// so this is genuinely a different mechanism from `TurnParam`, not a rename of it.
    CliFlag { flag: String },
    /// Appended to the model id, e.g. Claude's 1M-context variant (`claude-opus-5[1m]`).
    ModelSuffix { suffix: String },
    /// Merged into the CLI's `--settings` JSON, e.g. Claude's fast mode.
    CliSettings { json: Value },
    /// A separate control-plane call, e.g. Pi's `set_thinking_level`.
    Rpc { method: String, field: String },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(in crate::acp) enum OptionKind {
    Select,
    Toggle,
}

/// One value a select parameter accepts.
///
/// `meta` exists because capabilities are frequently a property of the *value*, not of the
/// parameter: which effort levels a model supports, whether it offers a 1M window or a faster
/// tier, what it defaults to. Without somewhere to put that, a runtime had to bypass this type
/// and splice its own array in after serialization — which is the same "declare, then work
/// around the declaration" habit this module exists to end.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(in crate::acp) struct OptionValue {
    pub(in crate::acp) value: String,
    pub(in crate::acp) name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(in crate::acp) description: String,
    /// Merged into this value's JSON object, so consumers read capabilities as plain fields.
    #[serde(skip)]
    pub(in crate::acp) meta: Option<Value>,
}

impl OptionValue {
    pub(in crate::acp) fn new(value: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            name: name.into(),
            description: String::new(),
            meta: None,
        }
    }

    /// Capability fields for this specific value, merged in at serialization time.
    pub(in crate::acp) fn with_meta(mut self, meta: Value) -> Self {
        self.meta = Some(meta);
        self
    }

    fn to_value(&self) -> Value {
        let mut out = json!({ "value": self.value, "name": self.name });
        if !self.description.is_empty() {
            out["description"] = json!(self.description);
        }
        if let (Some(Value::Object(meta)), Value::Object(base)) = (self.meta.clone(), &mut out) {
            for (key, value) in meta {
                base.insert(key, value);
            }
        }
        out
    }

    pub(in crate::acp) fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

/// One parameter a runtime accepts.
#[derive(Debug, Clone, Serialize)]
pub(in crate::acp) struct OptionSpec {
    /// Also the key this is stored under, so there is no id-to-key translation anywhere.
    pub(in crate::acp) id: String,
    pub(in crate::acp) name: String,
    pub(in crate::acp) kind: OptionKind,
    pub(in crate::acp) wire: Wire,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(in crate::acp) description: String,
    /// What the runtime itself uses when the user picks nothing — read from the CLI, never
    /// assumed, so an unset control can show what will actually happen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::acp) default: Option<String>,
    /// Empty means it applies to every model of this runtime.
    pub(in crate::acp) applies_to_models: Vec<String>,
    pub(in crate::acp) values: Vec<OptionValue>,
}

impl OptionSpec {
    pub(in crate::acp) fn select(
        id: &str,
        name: &str,
        wire: Wire,
        values: Vec<OptionValue>,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: OptionKind::Select,
            wire,
            description: String::new(),
            default: None,
            applies_to_models: Vec::new(),
            values,
        }
    }

    pub(in crate::acp) fn toggle(id: &str, name: &str, wire: Wire) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: OptionKind::Toggle,
            wire,
            description: String::new(),
            default: None,
            applies_to_models: Vec::new(),
            values: Vec::new(),
        }
    }

    pub(in crate::acp) fn describe(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub(in crate::acp) fn defaulting_to(mut self, default: Option<String>) -> Self {
        self.default = default;
        self
    }

    pub(in crate::acp) fn for_models(mut self, models: Vec<String>) -> Self {
        self.applies_to_models = models;
        self
    }

    /// Serialized with the legacy field names alongside the new ones, so consumers can be
    /// migrated one at a time instead of in a single flag-day change.
    pub(in crate::acp) fn to_value(&self) -> Value {
        let values = self
            .values
            .iter()
            .map(OptionValue::to_value)
            .collect::<Vec<_>>();
        let legacy_type = match self.kind {
            OptionKind::Select => "select",
            OptionKind::Toggle => "toggle",
        };
        json!({
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
            "wire": self.wire,
            "description": self.description,
            "default": self.default,
            "appliesToModels": self.applies_to_models,
            "values": values,

            // Legacy shape, still read by the current UI.
            "type": legacy_type,
            "category": legacy_category(&self.id),
            "currentValue": "",
            "options": values,
        })
    }
}

/// The old `category` field the UI still matches on. Derived here rather than asked for, so a
/// runtime never has to know about a taxonomy that is on its way out.
fn legacy_category(id: &str) -> &'static str {
    match id {
        "model" => "model",
        "mode" => "mode",
        "effort" | "reasoning_effort" | "thinking_level" => "effort",
        _ => "toggle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_select_declares_how_its_value_reaches_the_cli() {
        let spec = OptionSpec::select(
            "effort",
            "Effort",
            Wire::TurnParam {
                name: "effort".to_string(),
                on_value: None,
            },
            vec![OptionValue::new("high", "high").described("Greater reasoning depth")],
        )
        .defaulting_to(Some("high".to_string()));

        let value = spec.to_value();
        assert_eq!(value["kind"], "select");
        assert_eq!(value["wire"]["kind"], "turn_param");
        assert_eq!(value["wire"]["name"], "effort");
        assert_eq!(value["default"], "high");
        // The send path reads `wire`; it must not have to guess from the id again.
        assert_eq!(value["values"][0]["description"], "Greater reasoning depth");
    }

    #[test]
    fn a_toggle_carries_the_value_to_send_when_it_is_on() {
        let spec = OptionSpec::toggle(
            "fast",
            "Fast mode",
            Wire::TurnParam {
                name: "serviceTier".to_string(),
                on_value: Some("priority".to_string()),
            },
        )
        .for_models(vec!["gpt-5.6-sol".to_string()]);

        let value = spec.to_value();
        assert_eq!(value["kind"], "toggle");
        assert_eq!(value["wire"]["on_value"], "priority");
        assert_eq!(value["appliesToModels"][0], "gpt-5.6-sol");
    }

    /// Claude delivers its two knobs by completely different mechanisms, which is exactly why
    /// the delivery mechanism has to be data rather than a branch in the send path.
    #[test]
    fn claude_knobs_declare_two_different_delivery_mechanisms() {
        let one_million = OptionSpec::toggle(
            "one_million",
            "1M context",
            Wire::ModelSuffix {
                suffix: "[1m]".to_string(),
            },
        );
        let fast = OptionSpec::toggle(
            "fast",
            "Fast mode",
            Wire::CliSettings {
                json: json!({ "fastMode": true }),
            },
        );
        assert_eq!(one_million.to_value()["wire"]["kind"], "model_suffix");
        assert_eq!(fast.to_value()["wire"]["kind"], "cli_settings");
    }

    #[test]
    fn legacy_consumers_still_see_the_shape_they_match_on() {
        let spec = OptionSpec::select(
            "reasoning_effort",
            "Effort",
            Wire::TurnParam {
                name: "effort".to_string(),
                on_value: None,
            },
            vec![OptionValue::new("low", "low")],
        );
        let value = spec.to_value();
        assert_eq!(value["type"], "select");
        assert_eq!(value["category"], "effort");
        assert_eq!(value["options"][0]["value"], "low");
    }
}
