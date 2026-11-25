//! Checking a tool call's arguments against the tool's JSON Schema.
//!
//! The model writes `arguments` as text, so it can be broken JSON, miss a required field, or give
//! a number as a string. The error goes back to the model as the tool's result, with the expected
//! schema, so it can fix the call on its next turn.

use serde_json::{Map, Value};

/// The arguments as a JSON object, or what's wrong with them (for the model to read).
///
/// Supports the subset the tools use: `type: object`, `properties`, `required`, and the types
/// `string`, `integer`, `boolean` and `array` of strings. Two lenient fixes, because local models
/// make them often and the meaning is plain: an empty string is `{}`, and an integer or boolean
/// sent as a string (`"10"`, `"true"`) is converted. Unknown keys are ignored.
pub fn validate(args_json: &str, schema: &Value) -> Result<Value, String> {
    let expected = || format!("Expected arguments: {}", compact(schema));

    let parsed: Value = if args_json.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(args_json)
            .map_err(|err| format!("the arguments are not valid JSON ({err}). {}", expected()))?
    };
    let Value::Object(mut args) = parsed else {
        return Err(format!(
            "the arguments must be a JSON object. {}",
            expected()
        ));
    };

    let mut problems = Vec::new();
    let required = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for name in required {
        if !args.contains_key(name) || args[name].is_null() {
            problems.push(format!("`{name}` is required"));
        }
    }

    if let Some(properties) = schema["properties"].as_object() {
        for (name, property) in properties {
            let Some(value) = args.get_mut(name) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            if let Err(problem) = check_type(value, property) {
                problems.push(format!("`{name}` {problem}"));
            }
        }
    }

    if problems.is_empty() {
        Ok(Value::Object(args))
    } else {
        Err(format!(
            "invalid arguments: {}. {}",
            problems.join("; "),
            expected()
        ))
    }
}

/// Checks (and where it's plain, converts) one value.
fn check_type(value: &mut Value, property: &Value) -> Result<(), String> {
    match property["type"].as_str() {
        Some("string") if value.is_string() => Ok(()),
        Some("string") => Err("must be a string".into()),
        Some("integer") => {
            if value.is_i64() || value.is_u64() {
                return Ok(());
            }
            let converted = match value {
                Value::String(text) => text.trim().parse::<i64>().ok(),
                Value::Number(n) => n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64),
                _ => None,
            };
            let number = converted.ok_or("must be an integer")?;
            *value = Value::from(number);
            Ok(())
        }
        Some("boolean") => {
            if value.is_boolean() {
                return Ok(());
            }
            let converted = match value.as_str().map(str::trim) {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            };
            *value = Value::Bool(converted.ok_or("must be true or false")?);
            Ok(())
        }
        Some("array") => {
            let items = value.as_array().ok_or("must be an array")?;
            if property["items"]["type"] == "string" && !items.iter().all(Value::is_string) {
                return Err("must be an array of strings".into());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The schema as one line, without the descriptions (the model has those already).
fn compact(schema: &Value) -> String {
    let mut out = schema.clone();
    if let Some(properties) = out.get_mut("properties").and_then(Value::as_object_mut) {
        for property in properties.values_mut() {
            if let Some(property) = property.as_object_mut() {
                property.remove("description");
            }
        }
    }
    out.to_string()
}
