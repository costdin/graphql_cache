pub mod cache;
use crate::graphql::json::merge_json;
use crate::graphql::parser::{
    expand_operation, Document, Error, Field, Operation, OperationType, Parameter, Traversable,
};
use crate::graphql_deserializer::{CacheHint, CacheScope, GraphQLResponse};
use serde_json::map::Map;
use serde_json::value::Value;
use serde_json::{from_value, json};
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
    for (value, mut hint) in cache_hints
        .into_iter()
        .filter(|h| h.1.scope == CacheScope::PUBLIC && h.1.path.len() > 0)
    {
        let (traversed_fields, cached_field) = query.traverse(&hint.path).unwrap();
        for (value, cache_key) in explode(value, cached_field, &traversed_fields) {
            cache.insert(cache_key, hint.max_age, value);
        }
    }
}

fn explode<'a>(
    value: Value,
    field: &Field<'a>,
    traversed_fields: &[&Field<'a>],
) -> Vec<(Value, String)> {
    let x = (
        value,
        field_to_cache_key(field),
    );
    vec![x]
}

//fn explode_recursive<'a>(
//    value: Value,
//    field: &Field<'a>,
//    traversed_fields: &[&Field<'a>],
//    accumulator: Vec<(Value, String)>,
//) -> Vec<(Value, String)> {
//    accumulator.push((value, get_deep_cache_key(traversed_fields, field)))
//}


fn explode_recursive<'a>(
    value: Value,
    field: &Field<'a>,
    traversed_fields: &'a [&Field<'a>],
    mut accumulator: Vec<(Value, String)>,
) -> Vec<(Value, String)> {
    let (subfields, new_value) = match field {
        Field::Field {
            name,
            alias,
            parameters,
            fields: subfields,
        } if parameters.len() > 0 => match accumulator.pop() {
            Some((Value::Object(mut last_value), cache_name)) => {
                let field_alias = alias.unwrap_or(name);
                let residual = last_value.remove(field_alias).unwrap();
                accumulator.push((Value::Object(last_value), cache_name));

                let k = traversed_fields
                    .iter()
                    .map(|f| field_to_cache_key(f))
                    .collect::<Vec<_>>();

                accumulator.push((residual.clone(), get_deep_cache_key(k.as_slice(), field)));
                (subfields, residual)
            }
            _ => return accumulator,
        },
        Field::Field {
            name,
            alias,
            parameters,
            fields: subfields,
        } => (subfields, value),
        Field::Fragment { name: _ } => return accumulator,
    };

    let new_traversed_fields = [traversed_fields, &vec![field]].concat();
    for subfield in subfields {
        explode_recursive(new_value, subfield, &new_traversed_fields, accumulator);
    }

    accumulator
}

fn match_field_with_cache<'a>(
    field: Field<'a>,
    cache: &cache::Cache<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    match get_deep_cached_item(&[], &field, cache) {
        Ok(Value::Object(mut v)) => {
            let cached_value = v.remove(field.get_name());
            match_field_with_cache_recursive(&mut Vec::new(), field, cached_value, &cache)
        },
        _ => (Some(field), None)
    }
}

// takes a query field, the already processed cached fields and the new cache
fn match_field_with_cache_recursive<'a>(
    stack: &mut Vec<String>,
    field: Field<'a>,
    cached_value_option: Option<Value>,
    cache: &cache::Cache<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    let cached_value = match (field.has_parameters(), cached_value_option) {
        (has_parameters, _) if has_parameters => {
            match get_deep_cached_item(stack, &field, cache) {
                Ok(c) => c,
                Err(_) => return (Some(field), None),
            }
        }
        (_, Some(c)) => c,
        _ => return (Some(field), None),
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
        _ => return (Some(field), None),
    };

    let cache_key = field_to_cache_key(&field);
    let (alias, name, subfields) = match field {
        Field::Field {
            alias,
            name,
            parameters: _,
            fields,
        } => (alias, name, fields),
        _ => return (Some(field), None),
    };

    stack.push(cache_key);

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
            + field.get_parameters()
                .iter()
                .map(|p| format!("{:?}", p))
                .collect::<Vec<_>>()
                .join("-")
                .as_str()
    }
}

fn get_deep_cache_key<'a>(
    cache_keys_path: &[String],
    current_item: &Field<'a>,
) -> String {
    if cache_keys_path.len() > 0 {
        cache_keys_path.join("+") + "+" + &field_to_cache_key(current_item)    
    } else {
        field_to_cache_key(current_item)
    }
}

fn get_deep_cached_item<'a>(
    cache_keys_path: &[String],
    current_item: &Field<'a>,
    cache: &cache::Cache<String, Value>,
) -> Result<Value, Error> {
    let cache_key = get_deep_cache_key(cache_keys_path, current_item);

    match cache.get(&cache_key) {
        Some(field_cache) => {
            let mut cached_value = json!({});
            for x in field_cache.into_iter() {
                merge_json(&mut cached_value, (*x).clone())
            }

            Ok(cached_value)
        }
        None => Ok(json!({})),
    }
}

struct key_items<'a> {
    name: &'a str,
    parameters: Vec<Parameter<'a>>,
}
