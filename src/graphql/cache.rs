pub mod cache;
use crate::graphql::json::{extract_mut, merge_json};
use crate::graphql::parser::{
    expand_operation, Document, Error, Field, Operation, OperationType, Parameter, ParameterValue,
    Traversable,
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
    user_id: Option<String>,
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
            let (residual_field, cached_field) = match_field_with_cache(field, &user_id, &cache);

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

        update_cache(&cache, &user_id, hints, &doc.operations[0]);

        merge_json(&mut response_data, data_from_cache);

        Ok(json!({ "data": response_data }))
    } else {
        Ok(json!({ "data": data_from_cache }))
    }
}

fn update_cache<'a>(
    cache: &cache::Cache<String, Value>,
    user_id: &Option<String>,
    cache_hints: Vec<(Value, CacheHint)>,
    query: &Operation<'a>,
) {
    for (value, hint) in cache_hints.into_iter().filter(|h| h.1.path.len() > 0) {
        if let Some((traversed_fields, cached_field)) = query.traverse(&hint.path) {
            for (cache_key, cache_value) in get_cache_values(traversed_fields, cached_field, value)
            {
                let cache_key = match (hint.scope, user_id) {
                    (CacheScope::PUBLIC, _) => cache_key,
                    (CacheScope::PRIVATE, Some(u)) => to_private_cache_key(u, &cache_key),
                    (CacheScope::PRIVATE, None) => continue,
                };

                cache.insert(cache_key, hint.max_age, cache_value);
            }
        }
    }
}

fn to_private_cache_key(user_id: &String, cache_key: &String) -> String {
    format!("{} {}", user_id, cache_key)
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
        .map(|(cache_key, v, path)| (cache_key, dealias_fields(v.unwrap(), &path)))
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
    user_id: &Option<String>,
    cache: &cache::Cache<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    let cached_value = get_cached_item(&field_to_cache_key(&field), user_id, cache);
    match_field_with_cache_recursive(&mut Vec::new(), field, user_id, cached_value, cache)
}

fn match_field_with_cache_recursive<'a>(
    stack: &mut Vec<String>,
    field: Field<'a>,
    user_id: &Option<String>,
    cached_value: Option<Value>,
    cache: &cache::Cache<String, Value>,
) -> (Option<Field<'a>>, Option<Value>) {
    if field.is_leaf() {
        return match cached_value {
            Some(v @ Value::String(_)) => (None, Some(v)),
            Some(v @ Value::Bool(_)) => (None, Some(v)),
            Some(v @ Value::Number(_)) => (None, Some(v)),
            Some(Value::Array(a)) if a.len() > 0 && !a[0].is_object() => {
                (None, Some(Value::Array(a)))
            }
            _ => (Some(field), None),
        };
    }

    let mut cache_map = match cached_value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    stack.push(field_to_cache_key(&field));

    let (alias, name, subfields, parameters) = match field {
        Field::Field {
            alias,
            name,
            fields,
            parameters,
            ..
        } => (alias, name, fields, parameters),
        _ => return (Some(field), None),
    };

    // produce a map of parameterless fields with the same name
    // we use this in the next loop to get fields from the cache
    // if a field is unique, then we can remove it from cached_value
    // if a field is not unique, then we have to clone it
    let mut subfield_map = HashMap::<String, i8>::new();
    for s in subfields.iter().filter(|s| !s.has_parameters()) {
        *subfield_map.entry(s.get_name().to_string()).or_insert(0) += 1;
    }

    let mut value_from_cache = Map::new();
    let mut residual_subfields = Vec::<Field>::new();
    for subfield in subfields {
        let subfield_name = subfield.get_name();
        let subfield_alias = String::from(subfield.get_alias());

        // If a subfield has parameters, then get it from the cache
        // else if a subfield is unique, then extract the cache value from the cache object
        // else clone the cache value (so it can be used by the next duplicate)
        let field_from_cache = if subfield.has_parameters() {
            let key = concatenate_cache_keys(stack, &subfield);
            get_cached_item(&key, user_id, cache)
        } else {
            match subfield_map.get_mut(subfield.get_name()) {
                Some(v) if v > &mut 1 => {
                    *v -= 1;
                    Some(cache_map[subfield_name].clone())
                }
                _ => cache_map.remove(subfield_name),
            }
        };

        let (residual_subfield, from_cache) =
            match_field_with_cache_recursive(stack, subfield, user_id, field_from_cache, cache);

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
            parameters,
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
    let result = field.get_name().to_string();
    let parameters = field.get_parameters();

    if parameters.len() == 0 {
        result
    } else {
        parameters
            .iter()
            .fold(result, |acc, p| append_parameter_to_cache_key(acc, p))
    }
}

fn append_parameter_to_cache_key<'a>(cache_key: String, parameter: &Parameter<'a>) -> String {
    let result = cache_key + "_" + parameter.name;

    match &parameter.value {
        ParameterValue::Nil => result + "NIL",
        ParameterValue::Scalar(s) => result + s,
        ParameterValue::Variable(v) => result + v,
        ParameterValue::Object(obj) => result + &format!("OBJ{:?}", obj),
        ParameterValue::List(lst) => result + &format!("LST{:?}", lst),
    }
}

fn concatenate_cache_keys<'a>(cache_keys: &[String], field: &Field<'a>) -> String {
    cache_keys.join("+") + "+" + &field_to_cache_key(field)
}

fn fields_to_cache_key<'a>(fields: &[&Field<'a>]) -> String {
    fields
        .iter()
        .map(|f| field_to_cache_key(f))
        .collect::<Vec<String>>()
        .join("+")
}

fn get_cached_item<'a>(
    cache_key: &String,
    user_id: &Option<String>,
    cache: &cache::Cache<String, Value>,
) -> Option<Value> {
    let public_cache = cache.get(&cache_key);
    let private_cache = match user_id {
        Some(uid) => cache.get(&to_private_cache_key(uid, cache_key)),
        None => None,
    };

    let cached_fields = match (public_cache, private_cache) {
        (Some(mut p), Some(r)) => {
            p.extend_from_slice(&r);
            p
        }
        (Some(p), None) => p,
        (None, Some(r)) => r,
        (None, None) => return None,
    };

    let mut cached_value = json!({});
    for x in cached_fields.into_iter() {
        merge_json(&mut cached_value, (*x).clone())
    }

    Some(cached_value)
}

fn get_cacheable_fields<'a>(
    field: &'a Field<'a>,
    mut initial_path: Vec<&'a Field<'a>>,
) -> Vec<Vec<&'a Field<'a>>> {
    let mut cachable_fields = Vec::new();

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

    if accumulator.len() == 0 {
        accumulator.push(vec![stack[0]]);
    }

    for subfield in field.get_subfields() {
        extract_fields_with_parameters_recursive(subfield, stack, accumulator);
    }

    stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::cache::create_cache;
    use crate::graphql::parser::*;
    use serde_json::json;
    use serde_json::value::Value;
    use std::pin::Pin;

    #[tokio::test]
    async fn process_query_does_not_send_request_if_all_fields_are_cached() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query).unwrap();

        let result1 = process_query(
            parsed_query,
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = process_query(
            parsed_query2,
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(result1, result2);
    }

    #[tokio::test]
    async fn process_query_doesnt_send_request_if_all_fields_are_cached2() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 = "{field1{subfield1}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let expected_result_1 = json!({"field1":{"subfield1":55,"subfield2":777,"aliased_subfield":123,"aliased_private_subfield":111}});
        let cache_hints = vec![
            (vec!["field1".to_string()], 2000i16, false),
            (
                vec!["field1".to_string(), "subfield1".to_string()],
                1000,
                false,
            ),
            (
                vec!["field1".to_string(), "aliased_private_subfield".to_string()],
                1000,
                true,
            ),
        ];

        let result1 = process_query(
            parsed_query,
            cache.clone(),
            Some(String::from("u1")),
            create_send_request(expected_result_1.clone(), cache_hints),
        )
        .await
        .unwrap();

        let result2 = process_query(
            parsed_query2,
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(result1, json!({ "data": expected_result_1 }));
        assert_eq!(result2, json!({"data":{"field1":{"subfield1":55}}}));
    }

    #[tokio::test]
    async fn process_query_doesnt_send_request_if_all_fields_are_cached_and_aliased() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 = "{aliased_field1: field1{aliased_subfield1: subfield1}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = process_query(
            parsed_query,
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = process_query(
            parsed_query2,
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(
            result1,
            json!({"data":{"field1":{"subfield1":55,"subfield2":777,"aliased_subfield":123,"aliased_private_subfield":111}}})
        );
        assert_eq!(
            result2,
            json!({"data":{"aliased_field1":{"aliased_subfield1":55}}})
        );
    }

    #[tokio::test]
    async fn process_query_does_not_get_value_from_private_caches_for_different_users() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 = "{field1{subfield1, subfield3(id: 11)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = process_query(
            parsed_query,
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = process_query(
            parsed_query2,
            cache.clone(),
            Some(String::from("u2")),
            create_send_request(json!({"field1": {"subfield3":999}}), vec![]),
        )
        .await
        .unwrap();

        assert_eq!(
            result1,
            json!({"data":{"field1":{"subfield1":55,"subfield2":777,"aliased_subfield":123,"aliased_private_subfield":111}}})
        );
        assert_eq!(
            result2,
            json!({"data":{"field1":{"subfield1":55, "subfield3":999}}})
        );
    }

    fn create_send_request<'a>(
        data: Value,
        cache_hints: Vec<(Vec<String>, i16, bool)>,
    ) -> Box<
        dyn Fn(
            Document<'a>,
        ) -> Pin<Box<dyn Future<Output = (Result<Value, Error>, Document<'a>)> + '_>>,
    > {
        Box::new(move |d| Box::pin(fake_send_request_p(data.clone(), cache_hints.clone(), d)))
    }

    async fn fake_not_called_send_request<'a>(
        _: Document<'a>,
    ) -> (Result<Value, Error>, Document<'a>) {
        panic!("This method should never be called")
    }

    async fn fake_send_request_p<'a>(
        data: Value,
        cache_hints: Vec<(Vec<String>, i16, bool)>,
        document: Document<'a>,
    ) -> (Result<Value, Error>, Document<'a>) {
        let cache_hints = cache_hints
            .iter()
            .map(|(path, max_age, is_private)| {
                if *is_private {
                    json!({"path": path, "maxAge": max_age, "scope": "PRIVATE"})
                } else {
                    json!({"path": path, "maxAge": max_age})
                }
            })
            .collect::<Vec<_>>();

        let result = Ok(json!(
            {
                "data": data,
                "extensions": {
                    "cacheControl": {
                        "version": 1,
                        "hints": cache_hints
                    }
                }
            }
        ));

        (result, document)
    }

    async fn fake_send_request<'a>(document: Document<'a>) -> (Result<Value, Error>, Document<'a>) {
        let result = Ok(json!(
            {
                "data": {
                    "field1": {
                        "subfield1": 55,
                        "subfield2": 777,
                        "aliased_subfield": 123,
                        "aliased_private_subfield": 111
                    }
                },
                "extensions": {
                    "cacheControl": {
                        "version": 1,
                        "hints": [
                            {
                                "path": ["field1"],
                                "maxAge": 2000
                            },
                            {
                                "path": ["field1", "subfield1"],
                                "maxAge": 1000
                            },
                            {
                                "path": ["field1", "aliased_private_subfield"],
                                "maxAge": 1000,
                                "scope": "PRIVATE"
                            }
                        ]
                    }
                }
            }
        ));

        (result, document)
    }
}
