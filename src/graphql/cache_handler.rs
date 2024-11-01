use super::cache::Cache;
use crate::graphql::json::{extract_mut, merge_json};
use crate::graphql::parser::{
    expand_operation, Error, Field, FragmentDefinition, Operation, OperationType, Parameter,
    ParameterValue, Traversable,
};
use crate::graphql_deserializer::{CacheHint, CacheScope, GraphQLResponse};
use futures::future::join_all;
use itertools::Itertools;
use serde_json::map::Map;
use serde_json::value::Value;
use serde_json::{from_value, json};
use std::collections::HashMap;
use std::future::Future;

/// Executes an operation against the cache.
/// Any residual field (which couldn't be solved by the cache) is forwarded to the get_fn() function
pub async fn execute_operation<'a, F, Fut>(
    operation: Operation<'a>,
    fragment_definitions: Vec<FragmentDefinition<'a>>,
    variables: Map<String, Value>,
    cache: Cache,
    user_id: Option<String>,
    get_fn: F,
) -> Result<Value, Error>
where
    F: FnOnce(Operation<'a>, Map<String, Value>) -> Fut,
    Fut: Future<Output = (Result<Value, Error>, Operation<'a>, Map<String, Value>)>,
{
    // If the operation is not a query, forward the whole document to the getfn() function
    if operation.operation_type != OperationType::Query {
        let (result, _, _) = get_fn(operation, variables).await;
        return result;
    }

    // Replace all fragments with actual fields
    // Expanded operation does not contain any fragment
    let expanded_operation = expand_operation(operation, fragment_definitions)?;
    let (residual_operation, data_from_cache) =
        match_operation_with_cache(expanded_operation, &variables, &user_id, &cache).await;

    match residual_operation {
        Some(operation) => {
            let deduplicated_operation = operation.deduplicate_fields()?;

            let (response, op, var) = get_fn(deduplicated_operation, variables).await;
            let result: GraphQLResponse = from_value(response?)?;
            let (mut response_data, hints) = result.compress_cache_hints();

            update_cache(cache, &user_id, hints, &op, &var).await;
            merge_json(&mut response_data, data_from_cache);

            let final_result = expand_response(response_data, &op, &operation);

            Ok(json!({ "data": final_result }))
        }
        None => Ok(json!({ "data": data_from_cache })),
    }
}

fn expand_response(
    json: Value,
    deduplicated_operation: &Operation,
    operation: &Operation,
) -> Value {
    let mut map = match json {
        Value::Object(map) => map,
        _ => return json,
    };

    for f in operation.fields.iter() {
        let df = deduplicated_operation
            .fields
            .iter()
            .filter(|ff| ff.is_same_field(f))
            .nth(0)
            .unwrap();
        let v = map[df.get_alias()].clone();

        map.insert(f.get_alias().to_string(), expand_response_field(v, df, f));
    }

    Value::Object(map)
}

fn expand_response_field(json: Value, deduplicated_field: &Field, field: &Field) -> Value {
    let mut map = match json {
        Value::Object(map) => map,
        _ => return json,
    };

    for f in field.get_subfields().iter() {
        let df = deduplicated_field
            .get_subfields()
            .iter()
            .filter(|ff| ff.is_same_field(f))
            .nth(0)
            .unwrap();
        let v = map[df.get_alias()].clone();

        map.insert(f.get_alias().to_string(), expand_response_field(v, df, f));
    }

    Value::Object(map)
}

async fn update_cache<'a>(
    cache: Cache,
    user_id: &Option<String>,
    cache_hints: Vec<(Value, CacheHint)>,
    query: &Operation<'a>,
    variables: &Map<String, Value>,
) {
    for (value, hint) in cache_hints.into_iter().filter(|h| h.1.path.len() > 0) {
        if let Some((traversed_fields, cached_field)) = query.traverse(&hint.path) {
            for (cache_key, cache_value) in
                get_cache_values(traversed_fields, cached_field, variables, value)
            {
                let cache_key = match (hint.scope, user_id) {
                    (CacheScope::PUBLIC, _) => cache_key,
                    (CacheScope::PRIVATE, Some(u)) => to_private_cache_key(u, &cache_key),
                    (CacheScope::PRIVATE, None) => continue,
                };

                if let Err(err) = cache.insert(cache_key, hint.max_age, cache_value).await {
                    println!("Cache Error");
                }
            }
        }
    }
}

fn to_private_cache_key(user_id: &str, cache_key: &str) -> String {
    [user_id, cache_key].join("")
}

fn get_cache_values<'a>(
    initial_path: Vec<&'a Field<'a>>,
    field: &'a Field<'a>,
    variables: &Map<String, Value>,
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
                fields
                    .iter()
                    .map(|f| field_to_cache_key(f, variables))
                    .collect::<Vec<String>>(),
                fields,
            )
        })
        .map(|(field_keys, fields)| {
            (
                field_keys.join("+"),
                extract_mut(&mut value, &fields_to_json_path(&fields)),
                fields,
            )
        })
        .filter(|(_, v, _)| v.is_some())
        .map(|(cache_key, v, path)| (cache_key, dealias_fields(v.unwrap(), &path, variables)))
        .collect::<Vec<_>>()
}

fn fields_to_json_path(fields: &[&Field]) -> Vec<String> {
    fields
        .iter()
        .map(|f| String::from(f.get_alias()))
        .collect::<Vec<_>>()
}

fn dealias_fields(mut json_value: Value, path: &[&Field], variables: &Map<String, Value>) -> Value {
    dealias_path_recursive(&mut json_value, path, variables);

    json_value
}

fn dealias_path_recursive(json_value: &mut Value, path: &[&Field], variables: &Map<String, Value>) {
    let (current_field, path_remainder): (&Field, &[&Field]) = match path {
        [] => return,
        [elem] => {
            dealias_field(json_value, *elem, variables);
            return;
        }
        p => (*p.iter().nth(0).unwrap(), &p[1..]),
    };

    let (name, alias) = (
        field_to_cache_key(current_field, variables),
        current_field.get_alias(),
    );

    let map = match json_value {
        Value::Object(map) => map,
        _ => return,
    };
    let mut v = map.remove(alias).unwrap();
    dealias_path_recursive(&mut v, path_remainder, variables);

    map.insert(String::from(name), v);
}

fn dealias_field(json_value: &mut Value, current_field: &Field, variables: &Map<String, Value>) {
    let (name, alias) = (
        field_to_cache_key(current_field, variables),
        current_field.get_alias(),
    );

    let map = match json_value {
        Value::Object(map) => map,
        _ => return,
    };

    match map.remove(alias) {
        Some(mut v) => {
            for subfield in current_field.get_subfields() {
                dealias_field(&mut v, subfield, variables);
            }

            map.insert(String::from(name), v);
        }
        _ => {}
    }
}

///
/// Splits a field into cacheable fields. A cacheable field
/// is a field that can be contained in a single cache entry.
/// If a subfield has a parameter, it can't be contained in its
/// parent's field, because a subfield may be queried using
/// different parameters:
///
/// { company(id: 12) { name subsidiary(id: 13) { name } } }
/// { company(id: 12) { name subsidiary(id: 14) { name } } }
///
/// so company.subsidiary can't be in the same cacheable block
/// as its parent.
///
/// The path of a cacheable field is the list of fields that are
/// traversed to reach it. This is represented as a Vec of fields.
///
fn cacheable_fields<'a>(field: &'a Field<'a>) -> Vec<Vec<&'a Field<'a>>> {
    let mut stack = vec![field];
    let mut result = vec![stack.clone()];

    for subfield in field.get_subfields().into_iter() {
        cacheable_fields_int(subfield, &mut stack, &mut result);
    }

    result
}

/// A recursive function used by cacheable_fields
fn cacheable_fields_int<'a>(
    field: &'a Field<'a>,
    stack: &mut Vec<&'a Field<'a>>,
    result: &mut Vec<Vec<&'a Field<'a>>>,
) {
    stack.push(field);
    if field.has_parameters() {
        result.push(stack.clone());
    }

    for subfield in field.get_subfields().into_iter() {
        cacheable_fields_int(subfield, stack, result);
    }

    stack.pop();
}

async fn match_operation_with_cache<'a>(
    operation: Operation<'a>,
    variables: &Map<String, Value>,
    user_id: &Option<String>,
    cache: &Cache,
) -> (Option<Operation<'a>>, Value) {
    let mut residual_fields = Vec::<Field>::new();
    let mut cached_result = Map::new();
    let mut cached_value = json!({});

    let cache_keys = operation
        .fields
        .iter()
        .map(cacheable_fields)
        .flatten()
        .map(|f| fields_to_cache_key(&f, &variables))
        .unique()
        .collect::<Vec<_>>();

    let cache_requests = cache_keys
        .iter()
        .map(|key| get_cached_item(&key, &user_id, &cache));
    let cache_items = join_all(cache_requests).await;

    for item in cache_items {
        match item {
            Some(x) => merge_json(&mut cached_value, x),
            None => {}
        }
    }

    for field in operation.fields {
        let alias = String::from(field.get_alias());
        let v = cached_value.get(field_to_cache_key(&field, &variables));

        let (residual_field, cached_field) = match v {
            Some(cached_value) => {
                match_field_with_cache_recursive(field, &variables, Some(cached_value.clone()))
            }
            None => (Some(field), None),
        };

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

    let residual_operation = if residual_fields.len() > 0 {
        let operation = Operation {
            name: operation.name,
            fields: residual_fields,
            variables: operation.variables,
            operation_type: operation.operation_type,
        };

        Some(operation)
    } else {
        None
    };

    (residual_operation, Value::Object(cached_result))
}

fn match_field_with_cache_recursive<'a>(
    field: Field<'a>,
    variables: &Map<String, Value>,
    cached_value: Option<Value>,
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
        let temp_subf: String;
        let subfield_name = if subfield.has_parameters() {
            temp_subf = field_to_cache_key(&subfield, &variables);
            &temp_subf
        } else {
            subfield.get_name()
        };
        let subfield_alias = String::from(subfield.get_alias());

        // If a subfield is unique, then extract the cache value from the cache object
        // else clone the cache value (so it can be used by the next duplicate)
        let field_from_cache = match subfield_map.get_mut(subfield_name) {
            Some(v) if v > &mut 1 => {
                *v -= 1;
                Some(cache_map[subfield_name].clone())
            }
            _ => cache_map.remove(subfield_name),
        };

        let (residual_subfield, from_cache) =
            match_field_with_cache_recursive(subfield, variables, field_from_cache);

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

fn field_to_cache_key<'a>(field: &Field<'a>, variables: &Map<String, Value>) -> String {
    let result = field.get_name().to_string();
    let parameters = field.get_parameters();

    if parameters.len() == 0 {
        result
    } else {
        parameters.iter().fold(result, |acc, p| {
            append_parameter_to_cache_key(acc, p, variables)
        })
    }
}

fn append_parameter_to_cache_key<'a>(
    cache_key: String,
    parameter: &Parameter<'a>,
    variables: &Map<String, Value>,
) -> String {
    let result = cache_key + "_" + parameter.name;

    match &parameter.value {
        ParameterValue::Nil => result + "NIL",
        ParameterValue::Scalar(s) => result + s,
        ParameterValue::Variable(v) => match &variables[*v] {
            Value::Bool(b) => result + &format!("{}", b),
            Value::Number(n) => result + &format!("{}", n),
            Value::String(s) => result + &format!("\"{}\"", s),
            Value::Object(o) => result + &format!("OBJ{:?}", o),
            Value::Array(o) => result + &format!("LST{:?}", o),
            Value::Null => result + "NIL",
        },
        ParameterValue::Object(obj) => result + &format!("OBJ{:?}", obj),
        ParameterValue::List(lst) => result + &format!("LST{:?}", lst),
    }
}

fn fields_to_cache_key<'a>(fields: &[&Field<'a>], variables: &Map<String, Value>) -> String {
    fields
        .iter()
        .map(|f| field_to_cache_key(f, variables))
        .collect::<Vec<String>>()
        .join("+")
}

async fn get_cached_item<'a>(
    cache_key: &String,
    user_id: &Option<String>,
    cache: &Cache,
) -> Option<Value> {
    let public_cache = cache.get(&cache_key).await;
    let private_cache = match user_id {
        Some(uid) => cache.get(&to_private_cache_key(uid, cache_key)).await,
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
        merge_json(&mut cached_value, x)
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
    use crate::graphql::parser::*;
    use serde_json::json;
    use serde_json::value::Value;
    use std::pin::Pin;

    pub fn create_cache() -> Cache {
        Cache::new()
    }

    #[tokio::test]
    async fn execute_operation_does_not_send_request_if_all_fields_are_cached() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(result1, result2);
    }

    #[tokio::test]
    async fn execute_operation_doesnt_send_request_if_all_fields_are_cached2() {
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

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            create_send_request(expected_result_1.clone(), cache_hints),
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
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
    async fn execute_operation_doesnt_send_request_if_all_fields_are_cached_and_aliased() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 = "{aliased_field1: field1{aliased_subfield1: subfield1}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
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
    async fn execute_operation_doesnt_send_request_if_all_fields_are_cached_and_aliased_with_parameters(
    ) {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 =
            "{aliased_field1: field1{aliased_subfield1: subfield1 the_alias: subfield3(id: 13)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
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
            json!({"data":{"aliased_field1":{"aliased_subfield1":55, "the_alias": 123}}})
        );
    }

    #[tokio::test]
    async fn execute_operation_send_request_if_subfield_has_different_parameter() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 =
            "{aliased_field1: field1{aliased_subfield1: subfield1 the_alias: subfield3(id: 15)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request_new_param,
        )
        .await
        .unwrap();

        assert_eq!(
            result1,
            json!({"data":{"field1":{"subfield1":55,"subfield2":777,"aliased_subfield":123,"aliased_private_subfield":111}}})
        );
        assert_eq!(
            result2,
            json!({"data":{"aliased_field1":{"aliased_subfield1":55, "the_alias": 999}}})
        );
    }

    #[tokio::test]
    async fn execute_operation_does_not_get_value_from_private_caches_for_different_users() {
        let cache = create_cache();

        let query = "{field1{subfield1 subfield2 aliased_subfield: subfield3(id: 13) aliased_private_subfield: subfield3(id: 11)}}";
        let query2 = "{field1{subfield1, subfield3(id: 11)}}";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_send_request,
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
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

    #[tokio::test]
    async fn execute_operation_handles_deep_field_cache() {
        let cache = create_cache();

        let query = "{field1(id: 10){subfield1{ subsubfield1 subsubfield2 } } }";
        let query2 = "{field1(id: 10){subfield1{ subsubfield1 subsubfield2 } } }";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            create_send_request(
                json!({"field1": {"subfield1":{ "subsubfield1": 123, "subsubfield2": 234 }}}),
                vec![
                    (vec![String::from("field1")], 0, false),
                    (
                        vec![String::from("field1"), String::from("subfield1")],
                        1000,
                        false,
                    ),
                    (
                        vec![
                            String::from("field1"),
                            String::from("subfield1"),
                            String::from("subsubfield1"),
                        ],
                        200,
                        true,
                    ),
                ],
            ),
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            Map::new(),
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(
            result1,
            json!({"data":{"field1":{"subfield1": {"subsubfield1": 123, "subsubfield2": 234 }}}})
        );
        assert_eq!(
            result2,
            json!({"data":{"field1":{"subfield1": {"subsubfield1": 123, "subsubfield2": 234 }}}})
        );
    }

    #[tokio::test]
    async fn execute_operation_handles_variables() {
        let cache = create_cache();

        let query = "query TheQuery($fieldId: ID!){field1(id: $fieldId){subfield1{ subsubfield1 subsubfield2 } } }";
        let query2 = "query TheQuery($fieldId2: ID!){field1(id: $fieldId2){subfield1{ subsubfield1 subsubfield2 } } }";

        let parsed_query = parse_query(query).unwrap();
        let parsed_query2 = parse_query(query2).unwrap();

        let mut variables = Map::new();
        variables.insert(String::from("fieldId"), json!(20));

        let mut variables2 = Map::new();
        variables2.insert(String::from("fieldId2"), json!(20));

        let result1 = execute_operation(
            parsed_query.operations.into_iter().nth(0).unwrap(),
            parsed_query.fragment_definitions,
            variables,
            cache.clone(),
            Some(String::from("u1")),
            create_send_request(
                json!({"field1": {"subfield1":{ "subsubfield1": 123, "subsubfield2": 234 }}}),
                vec![
                    (vec![String::from("field1")], 0, false),
                    (
                        vec![String::from("field1"), String::from("subfield1")],
                        1000,
                        false,
                    ),
                    (
                        vec![
                            String::from("field1"),
                            String::from("subfield1"),
                            String::from("subsubfield1"),
                        ],
                        200,
                        true,
                    ),
                ],
            ),
        )
        .await
        .unwrap();

        let result2 = execute_operation(
            parsed_query2.operations.into_iter().nth(0).unwrap(),
            parsed_query2.fragment_definitions,
            variables2,
            cache.clone(),
            Some(String::from("u1")),
            fake_not_called_send_request,
        )
        .await
        .unwrap();

        assert_eq!(
            result1,
            json!({"data":{"field1":{"subfield1": {"subsubfield1": 123, "subsubfield2": 234 }}}})
        );
        assert_eq!(
            result2,
            json!({"data":{"field1":{"subfield1": {"subsubfield1": 123, "subsubfield2": 234 }}}})
        );
    }

    fn create_send_request<'a>(
        data: Value,
        cache_hints: Vec<(Vec<String>, i16, bool)>,
    ) -> Box<
        dyn Fn(
            Operation<'a>,
            Map<String, Value>,
        ) -> Pin<
            Box<
                dyn Future<Output = (Result<Value, Error>, Operation<'a>, Map<String, Value>)> + '_,
            >,
        >,
    > {
        Box::new(move |d, v| Box::pin(fake_send_request_p(data.clone(), cache_hints.clone(), d, v)))
    }

    async fn fake_not_called_send_request<'a>(
        _: Operation<'a>,
        _: Map<String, Value>,
    ) -> (Result<Value, Error>, Operation<'a>, Map<String, Value>) {
        panic!("This method should never be called")
    }

    async fn fake_send_request_p<'a>(
        data: Value,
        cache_hints: Vec<(Vec<String>, i16, bool)>,
        document: Operation<'a>,
        variables: Map<String, Value>,
    ) -> (Result<Value, Error>, Operation<'a>, Map<String, Value>) {
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

        (result, document, variables)
    }

    async fn fake_send_request<'a>(
        document: Operation<'a>,
        variables: Map<String, Value>,
    ) -> (Result<Value, Error>, Operation<'a>, Map<String, Value>) {
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

        (result, document, variables)
    }

    async fn fake_send_request_new_param<'a>(
        document: Operation<'a>,
        variables: Map<String, Value>,
    ) -> (Result<Value, Error>, Operation<'a>, Map<String, Value>) {
        let result = Ok(json!(
            {
                "data": {
                    "aliased_field1": {
                        "the_alias": 999
                    }
                },
                "extensions": {
                    "cacheControl": {
                        "version": 1,
                        "hints": [
                            {
                                "path": ["field1"],
                                "maxAge": 2000
                            }
                        ]
                    }
                }
            }
        ));

        (result, document, variables)
    }
}
