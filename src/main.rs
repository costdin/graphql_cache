mod auth;
mod graphql;
mod graphql_deserializer;

use auth::{authorize_header, get_oidc_config, AuthConfiguration, AuthHeader, AuthorizationType};
use clap::Parser;
use graphql::cache::Cache;
use graphql::parser::serialize_operation;
use serde::Deserialize;
use serde_json;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use warp::Filter;

#[derive(Parser)]
struct CliArguments {
    config: Option<std::path::PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Config {
    redis_connection_string: String,
    oidc_configuration_endpoint: String,
    oidc_token_header: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    let args = CliArguments::parse();
    let config_path = args
        .config
        .unwrap_or(std::path::PathBuf::from_str("./etc/config.json").unwrap());

    let file_content = fs::read_to_string(config_path).expect("Unable to read file");
    let config: Config = serde_json::from_str(&file_content).expect("Unable to parse");

    #[cfg(not(test))]
    let cache = Cache::new(&config.redis_connection_string)
        .await
        .expect("Error initializing cache");
    #[cfg(test)]
    let cache = Cache::new();

    // We must leak the Box in order to get a `&'static str` borrow
    // `warp::header` requires the header name to be passed as a `&'static str`
    let header_name: &'static str = Box::leak(config.oidc_token_header.into_boxed_str());

    let auth_configuration =
        match get_oidc_config(&config.oidc_configuration_endpoint, header_name).await {
            Ok(auth_configuration) => auth_configuration,
            _ => {
                println!("Invalid OIDC configuration, fallback to simple auth");

                AuthConfiguration {
                    authorization_header: header_name,
                    authorization_type: AuthorizationType::Simple,
                }
            }
        };

    let end = warp::path("end").map(|| {
        exit(0);
        ""
    });
    let endpoint = warp::path("hello")
        .and(warp::addr::remote())
        .and(warp::body::json())
        .and(authorize_header(Arc::new(auth_configuration)))
        .and_then(move |c, d, auth_token| handle_request(c, d, auth_token, cache.clone()));

    let routes = endpoint.or(end);
    warp::serve(routes).run(([0, 0, 0, 0], 3033)).await;
}

async fn forward_graphql_request<'a>(
    operation: graphql::parser::Operation<'a>,
    variables: Map<String, Value>,
    auth_header: Option<String>,
) -> (
    Result<Value, graphql::parser::Error>,
    graphql::parser::Operation<'a>,
    Map<String, Value>,
) {
    let sss = serialize_operation(&operation);
    println!("Request: {}", sss);
    let mut map = HashMap::new();
    map.insert("query", Value::String(sss));
    map.insert("variables", Value::Object(variables));

    let client = reqwest::Client::new();
    let request = client.post("http://192.168.1.50:4000/").json(&map);

    let request_builder = if let Some(header) = auth_header {
        request.header("Authorization", header)
    } else {
        request
    };

    let res = request_builder.send().await;

    let the_v = match map.remove("variables") {
        Some(Value::Object(v)) => v,
        _ => Map::new(),
    };

    let resp = match res {
        Ok(r) => r.json::<Value>().await,
        Err(e) => {
            return (
                Err(graphql::parser::Error::new(format!(
                    "Request error: {:?}",
                    e
                ))),
                operation,
                the_v,
            )
        }
    };

    match resp {
        Ok(r) => (Ok(r), operation, the_v),
        Err(e) => (
            Err(graphql::parser::Error::new(format!(
                "Deserialization error: {:?}",
                e
            ))),
            operation,
            the_v,
        ),
    }
}

async fn handle_request(
    _addr_opt: Option<SocketAddr>,
    mut body: HashMap<String, Value>,
    auth_header: Option<AuthHeader>,
    cache: Cache,
) -> Result<impl warp::Reply, Infallible> {
    let (auth_token, auth_header_value) = match auth_header {
        Some(t) => (Some(t.sub), Some(t.header)),
        _ => (None, None),
    };

    let q = match body.remove("query") {
        Some(Value::String(q)) => q,
        _ => return Ok(format!("no")),
    };

    let document = match graphql::parser::parse_query(&q) {
        Ok(r) => r,
        Err(_) => return Ok(format!("nein")),
    };

    let variables = match body.remove("variables") {
        Some(Value::Object(map)) => map,
        Some(_) => return Ok(format!("nein variables")),
        None => serde_json::Map::<String, Value>::new(),
    };

    let (operation, fragment_definitions) = if document.operations.len() > 1 {
        if let Some(Value::String(operation_name)) = body.remove("operationName") {
            match document.filter_operation(&operation_name) {
                Ok(d) => (
                    d.operations.into_iter().nth(0).unwrap(),
                    d.fragment_definitions,
                ),
                Err(_) => return Ok(String::from("operationName ist gulen")),
            }
        } else {
            return Ok(format!("nein operationName"));
        }
    } else {
        (
            document.operations.into_iter().nth(0).unwrap(),
            document.fragment_definitions,
        )
    };

    let result = match graphql::cache_handler::execute_operation(
        operation,
        fragment_definitions,
        variables,
        cache,
        auth_token,
        |a, b| forward_graphql_request(a, b, auth_header_value),
    )
    .await
    {
        Ok(r) => format!("{}", r.to_string()),
        Err(e) => format!("{:?}", e),
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::graphql;
    use super::graphql_deserializer::{CacheScope, GraphQLResponse};
    use futures::executor::block_on;
    use serde_json::Value;

    #[test]
    fn test_parser() -> () {
        let xxxx = "{ f1(p1: 1,                          p2: \"parm2\") { f2 }}";
        let resultxxxx = graphql::parser::parse_query(xxxx);
        assert!(resultxxxx.is_ok());

        let stri = "                          {    field1  (p1 :                         1,         p2:\"as        \\\"      d              \"    )     {      subf1      subf2(  p3   :0)   { s     } }}      ";
        let rezult = graphql::parser::parse_query(stri);
        assert!(rezult.is_ok());

        let stri2 = "{ggggg: field1(p1:{v1:1,v2:\"2\",v3:{vv3:33},v4:[12,13,15]},p2:\"as        \\\"      d              \"){f1: subf1 subf2(p3:0){s}}, cccc: field1(p1:1){subf1}}";
        let rezult2 = graphql::parser::parse_query(stri2);
        assert!(rezult2.is_ok());

        let stri3 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } }";
        let rezult3 = graphql::parser::parse_query(stri3);
        assert!(rezult3.is_ok());

        let stri4 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } }";
        let rezult4 = graphql::parser::parse_query(stri4);
        assert!(rezult4.is_ok());

        let stri5 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }";
        let rezult5 = graphql::parser::parse_query(stri5);
        assert!(rezult5.is_ok());

        let stri6 = "query TheQuery{alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }";
        let rezult6 = graphql::parser::parse_query(stri6);
        assert!(rezult6.is_ok());

        let stri7 = "query TheQuery{alias1: field1(id: $p1) { dob ...userFragment } alias2: field2(id: $p1, name: \"the name\") {...userFragment } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) {...userFragment surname } alias2: field2(id: $p1, name: \"the second name\") {...userFragment surname } } fragment userFragment on User { id name }";
        let rezult7 = graphql::parser::parse_query(stri7);
        assert!(rezult7.is_ok());

        let stri8 = "{field1(p1:1,p2:\"as              d              \"){subf1 subf2(p3:0){s}}}";
        let result8 = graphql::parser::parse_query(stri8);
        assert!(result8.is_ok());

        let stri9 = "  {    field1 (    p1 : 1   ,  p2    :   \"asd\" ) {    subf1   subf2   (    p3 :  0 )   {   s  }  }    } ";
        let result9 = graphql::parser::parse_query(stri9);
        assert!(result9.is_ok());
    }

    #[tokio::test]
    async fn test_cache_small() -> () {
        let cache = graphql::cache::MemoryCache::new();
        let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

        block_on(cache.insert(format!("1aaaddccc{}", 0), 10000, json.clone()));
        block_on(cache.insert(format!("1aaa{}{}", 1, 0), 10000, json.clone()));

        match cache.get(&format!("1aaa{}{}", 1, 0)).await {
            Some(_) => {}
            None => assert_eq!(1, 0),
        };
    }

    #[test]
    fn test_deserializer() -> serde_json::Result<()> {
        let data = r#"
        {
            "data": {
                "user": {
                    "id": 10,
                    "name": "the name",
                    "age": 20,
                    "company": {
                        "id": 100,
                        "name": "the company"
                    },
                    "friend": {
                        "id": 11,
                        "name": "the friend",
                        "age": 25,
                        "friend": {
                            "id": 12,
                            "name": "the friend of the friend",
                            "age": 27
                        }
                    }
                }
            },
            "extensions": {
                "cacheControl": {
                    "version": 1,
                    "hints": [
                        {
                            "path": ["user"],
                            "maxAge": 200
                        },
                        {
                            "path": [ "user", "id" ],
                            "maxAge": 100
                        },
                        {
                            "path": [ "user", "name" ],
                            "scope": "PRIVATE"
                        },
                        {
                            "path": [ "user", "company" ],
                            "scope": "PRIVATE",
                            "maxAge": 150
                        },
                        {
                            "path": [ "user", "company", "id" ],
                            "scope": "PUBLIC"
                        },
                        {
                            "path": [ "user", "friend" ],
                            "maxAge": 500,
                            "scope": "PUBLIC"
                        },
                        {
                            "path": [ "user", "friend", "friend" ],
                            "maxAge": 200,
                            "scope": "PUBLIC"
                        },
                        {
                            "path": [ "user", "friend", "friend", "id" ],
                            "scope": "PUBLIC"
                        }
                    ]
                }
            }
        }"#;

        let result: GraphQLResponse = serde_json::from_str(data)?;
        let _ = std::collections::HashMap::<String, Value>::new();
        let cache = graphql::cache::MemoryCache::new();

        let (response_data, hints) = result.compress_cache_hints();

        let field_name: String = match response_data {
            Value::Object(m) => m.keys().into_iter().nth(0).unwrap().clone(),
            _ => {
                assert_eq!(1, 0);
                "".to_owned()
            }
        };

        for (value, hint) in hints {
            match (hint.scope, hint.max_age) {
                (CacheScope::PUBLIC, duration) => {
                    println!("{:#?}", hint);
                    println!("{:#?}", value);

                    block_on(cache.insert(field_name.clone(), duration, value));
                }
                _ => {}
            }
        }
        return Ok(());
    }

    #[cfg(feature = "slow_tests")]
    mod slow_tests {
        use super::super::graphql;
        use futures::executor::block_on;
        use rand::Rng;
        use serde_json::Value;
        use std::thread;
        use std::time::Duration;
        use std::time::SystemTime;

        #[test]
        fn test_cache_cleanup() -> () {
            let cache = graphql::cache::MemoryCache::new();
            let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

            block_on(cache.insert(format!("long_lasting"), 50000, json.clone()));
            for i in 0..1000000 {
                block_on(cache.insert(format!("xxxx{}", i), 1, json.clone()));
            }

            std::thread::sleep(Duration::from_secs(5));

            // At this point, all entries have expired, apart from `long_lasting`

            let c = cache.clone();
            let thread = thread::spawn(move || {
                let mut rng = rand::thread_rng();

                for _ in 0..1000000 {
                    let i = rng.gen_range(0..1000000);
                    let key = format!("xxxx{}", i);
                    let v = c.get(&key);
                    assert!(block_on(v).is_none());
                }
            });

            std::thread::sleep(Duration::from_secs(10));

            let key = format!("long_lasting");
            let cache_entry = cache.get(&key);
            assert!(block_on(cache_entry).is_some());

            assert!(thread.join().is_ok());
        }

        #[tokio::test]
        async fn test_cache() -> () {
            let cache = graphql::cache::MemoryCache::new();

            let c = cache.clone();
            let d = cache.clone();
            let e = cache.clone();
            let f = cache.clone();

            let vc: Value = serde_json::from_str(r#"{"a": 1234}"#).unwrap();
            let vd: Value = serde_json::from_str(r#"{"a": 5555}"#).unwrap();

            let start = SystemTime::now();
            block_on(cache.insert(String::from("adsasd0"), 010, vc.clone()));
            block_on(cache.insert(String::from("adsasd1"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd2"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd3"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd4"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd5"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd6"), 123, vd.clone()));
            block_on(cache.insert(String::from("adsasd7"), 123, vd.clone()));

            println!("{{ \"v\": 11 }}");

            let mut threads = Vec::new();
            for i in 0..1000 {
                let fff = cache.clone();
                let ttt = thread::spawn(move || {
                    let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

                    for x in 0i32..10000 {
                        let c = if i % 2 == 0 { 10000 - x } else { x };

                        block_on(fff.insert(format!("1aaaddccc{}", c), 1, json.clone()));
                        block_on(fff.insert(format!("1aaa{}{}", i, c), 100, json.clone()));

                        match block_on(fff.get(&format!("1aaa{}{}", i, c))) {
                            Some(_) => {}
                            None => assert_eq!(1, 0),
                        };
                    }
                });

                threads.push(ttt);
            }

            let t1 = thread::spawn(move || {
                let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

                for x in 0..10000 {
                    let daaaa = json.clone();
                    block_on(c.insert(format!("aaa{}", x), 1, daaaa));
                }
            });

            let t2 = thread::spawn(move || {
                let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

                for x in 0..12000 {
                    let daaaa = json.clone();
                    block_on(d.insert(format!("aaa{}", x), 1, daaaa));
                }
            });

            match cache.get(&String::from("adsasd0")).await {
                Some(vec) => {
                    let value = &vec[0];
                    assert_eq!(value.to_string(), vc.to_string())
                }
                _ => assert_eq!(1, 0),
            }

            std::thread::sleep(Duration::from_secs(60));
            match cache.get(&String::from("adsasd0")).await {
                Some(_) => assert_eq!(1, 0),
                None => {}
            }

            assert!(t1.join().is_ok());
            assert!(t2.join().is_ok());

            println!("t1 & t2 completed");

            std::thread::sleep(Duration::from_secs(360));

            let t3 = thread::spawn(move || {
                let _ = vec![
                    String::from("asd"),
                    String::from("hjk"),
                    String::from("poi"),
                ];

                for x in 0i32..6000 {
                    let key = format!("aaa{}", x);

                    match block_on(e.get(&key)) {
                        None => {}
                        _ => assert_eq!(1, 0),
                    }
                }
            });
            let t4 = thread::spawn(move || {
                let _ = vec![
                    String::from("asd"),
                    String::from("hjk"),
                    String::from("poi"),
                ];

                for x in 6000i32..12000 {
                    let key = format!("aaa{}", x);

                    match block_on(f.get(&key)) {
                        None => {}
                        _ => assert_eq!(1, 0),
                    }
                }
            });
            assert!(t3.join().is_ok());
            assert!(t4.join().is_ok());

            println!("t3 & t4 completed");

            while threads.len() > 0 {
                assert!(threads.pop().unwrap().join().is_ok());
            }

            println!("silly threads completed");
            std::thread::sleep(Duration::from_secs(5));

            for i in 0i32..1000 {
                let fff = cache.clone();
                let _ = vec![
                    String::from("asd"),
                    String::from("hjk"),
                    String::from("poi"),
                ];

                for x in 0i32..10000 {
                    let c = if i % 2 == 0 { 10000 - x } else { x };

                    match fff.get(&format!("1aaaddccc{}", c)).await {
                        Some(_) => assert_eq!(1, 0),
                        None => {}
                    };
                }
            }

            println!("clean up completed");
            println!("It took {:?} seconds", start.elapsed().unwrap().as_secs());

            for i in 0..1000 {
                let fff = cache.clone();
                let _ = vec![
                    String::from("asd"),
                    String::from("hjk"),
                    String::from("poi"),
                ];

                for x in 0..10000 {
                    let c = if i % 2 == 0 { 10000 - x } else { x };

                    match fff.get(&format!("1aaaddccc{}", c)).await {
                        Some(_) => assert_eq!(1, 0),
                        None => {}
                    };
                }
            }
            println!("double clean up completed");
        }
    }
}
