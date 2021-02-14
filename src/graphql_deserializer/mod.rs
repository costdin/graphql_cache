use crate::graphql::json::{extract, merge_json, remove_field};
use serde;
use serde::Deserialize;
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Deserialize, Debug)]
pub struct GraphQLResponse {
    pub data: Value,
    pub extensions: Option<GraphQLExtensions>,
}

impl GraphQLResponse {
    pub fn compress_cache_hints(self) -> (Value, Vec<(Value, CacheHint)>) {
        let mut cache = match self.extensions {
            Some(c) => c.cache_control,
            None => return (self.data, Vec::new()),
        };

        if cache.hints.len() == 0 {
            return (self.data, vec![]);
        }

        cache.hints.sort_by(order_hints);

        let mut compressed_hints = Vec::<(Value, CacheHint)>::new();
        let mut stack = Vec::<(Value, CacheHint)>::new();
        for hint in cache.hints {
            let (last_scope, last_max_age, traversed_hierarchy) =
                get_hints_from_stack(&hint, &mut stack);

            match traversed_hierarchy {
                Some(r) => compressed_hints.extend(r),
                None => {}
            };

            let cached_value = match extract(&self.data, &hint.path) {
                Some(v) => v,
                None => continue,
            };

            let scope = hint.scope.unwrap_or(last_scope);

            let max_age = hint.max_age.unwrap_or(last_max_age);

            add_to_stack(
                &mut stack,
                cached_value,
                CacheHint {
                    path: hint.path,
                    scope: scope,
                    max_age: max_age,
                },
            );
        }

        stack.retain(|(value, _)| !value.is_null());
        compressed_hints.extend(stack);

        return (self.data, compressed_hints);
    }
}

#[derive(Debug)]
pub struct CacheHint {
    pub path: Vec<String>,
    pub max_age: u16,
    pub scope: CacheScope,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLExtensions {
    pub cache_control: CacheControl,
}

#[derive(Deserialize, Debug)]
pub struct CacheControl {
    pub version: u8,
    pub hints: Vec<CacheHintDto>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CacheHintDto {
    pub path: Vec<String>,
    pub max_age: Option<u16>,
    pub scope: Option<CacheScope>,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum CacheScope {
    PUBLIC,
    PRIVATE,
}

// Returns the scope and duration from the hint stack. The hierarchy is built from the path, represented as follows:
// - [ "user" ]
// -- [ "user", "id" ]
// -- [ "user", "company" ]
// --- [ "user", "company", "id" ]
// --- [ "user", "company", "name" ]
// -- [ "user", "friend" ]
// --- [ "user", "friend", "id" ]
// If the current hint is out of the hierarchy, in the example when [ "user", "friend" ] is processed (after [ "user", "company", "name" ])
// the list of traversed nodes is returned (in this case [ "user", "company" ], [ "user", "company", "id" ], [ "user", "company", "name" ])
fn get_hints_from_stack(
    hint: &CacheHintDto,
    stack: &mut Vec<(Value, CacheHint)>,
) -> (CacheScope, u16, Option<Vec<(Value, CacheHint)>>) {
    match stack.pop() {
        Some((Value::Null, parent_hint)) if hint.path.starts_with(&parent_hint.path) => {
            (parent_hint.scope, parent_hint.max_age, None)
        }
        Some((parent_value, parent_hint)) if hint.path.starts_with(&parent_hint.path) => {
            let res = (parent_hint.scope, parent_hint.max_age, None);
            stack.push((remove_field(parent_value, &hint.path), parent_hint));

            res
        }
        Some((parent_value, parent_hint)) => {
            let (scope, max_age, tr) = get_hints_from_stack(hint, stack);
            let traversed_hierarchy = match (parent_value, tr) {
                (Value::Null, r @ Some(_)) => r,
                (Value::Null, None) => None,
                (parent_value, Some(mut r)) => {
                    r.push((parent_value, parent_hint));
                    Some(r)
                }
                (parent_value, None) => Some(vec![(parent_value, parent_hint)]),
            };

            (scope, max_age, traversed_hierarchy)
        }
        None => (
            hint.scope.unwrap_or(CacheScope::PUBLIC),
            hint.max_age.unwrap_or(0),
            None,
        ),
    }
}

fn add_to_stack<'a>(stack: &'a mut Vec<(Value, CacheHint)>, value: Value, hint: CacheHint) {
    for i in (0..stack.len()).rev() {
        if hint.max_age == stack[i].1.max_age && hint.scope == stack[i].1.scope {
            merge_json(&mut stack[i].0, value);
            stack.push((Value::Null, hint));
            return;
        }
    }

    stack.push((value, hint));
}

fn order_hints(h1: &CacheHintDto, h2: &CacheHintDto) -> Ordering {
    order_hints_paths(&h1.path, &h2.path)
}

fn order_hints_paths(s1: &[String], s2: &[String]) -> Ordering {
    let s1_first = s1.first();
    let s2_first = s2.first();

    if s1_first.is_none() && s2_first.is_none() {
        return Ordering::Equal;
    }

    let ss1 = match s1_first {
        Some(s) => s,
        None => return Ordering::Less,
    };

    let ss2 = match s2_first {
        Some(s) => s,
        None => return Ordering::Greater,
    };

    match ss1.cmp(ss2) {
        Ordering::Equal => order_hints_paths(&s1[1..], &s2[1..]),
        not_equal => not_equal,
    }
}
