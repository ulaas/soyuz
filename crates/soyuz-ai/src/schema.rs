//! JSON Schema generation (schemars, subschemas inlined) and a small validator covering
//! the keywords our tool schemas use: `type`, `properties`, `required`,
//! `additionalProperties`, `items`, `enum`, `const`, `anyOf` / `oneOf`, numeric and length
//! bounds. Used to check tool arguments from both native tool calls and the JSON fallback.

use serde_json::Value;

/// Schema of `T` as a plain JSON object without `$schema` / `title` noise.
pub fn schema_of<T: schemars::JsonSchema>() -> Value {
    let settings = schemars::generate::SchemaSettings::draft07().with(|s| {
        s.inline_subschemas = true;
    });
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(schema).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("$schema");
        o.remove("title");
        o.remove("definitions");
        o.remove("$defs");
    }
    v
}

/// Validate `value` against `schema`; returns human-readable errors (empty = valid).
pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    check(schema, value, "$", &mut errs);
    errs
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => v.as_i64().is_some() || v.as_u64().is_some() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true,
    }
}

fn check(schema: &Value, v: &Value, at: &str, errs: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        if schema == &Value::Bool(false) {
            errs.push(format!("{at}: not allowed"));
        }
        return;
    };
    match s.get("type") {
        Some(Value::String(t)) if !type_ok(t, v) => {
            errs.push(format!("{at}: expected {t}, got {}", kind(v)));
            return;
        }
        Some(Value::Array(ts)) if !ts.iter().filter_map(Value::as_str).any(|t| type_ok(t, v)) => {
            errs.push(format!("{at}: expected one of {}, got {}", Value::Array(ts.clone()), kind(v)));
            return;
        }
        _ => {}
    }
    if let Some(Value::Array(opts)) = s.get("enum")
        && !opts.contains(v)
    {
        errs.push(format!("{at}: must be one of {}", Value::Array(opts.clone())));
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        errs.push(format!("{at}: must be {c}"));
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(alts)) = s.get(key) {
            let ok = alts.iter().any(|alt| {
                let mut e = Vec::new();
                check(alt, v, at, &mut e);
                e.is_empty()
            });
            if !ok {
                errs.push(format!("{at}: does not match any allowed form"));
            }
        }
    }
    if let Some(Value::Array(all)) = s.get("allOf") {
        for sub in all {
            check(sub, v, at, errs);
        }
    }
    match v {
        Value::Object(o) => {
            let props = s.get("properties").and_then(Value::as_object);
            if let Some(Value::Array(req)) = s.get("required") {
                for r in req.iter().filter_map(Value::as_str) {
                    if !o.contains_key(r) {
                        errs.push(format!("{at}: missing required field “{r}”"));
                    }
                }
            }
            for (k, val) in o {
                let sub_at = format!("{at}.{k}");
                match props.and_then(|p| p.get(k)) {
                    Some(sub) => check(sub, val, &sub_at, errs),
                    None => match s.get("additionalProperties") {
                        Some(Value::Bool(false)) => errs.push(format!("{at}: unknown field “{k}”")),
                        Some(extra @ Value::Object(_)) => check(extra, val, &sub_at, errs),
                        _ => {}
                    },
                }
            }
        }
        Value::Array(a) => {
            if let Some(items) = s.get("items") {
                for (i, item) in a.iter().enumerate() {
                    check(items, item, &format!("{at}[{i}]"), errs);
                }
            }
            bound(s, "minItems", a.len() as f64, at, errs, |n, b| n >= b);
            bound(s, "maxItems", a.len() as f64, at, errs, |n, b| n <= b);
        }
        Value::String(st) => {
            let n = st.chars().count() as f64;
            bound(s, "minLength", n, at, errs, |n, b| n >= b);
            bound(s, "maxLength", n, at, errs, |n, b| n <= b);
        }
        Value::Number(num) => {
            let n = num.as_f64().unwrap_or(0.0);
            bound(s, "minimum", n, at, errs, |n, b| n >= b);
            bound(s, "maximum", n, at, errs, |n, b| n <= b);
        }
        _ => {}
    }
}

fn bound(
    s: &serde_json::Map<String, Value>,
    key: &str,
    n: f64,
    at: &str,
    errs: &mut Vec<String>,
    ok: impl Fn(f64, f64) -> bool,
) {
    if let Some(b) = s.get(key).and_then(Value::as_f64)
        && !ok(n, b)
    {
        errs.push(format!("{at}: violates {key} {b}"));
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_common_keywords() {
        let s = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "minLength": 1},
                "n": {"type": ["integer", "null"], "minimum": 0},
                "mode": {"enum": ["a", "b"]},
                "list": {"type": "array", "items": {"type": "string"}, "maxItems": 2}
            },
            "required": ["path"],
            "additionalProperties": false
        });
        assert!(validate(&s, &json!({"path": "x", "n": 3, "mode": "a", "list": ["q"]})).is_empty());
        let e = validate(&s, &json!({"n": -1, "mode": "z", "list": ["q", 1, "r"], "extra": 1}));
        let joined = e.join("\n");
        for needle in ["missing required field “path”", "minimum", "one of", "$.list[1]", "maxItems", "unknown field"]
        {
            assert!(joined.contains(needle), "{needle} not in {joined}");
        }
        assert!(validate(&s, &json!("str")).iter().any(|e| e.contains("expected object")));
    }
}
