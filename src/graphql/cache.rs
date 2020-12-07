pub mod cache;
use crate::graphql::json::{extract_mut, merge_json};
use crate::graphql::parser::{
    expand_operation, Document, Error, Field, Operation, OperationType, Traversable,
};
use crate::graphql_deserializer::{CacheHint, CacheScope, GraphQLResponse};
use serde_json::map::Map;
use serde_json::value::Value;
use serde_json::{from_value, json};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

pub fn create_cache() -> Arc<cache::Cache<String, Value>> {
    cache::Cache::new()
}

/// Execute all queries contained in the document against the cache.
/// Any residual operation (mutation or subscription) is forwarded to the get_fn() function
/// Any residual field (which couldn't be solved by the cache) is forwarded to the get_fn() function
pub async fn process_query<'a, Fut>(
    document: Document<'a>,
    cache: Arc<cache::Cache<String, Value>>,
    get_fn: impl Fn(Document<'a>) -> Fut,
) -> Result<Value, Error>
where
    Fut: Future<Output = (Result<Value, Error>, Document<'a>)>,
{
    // If at least one operation is not a query, forward the whole document to the getfn() function
    // TODO: Try to execute query operations against the cache
    if !document
        .operations
        .iter()
        .all(|op| op.operation_type == OperationType::Query)
    {
        let (result, doc) = get_fn(document).await;
        return result;
    }

    let mut cached_result = Map::new();
    let mut residual_operations = Vec::<Operation>::new();
    for operation in document.operations {
        // Replace all fragments with actual fields
        // Expanded operation does not contain any fragment
        let expanded_operation = expand_operation(operation, &document.fragment_definitions)?;

        let mut residual_fields = Vec::<Field>::new();
        for field in expanded_operation.fields {
            let alias = String::from(field.get_alias());
            let (residual_field, cached_field) = match_field_with_cache(field, &cache);

            match residual_field {
                Some(f) => residual_fields.push(f),
                None => {}
            };

            match cached_field {
                Some(r) => {
                    cached_result.insert(alias, r);
                }
                None => {}
            };
        }

        if residual_fields.len() > 0 {
            let operation = Operation {
                name: expanded_operation.name,
                fields: residual_fields,
                variables: expanded_operation.variables,
                operation_type: expanded_operation.operation_type,
            };

            residual_operations.push(operation);
        }
    }

    let data_from_cache = Value::Object(cached_result);

    if residual_operations.len() > 0 {
        let document = Document {
            operations: residual_operations,
            fragment_definitions: document.fragment_definitions,
        };

        let (response, doc) = get_fn(document).await;
        let result: GraphQLResponse = from_value(response?)?;

        let (mut response_data, hints) = result.compress_cache_hints();

        update_cache(&cache, hints, &doc.operations[0]);

        merge_json(&mut response_data, data_from_cache);

        Ok(json!({ "data": response_data }))
    } else {
        Ok(json!({ "data": data_from_cache }))
    }
}

fn update_cache<'a>(
    cache: &cache::Cache<String, Value>,
    cache_hints: Vec<(Value, CacheHint)>,
    query: &Operation<'a>,
) {
    for (value, hint) in cache_hints
        .into_iter()
        .filter(|h| h.1.scope == CacheScope::PUBLIC && h.1.path.len() > 0)
    {
        let (traversed_fields, cached_field) = query.traverse(&hint.path).unwrap();
        for (cache_key, cache_value) in get_cache_values(traversed_fields, cached_field, value) {
            cache.insert(cache_key, hint.max_age, cache_value);
        }
    }
}

fn get_cache_values<'a>(
    initial_path: Vec<&'a Field<'a>>,
    field: &'a Field<'a>,
    mut value: Value,
) -> Vec<(String, Value)> {
    let mut cacheable_fields = get_cacheable_fields(field, initial_path);
    // reverse collection so that fields closest to the root
    // are processed last
    cacheable_fields.sort_by(|path1, path2| path2.len().cmp(&path1.len()));

    cacheable_fields
        .into_iter()
        .map(|fields| {
            (
                fields_to_cache_key(&fields),
                extract_mut(&mut value, &fields_to_json_path(&fields)),
                fields,
            )
        })
        .filter(|(_, v, _)| v.is_some())
        .map(|(cache_key, v, path)| (cache_key, v.unwrap(), path))
        .map(|(cache_key, v, path)| (cache_key, dealias_fields(v, &path)))
        .collect::<Vec<_>>()
}

fn fields_to_json_path(fields: &[&Field]) -> Vec<String> {
    fields
        .iter()
        .map(|f| String::from(f.get_alias()))
        .collect::<Vec<_>>()
}

fn dealias_fields(mut json_value: Value, path: &[&Field]) -> Value {
    dealias_path_recursive(&mut json_value, path);

    json_value
}

fn dealias_path_recursive(json_value: &mut Value, path: &[&Field]) {
    let (current_field, path_remainder): (&Field, &[&Field]) = match path {
        [] => return,
        [elem] => {
            dealias_field(json_value, *elem);
            return;
        }
        p => (*p.iter().nth(0).unwrap(), &p[1..]),
    };

    let (name, alias) = (current_field.get_name(), current_field.get_alias());

    let map = match json_value {
        Value::Object(map) => map,
        _ => return,
    };
    let mut v = map.remove(alias).unwrap();
    dealias_path_recursive(&mut v, path_remainder);

    map.insert(String::from(name), v);
}

fn dealias_field(json_value: &mut Value, current_field: &Field) {
    let (name, alias) = (current_field.get_name(), current_field.get_alias());

    let map = match json_value {
        Value::Object(map) => map,
        _ => return,
    };

    match map.remove(alias) {
        Some(mut v) => {
            for subfield in current_field.get_subfields() {
                dealias_field(&mut v, subfield);
            }

            map.insert(String::from(name), v);
        }
        _ => {}
    }
}

fn match_field_with_cache<'a>(
    field: Field<'a>,
    cache: &cache::Cache<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    let mut cached_items = get_cached_item(&field, cache);

    if cached_items.len() > 0 {
        let cached_value = cached_items.remove(&field_to_cache_key(&field));
        match_field_with_cache_recursive(&mut Vec::new(), field, cached_value, &mut cached_items)
    } else {
        (Some(field), None)
    }
}

fn match_field_with_cache_recursive<'a>(
    stack: &mut Vec<String>,
    field: Field<'a>,
    cached_value_option: Option<Value>,
    cache: &mut HashMap<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    let asd = if field.has_parameters() {
        let key = concatenate_cache_keys(stack, &field);
        cache.remove(&key)
    } else {
        cached_value_option
    };

    let cached_value = match asd {
        Some(v) => v,
        _ => Value::Object(Map::new()),
    };

    if field.is_leaf() {
        return match cached_value {
            v @ Value::String(_) => (None, Some(v)),
            v @ Value::Bool(_) => (None, Some(v)),
            v @ Value::Number(_) => (None, Some(v)),
            Value::Array(a) if a.len() > 0 && !a[0].is_object() => (None, Some(Value::Array(a))),
            _ => (Some(field), None),
        };
    }

    let mut cache_map = match cached_value {
        Value::Object(map) => map,
        _ => Map::new(),
    };

    stack.push(field_to_cache_key(&field));

    let (alias, name, subfields) = match field {
        Field::Field {
            alias,
            name,
            fields,
            ..
        } => (alias, name, fields),
        _ => return (Some(field), None),
    };

    let mut value_from_cache = Map::new();
    let mut residual_subfields = Vec::<Field>::new();
    for subfield in subfields {
        let subfield_name = subfield.get_name();
        let subfield_alias = String::from(subfield.get_alias());
        let field_from_cache = cache_map.remove(subfield_name);

        let (residual_subfield, from_cache) =
            match_field_with_cache_recursive(stack, subfield, field_from_cache, cache);

        match residual_subfield {
            Some(f) => residual_subfields.push(f),
            None => {}
        };

        match from_cache {
            Some(f) => {
                value_from_cache.insert(subfield_alias, f);
            }
            None => {}
        };
    }

    stack.pop();

    let residual_field_result = if residual_subfields.len() > 0 {
        Some(Field::new_field(
            alias,
            name,
            Vec::new(),
            residual_subfields,
        ))
    } else {
        None
    };

    let cache_result = if value_from_cache.len() > 0 {
        Some(Value::Object(value_from_cache))
    } else {
        None
    };

    (residual_field_result, cache_result)
}

fn field_to_cache_key<'a>(field: &Field<'a>) -> String {
    if field.get_parameters().len() == 0 {
        field.get_name().to_string()
    } else {
        field.get_name().to_string()
            + "_"
            + field
                .get_parameters()
                .iter()
                .map(|p| format!("{:?}", p))
                .collect::<Vec<_>>()
                .join("-")
                .as_str()
    }
}

fn concatenate_cache_keys<'a>(cache_keys: &[String], field: &Field<'a>) -> String {
    cache_keys.join("+") + "+" + &field_to_cache_key(field).as_str()
}

fn fields_to_cache_key<'a>(fields: &[&Field<'a>]) -> String {
    fields
        .iter()
        .map(|f| field_to_cache_key(f))
        .collect::<Vec<String>>()
        .join("+")
}

fn get_cached_item<'a>(
    root_field: &Field<'a>,
    cache: &cache::Cache<String, Value>,
) -> HashMap<String, Value> {
    let cachable_fields = get_cacheable_fields(root_field, vec![]);
    let mut result = HashMap::new();

    if cachable_fields.len() > 0 {
        for field_path in cachable_fields {
            let mut cached_value = json!({});
            let cache_key = fields_to_cache_key(&field_path);

            match cache.get(&cache_key) {
                Some(field_cache) => {
                    for x in field_cache.into_iter() {
                        merge_json(&mut cached_value, (*x).clone())
                    }

                    result.insert(cache_key, cached_value);
                }
                None => {}
            }
        }
    }

    result
}

fn get_cacheable_fields<'a>(
    field: &'a Field<'a>,
    mut initial_path: Vec<&'a Field<'a>>,
) -> Vec<Vec<&'a Field<'a>>> {
    let mut cachable_fields = vec![vec![field]];

    extract_fields_with_parameters_recursive(field, &mut initial_path, &mut cachable_fields);

    cachable_fields
}

fn extract_fields_with_parameters_recursive<'a>(
    field: &'a Field<'a>,
    stack: &mut Vec<&'a Field<'a>>,
    accumulator: &mut Vec<Vec<&'a Field<'a>>>,
) {
    stack.push(field);

    if field.has_parameters() {
        accumulator.push(stack.clone());
    }

    for subfield in field.get_subfields() {
        extract_fields_with_parameters_recursive(subfield, stack, accumulator);
    }

    stack.pop();
}
