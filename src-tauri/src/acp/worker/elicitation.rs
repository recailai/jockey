//! Maps ACP `elicitation/create` forms to Jockey's question model and answers back.
//!
//! The requested schema is read as raw JSON rather than the typed schema: adapters put
//! their own `_meta` on fields and options (Claude's custom-answer marker, Codex's
//! `user_note` / `isSecret`), and a strict parse would reject forms we can still render.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub(crate) type Answers = HashMap<String, Vec<String>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FieldKind {
    Text,
    Number,
    Integer,
    Boolean,
    Single,
    Multi,
}

#[derive(Clone, Debug)]
pub(crate) struct FieldSpec {
    /// Form field key; doubles as the question id shown to the UI.
    pub key: String,
    pub kind: FieldKind,
    /// Free-text companion field (`question_N_custom`, Codex `user_note`).
    pub other_key: Option<String>,
    /// Enum const that means "other" (Codex), sent when the user types their own answer.
    pub other_option: Option<String>,
    /// Enum consts the form accepts; typed values outside it are sent only via `other_key`.
    pub options: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct MappedForm {
    pub title: Option<String>,
    pub questions: Vec<Value>,
    pub fields: Vec<FieldSpec>,
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn codex_meta<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get("_meta")?.get("codex")?.get(key)
}

fn is_codex_other_option(option: &Value) -> bool {
    codex_meta(option, "role").and_then(Value::as_str) == Some("other")
        || codex_meta(option, "isOther").and_then(Value::as_bool) == Some(true)
}

fn custom_answer_target(key: &str, schema: &Value) -> Option<String> {
    let marker = schema.get("_meta")?.get("_askUserQuestionCustomAnswer")?;
    if marker.get("isCustomAnswer").and_then(Value::as_bool) == Some(true) {
        return marker
            .get("questionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| key.strip_suffix("_custom").map(str::to_string));
    }
    None
}

fn is_note(schema: &Value) -> bool {
    codex_meta(schema, "role").and_then(Value::as_str) == Some("user_note")
}

struct ParsedOption {
    value: String,
    label: String,
    description: Option<String>,
    preview: Option<String>,
    other: bool,
}

fn parse_option(option: &Value) -> Option<ParsedOption> {
    if let Some(s) = option.as_str() {
        return Some(ParsedOption {
            value: s.to_string(),
            label: s.to_string(),
            description: None,
            preview: None,
            other: false,
        });
    }
    let value = option
        .get("const")
        .or_else(|| option.get("value"))
        .and_then(Value::as_str)?
        .to_string();
    let preview = option
        .get("_meta")
        .and_then(|m| m.get("_claude/askUserQuestionOption"))
        .and_then(|m| m.get("preview"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ParsedOption {
        label: str_of(option, "title").unwrap_or(&value).to_string(),
        description: str_of(option, "description").map(str::to_string),
        preview,
        other: is_codex_other_option(option),
        value,
    })
}

fn options_of(schema: &Value) -> (FieldKind, Vec<ParsedOption>) {
    let is_array = schema.get("type").and_then(Value::as_str) == Some("array");
    let source = if is_array {
        schema.get("items").unwrap_or(&Value::Null)
    } else {
        schema
    };
    let kind = if is_array {
        FieldKind::Multi
    } else {
        FieldKind::Single
    };
    for key in ["oneOf", "anyOf"] {
        if let Some(list) = source.get(key).and_then(Value::as_array) {
            return (kind, list.iter().filter_map(parse_option).collect());
        }
    }
    if let Some(list) = source.get("enum").and_then(Value::as_array) {
        let names = source.get("enumNames").and_then(Value::as_array);
        let options = list
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let mut option = parse_option(v)?;
                if let Some(name) = names.and_then(|n| n.get(i)).and_then(Value::as_str) {
                    option.label = name.to_string();
                }
                Some(option)
            })
            .collect();
        return (kind, options);
    }
    (kind, Vec::new())
}

pub(crate) fn map_form(params: &Value) -> Result<MappedForm, String> {
    let message = str_of(params, "message").map(str::to_string);
    let schema = params
        .get("requestedSchema")
        .ok_or("elicitation form has no requestedSchema")?;
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("elicitation schema has no properties")?;
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    // Companion fields first, so the main question can be marked allowOther.
    let mut companions: HashMap<String, String> = HashMap::new();
    let mut unresolved_notes: Vec<&String> = Vec::new();
    for (key, prop) in props {
        if let Some(target) = custom_answer_target(key, prop) {
            if props.contains_key(&target) {
                companions.insert(target, key.clone());
                continue;
            }
        }
        if is_note(prop) {
            unresolved_notes.push(key);
        }
    }
    let selects: Vec<&String> = props
        .iter()
        .filter(|(k, p)| {
            !companions.values().any(|c| c == *k) && !is_note(p) && !options_of(p).1.is_empty()
        })
        .map(|(k, _)| k)
        .collect();
    let mut standalone_notes: Vec<&String> = Vec::new();
    for note in unresolved_notes {
        let owner = selects
            .iter()
            .filter(|s| note.starts_with(s.as_str()) && !companions.contains_key(s.as_str()))
            .max_by_key(|s| s.len())
            .or_else(|| {
                (selects.len() == 1 && !companions.contains_key(selects[0].as_str()))
                    .then(|| &selects[0])
            });
        match owner {
            Some(owner) => {
                companions.insert((*owner).clone(), note.clone());
            }
            None => standalone_notes.push(note),
        }
    }
    let hidden: Vec<&String> = companions.values().collect();

    let visible: Vec<(&String, &Value)> =
        props.iter().filter(|(k, _)| !hidden.contains(k)).collect();
    let single_field = visible.len() == 1;
    let mut message_used = false;
    let mut questions = Vec::new();
    let mut fields = Vec::new();

    for (key, prop) in visible {
        let type_name = prop.get("type").and_then(Value::as_str).unwrap_or("string");
        let (select_kind, parsed) = options_of(prop);
        let kind = if !parsed.is_empty() {
            select_kind
        } else {
            match type_name {
                "boolean" => FieldKind::Boolean,
                "number" => FieldKind::Number,
                "integer" => FieldKind::Integer,
                _ => FieldKind::Text,
            }
        };
        let title = str_of(prop, "title");
        let description = str_of(prop, "description");
        let prompt = match (description, single_field && message.is_some()) {
            (Some(d), _) => d.to_string(),
            (None, true) => {
                message_used = true;
                message.clone().unwrap_or_default()
            }
            (None, false) => title.unwrap_or(key).to_string(),
        };
        let header = title.filter(|t| *t != prompt).map(str::to_string);

        let other_option = parsed.iter().find(|o| o.other).map(|o| o.value.clone());
        let other_key = companions.get(key.as_str()).cloned();
        let shown: Vec<Value> = if kind == FieldKind::Boolean {
            vec![
                json!({ "value": "true", "label": "Yes" }),
                json!({ "value": "false", "label": "No" }),
            ]
        } else {
            parsed
                .iter()
                .filter(|o| !o.other)
                .map(|o| {
                    json!({
                        "value": o.value,
                        "label": o.label,
                        "description": o.description,
                        "preview": o.preview,
                    })
                })
                .collect()
        };
        let secret = codex_meta(prop, "isSecret").and_then(Value::as_bool) == Some(true);
        let select_like = matches!(
            kind,
            FieldKind::Single | FieldKind::Multi | FieldKind::Boolean
        );
        let input_type = match kind {
            FieldKind::Number | FieldKind::Integer => Some("number"),
            _ => None,
        };
        questions.push(json!({
            "id": key,
            "header": header,
            "prompt": prompt,
            "options": shown,
            "allowOther": select_like && (other_key.is_some() || other_option.is_some()),
            "secret": secret,
            "multi": kind == FieldKind::Multi,
            "required": required.contains(&key.as_str()),
            "inputType": input_type,
        }));
        fields.push(FieldSpec {
            key: key.clone(),
            kind,
            other_key,
            other_option,
            options: parsed.iter().map(|o| o.value.clone()).collect(),
        });
    }
    // A note that belongs to no select question is just a text question.
    for key in standalone_notes {
        let prop = &props[key.as_str()];
        questions.push(json!({
            "id": key,
            "header": Value::Null,
            "prompt": str_of(prop, "title").or(str_of(prop, "description")).unwrap_or(key),
            "options": [],
            "allowOther": false,
            "secret": codex_meta(prop, "isSecret").and_then(Value::as_bool) == Some(true),
            "multi": false,
            "required": false,
        }));
        fields.push(FieldSpec {
            key: key.clone(),
            kind: FieldKind::Text,
            other_key: None,
            other_option: None,
            options: Vec::new(),
        });
    }

    Ok(MappedForm {
        title: if message_used { None } else { message },
        questions,
        fields,
    })
}

/// Builds the `content` object of an `accept` response. Keys are exactly the form's field
/// keys; values carry the type the field's schema declares.
pub(crate) fn build_content(
    fields: &[FieldSpec],
    answers: &Answers,
) -> Result<Map<String, Value>, String> {
    let mut content = Map::new();
    for field in fields {
        let given: Vec<&str> = answers
            .get(&field.key)
            .map(|v| {
                v.iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if given.is_empty() {
            continue;
        }
        match field.kind {
            FieldKind::Text => {
                content.insert(field.key.clone(), json!(given[0]));
            }
            FieldKind::Number => {
                let n: f64 = given[0]
                    .parse()
                    .map_err(|_| format!("\"{}\" is not a number.", given[0]))?;
                content.insert(field.key.clone(), json!(n));
            }
            FieldKind::Integer => {
                let n: i64 = given[0]
                    .parse()
                    .map_err(|_| format!("\"{}\" is not a whole number.", given[0]))?;
                content.insert(field.key.clone(), json!(n));
            }
            FieldKind::Boolean => {
                let b = match given[0] {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("\"{other}\" is not yes or no.")),
                };
                content.insert(field.key.clone(), json!(b));
            }
            FieldKind::Single | FieldKind::Multi => {
                let (chosen, typed): (Vec<&str>, Vec<&str>) = given
                    .iter()
                    .partition(|v| field.options.iter().any(|o| o == **v));
                if let Some(text) = typed.first() {
                    // A typed answer replaces the selection, as in the CLI's "Other" box.
                    match (&field.other_key, &field.other_option) {
                        (Some(other_key), other_option) => {
                            content.insert(other_key.clone(), json!(text));
                            if let Some(option) = other_option {
                                let value = if field.kind == FieldKind::Multi {
                                    json!([option])
                                } else {
                                    json!(option)
                                };
                                content.insert(field.key.clone(), value);
                            }
                        }
                        (None, Some(option)) => {
                            content.insert(field.key.clone(), json!(option));
                        }
                        (None, None) => {
                            return Err(format!("\"{text}\" is not one of the choices."));
                        }
                    }
                } else if field.kind == FieldKind::Multi {
                    content.insert(field.key.clone(), json!(chosen));
                } else {
                    content.insert(field.key.clone(), json!(chosen[0]));
                }
            }
        }
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_form() -> Value {
        json!({
            "mode": "form", "sessionId": "s", "message": "Please answer the following questions.",
            "requestedSchema": { "type": "object", "properties": {
                "question_0": { "type": "string", "title": "Library", "description": "Which library?",
                    "oneOf": [
                        { "const": "A (Recommended)", "title": "A (Recommended)", "description": "fast" },
                        { "const": "B", "title": "B",
                          "_meta": { "_claude/askUserQuestionOption": { "preview": "<b>x</b>" } } }
                    ] },
                "question_0_custom": { "type": "string", "title": "Other",
                    "_meta": { "_askUserQuestionCustomAnswer": { "questionId": "question_0", "isCustomAnswer": true } } },
                "question_1": { "type": "array", "title": "Features", "description": "Which features?",
                    "items": { "anyOf": [ { "const": "x", "title": "x" }, { "const": "y", "title": "y" } ] } },
                "question_1_custom": { "type": "string", "title": "Other",
                    "_meta": { "_askUserQuestionCustomAnswer": { "questionId": "question_1", "isCustomAnswer": true } } }
            } }
        })
    }

    fn answers(items: &[(&str, &[&str])]) -> Answers {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    #[test]
    fn claude_questions_map_with_companions_folded_in() {
        let form = map_form(&claude_form()).unwrap();
        assert_eq!(
            form.title.as_deref(),
            Some("Please answer the following questions.")
        );
        assert_eq!(form.questions.len(), 2);
        let q0 = &form.questions[0];
        assert_eq!(q0["id"], "question_0");
        assert_eq!(q0["header"], "Library");
        assert_eq!(q0["prompt"], "Which library?");
        assert_eq!(q0["allowOther"], true);
        assert_eq!(q0["multi"], false);
        assert_eq!(q0["options"][0]["label"], "A (Recommended)");
        assert_eq!(q0["options"][0]["description"], "fast");
        assert_eq!(q0["options"][1]["preview"], "<b>x</b>");
        assert_eq!(form.questions[1]["multi"], true);
        assert_eq!(
            form.fields[0].other_key.as_deref(),
            Some("question_0_custom")
        );
    }

    #[test]
    fn claude_answers_use_exact_keys_and_custom_wins() {
        let form = map_form(&claude_form()).unwrap();
        let content = build_content(
            &form.fields,
            &answers(&[
                ("question_0", &["A (Recommended)"]),
                ("question_1", &["x", "y"]),
            ]),
        )
        .unwrap();
        assert_eq!(content["question_0"], "A (Recommended)");
        assert_eq!(content["question_1"], json!(["x", "y"]));
        assert!(!content.contains_key("question_0_custom"));

        let content =
            build_content(&form.fields, &answers(&[("question_0", &["my own"])])).unwrap();
        assert_eq!(content["question_0_custom"], "my own");
        assert!(!content.contains_key("question_0"));
    }

    #[test]
    fn single_question_uses_the_message_as_prompt() {
        let form = map_form(&json!({
            "message": "Pick one", "requestedSchema": { "type": "object", "properties": {
                "question_0": { "type": "string", "title": "Mode",
                    "oneOf": [{ "const": "a", "title": "a" }] } } }
        }))
        .unwrap();
        assert_eq!(form.questions[0]["prompt"], "Pick one");
        assert_eq!(form.questions[0]["header"], "Mode");
        assert!(form.title.is_none());
    }

    #[test]
    fn codex_other_option_note_and_secret() {
        let form = map_form(&json!({
            "message": "Codex asks", "requestedSchema": { "type": "object", "properties": {
                "env": { "type": "string", "title": "Env", "description": "Which env?",
                    "oneOf": [
                        { "const": "dev", "title": "dev", "description": "local" },
                        { "const": "other", "title": "Other", "_meta": { "codex": { "role": "other" } } }
                    ] },
                "env_note": { "type": "string", "_meta": { "codex": { "role": "user_note" } } },
                "token": { "type": "string", "title": "Token", "_meta": { "codex": { "isSecret": true } } }
            } }
        }))
        .unwrap();
        assert_eq!(form.questions.len(), 2);
        assert_eq!(form.questions[0]["allowOther"], true);
        assert_eq!(form.questions[0]["options"].as_array().unwrap().len(), 1);
        assert_eq!(form.questions[1]["secret"], true);
        let content = build_content(
            &form.fields,
            &answers(&[("env", &["staging please"]), ("token", &["s3cret"])]),
        )
        .unwrap();
        assert_eq!(content["env"], "other");
        assert_eq!(content["env_note"], "staging please");
        assert_eq!(content["token"], "s3cret");
    }

    #[test]
    fn generic_fields_keep_their_schema_types() {
        let form = map_form(&json!({
            "message": "Details", "requestedSchema": { "type": "object",
                "required": ["name"], "properties": {
                "name": { "type": "string", "title": "Name" },
                "age": { "type": "integer", "title": "Age" },
                "ratio": { "type": "number", "title": "Ratio" },
                "ok": { "type": "boolean", "title": "OK?" },
                "size": { "type": "string", "enum": ["s", "m"], "enumNames": ["Small", "Medium"] }
            } }
        }))
        .unwrap();
        assert_eq!(form.questions[0]["required"], true);
        assert_eq!(form.questions[1]["inputType"], "number");
        assert_eq!(form.questions[3]["options"][0]["label"], "Yes");
        assert_eq!(form.questions[4]["options"][1]["label"], "Medium");
        let content = build_content(
            &form.fields,
            &answers(&[
                ("name", &["Ann"]),
                ("age", &["42"]),
                ("ratio", &["0.5"]),
                ("ok", &["false"]),
                ("size", &["m"]),
            ]),
        )
        .unwrap();
        assert_eq!(content["age"], json!(42));
        assert_eq!(content["ratio"], json!(0.5));
        assert_eq!(content["ok"], json!(false));
        assert_eq!(content["size"], "m");
        assert!(build_content(&form.fields, &answers(&[("age", &["x"])])).is_err());
    }
}
