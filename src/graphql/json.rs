mod json;
use serde_json::{json, Value};

// https://stackoverflow.com/questions/47070876/how-can-i-merge-two-json-objects-with-rust
pub fn merge_json(a: &mut Value, b: Value) {
    match (a, b) {
        (a @ &mut Value::Object(_), Value::Object(b)) => {
            let a = a.as_object_mut().unwrap();
            for (k, v) in b {
                merge_json(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

pub fn extract_mut(json_value: &mut Value, path: &[String]) -> Option<Value> {
    if path.len() == 1 {
        match json_value {
            Value::Object(v) => match v.remove(&path[0]) {
                Some(f) => Some(json!({ &path[0]: f })),
                None => None,
            },
            _ => None,
        }
    } else {
        match json_value {
            Value::Object(v) if v.contains_key(&path[0]) => {
                match extract_mut(v.get_mut(&path[0]).unwrap(), &path[1..]) {
                    Some(f) => Some(json!({ &path[0]: f })),
                    None => None,
                }
            }
            _ => None,
        }
    }
}

pub fn extract_mut_ren(
    json_value: &mut Value,
    path: &[String],
    new_names: &[String],
) -> Option<Value> {
    if path.len() == 1 {
        match json_value {
            Value::Object(v) => match v.remove(&path[0]) {
                Some(f) => Some(json!({ &new_names[0]: f })),
                None => None,
            },
            _ => None,
        }
    } else {
        match json_value {
            Value::Object(v) if v.contains_key(&path[0]) => {
                match extract_mut_ren(v.get_mut(&path[0]).unwrap(), &path[1..], &new_names[1..]) {
                    Some(f) => Some(json!({ &new_names[0]: f })),
                    None => None,
                }
            }
            _ => None,
        }
    }
}

/*
pub fn extract_mut(json_value: &mut Value, path: &[String]) -> Option<Value> {
    if path.len() == 1 {
        match json_value {
            Value::Object(v) => match v.remove(&path[0]) {
                r @ Some(_) => r,
                None => None,
            },
            _ => None,
        }
    } else {
        match json_value {
            Value::Object(v) if v.contains_key(&path[0]) => {
                extract_mut(v.get_mut(&path[0]).unwrap(), &path[1..])
            }
            _ => None,
        }
    }
}
*/

pub fn extract(json_value: &Value, path: &[String]) -> Option<Value> {
    if path.len() == 0 {
        return Some(json_value.clone());
    }

    let field = match json_value {
        Value::Object(v) if v.contains_key(&path[0]) => extract(&json_value[&path[0]], &path[1..]),
        _ => return None,
    };

    match field {
        None => None,
        f => Some(json!({ &path[0]: f })),
    }
}

pub fn remove_field(json_value: Value, path: &[String]) -> Value {
    if path.len() == 0 {
        return json_value;
    }

    if path.len() == 1 {
        return match json_value {
            Value::Object(mut v) => {
                v.remove(&path[0]);
                Value::Object(v)
            }
            _ => json_value,
        };
    }

    match json_value {
        Value::Object(mut v) => match v.remove(&path[0]) {
            Some(removed_field) => {
                let new_field = remove_field(removed_field, &path[1..]);
                v.insert(path[0].clone(), new_field);
                Value::Object(v)
            }
            _ => Value::Object(v),
        },
        _ => json_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mut_can_xxx_hierarchy() {
        let hierarchy = [
            "f1".to_string(),
            "subf3".to_string(),
            "subsubf1".to_string(),
        ];
        let mut json =
            json!({"f1": { "subf1":1, "subf2":2, "subf3": {"subsubf1":31, "subsubf2": 32 } }});
        let extracted = extract_mut(&mut json, &hierarchy).unwrap();

        let expected_extracted = json!({"f1": { "subf3": { "subsubf1":31 } }});
        let expected_json = json!({"f1": { "subf1":1, "subf2":2, "subf3": { "subsubf2": 32 } }});
        assert_eq!(expected_extracted, extracted);
        assert_eq!(expected_json, json);
    }
}
