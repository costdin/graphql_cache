mod graphql;
mod graphql_deserializer;
use hyper::{Client, Uri};

use graphql_deserializer::CacheScope;
use rand::Rng;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Future;
use std::net::SocketAddr;
use std::str;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use warp::Filter;

fn vv(v: D) {
    println!("{:#?}", v);
}

#[derive(Debug)]
struct D {
    ccc: String,
}

#[tokio::main]
async fn main() {
    let data = r#"
    {
        "data": 12
    }"#;

    //let c: Value = serde_json::from_str(data).unwrap();
    //let c = D {
    //    ccc: "ADSSDSAD".to_string(),
    //};
    //let mut v = Vec::<D>::new();
    //vv(c);
    //v.push(c);

    match test_parser() {
        s if s == "Ok" => println!("parser test passed"),
        s => println!("parser test failed: {}", s),
    };

    match test_cache_update().await {
        Ok(()) => println!("cache update test passed"),
        Err(s) => println!("cache update test failed"),
    }

    match test_things().await {
        Ok(()) => println!("things test passed"),
        Err(s) => println!("things test failed"),
    }

    match test_deserializer() {
        Ok(_) => println!("deserializer test passed"),
        Err(s) => println!("deserializer test failed: {}", s),
    };

    match test_cache_small() {
        s if s == "Ok" => println!("small cache test passed"),
        s => println!("small cache test failed: {}", s),
    };

    match test_cache_cleanup() {
        s if s == "Ok" => println!("small cache test passed"),
        s => println!("small cache test failed: {}", s),
    };

    match test_cache() {
        s if s == "Ok" => println!("cache test passed"),
        s => println!("cache test failed: {}", s),
    };

    /*    let cache = graphql::cache::create_cache();

    let routes = warp::path("hello")
        .and(warp::path::param())
        .and(warp::header("user-agent"))
        .and(warp::addr::remote())
        .and(warp::body::json())
        .and_then(move |a, b, c, d| stuff(a, b, c, d, cache.clone()));

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await; */
}

pub struct the_struct<'a> {
    internal_string: &'a str,
}

pub async fn function_doing_things<'a, Fut>(
    the_struct: the_struct<'a>,
    get_fn: impl Fn(the_struct<'a>) -> Fut,
) -> i8
where
    Fut: Future<Output = (Value, the_struct<'a>)>,
{
    // do things to the_struct

    let (some_result, one_struct) = get_fn(the_struct).await;

    // use the_struct somehow

    9
}

fn test_parser() -> String {
    let stri = "                          {    field1  (p1 :                         1,         p2:\"as        \\\"      d              \"    )     {      subf1      subf2(  p3   :0)   { s     } }}      ";
    let rezult = graphql::parser::parse_query(stri);
    match rezult {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri2 = "{ggggg: field1(p1:{v1:1,v2:\"2\",v3:{vv3:33},v4:[12,13,15]},p2:\"as        \\\"      d              \"){f1: subf1 subf2(p3:0){s}}, cccc: field1(p1:1){subf1}}";
    let rezult2 = graphql::parser::parse_query(stri2);
    match rezult2 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri3 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } }";
    let rezult3 = graphql::parser::parse_query(stri3);
    match rezult3 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri4 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } }";
    let rezult4 = graphql::parser::parse_query(stri4);
    match rezult4 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri5 = "query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }";
    let rezult5 = graphql::parser::parse_query(stri5);
    match rezult5 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri6 = "query TheQuery{alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }";
    let rezult6 = graphql::parser::parse_query(stri6);
    match rezult6 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri7 = "query TheQuery{alias1: field1(id: $p1) { dob ...userFragment } alias2: field2(id: $p1, name: \"the name\") {...userFragment } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) {...userFragment surname } alias2: field2(id: $p1, name: \"the second name\") {...userFragment surname } } fragment userFragment on User { id name }";
    let rezult7 = graphql::parser::parse_query(stri7);
    match rezult7 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri8 = "{field1(p1:1,p2:\"as              d              \"){subf1 subf2(p3:0){s}}}";
    let result8 = graphql::parser::parse_query(stri8);
    match result8 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    let stri9 = "  {    field1 (    p1 : 1   ,  p2    :   \"asd\" ) {    subf1   subf2   (    p3 :  0 )   {   s  }  }    } ";
    let result9 = graphql::parser::parse_query(stri9);
    match result9 {
        Ok(_ast) => {}
        Err(e) => return format!("{:?}", e),
    }

    return String::from("Ok");
}

fn test_cache_cleanup() -> String {
    let cache = graphql::cache::create_cache();
    //let trtr2 = vec!(String::from("asd1"), String::from("hjk"), String::from("poi"));
    //let trtr = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));
    let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

    cache.insert(format!("xxxx0"), 50000, json.clone());
    for i in 0..1000000 {
        cache.insert(format!("xxxx{}", i), 1, json.clone());
    }

    std::thread::sleep(Duration::from_secs(5));

    let c = cache.clone();
    let thread = thread::spawn(move || {
        let mut rng = rand::thread_rng();

        for _ in 0..1000000 {
            let i = rng.gen_range(0, 1000000);
            match c.get(&format!("xxxx{}", i)) {
                Some(_) => {}
                None => {}
            }
        }
    });

    std::thread::sleep(Duration::from_secs(100));

    match cache.get(&format!("xxxx0")) {
        Some(v) => {
            println!("{:?}", v);
        }
        None => println!("000000000000000000"),
    };

    thread.join();

    let (read, expired, write) = cache.get_ops_count();
    println!("read: {} - expired: {} - write {}", read, expired, write);

    String::from("Ok")
}

fn test_cache_small() -> String {
    let cache = graphql::cache::create_cache();
    //let trtr = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));
    let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

    cache.insert(format!("1aaaddccc{}", 0), 10000, json.clone());
    cache.insert(format!("1aaa{}{}", 1, 0), 10000, json.clone());

    match cache.get(&format!("1aaa{}{}", 1, 0)) {
        Some(_) => {}
        None => println!("000000000000000000"),
    };

    let (read, expired, write) = cache.get_ops_count();
    println!("read: {} - expired: {} - write {}", read, expired, write);

    String::from("Ok")
}

fn test_cache() -> String {
    let cache = graphql::cache::create_cache();
    //let tttt = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));

    let c = cache.clone();
    let d = cache.clone();
    let e = cache.clone();
    let f = cache.clone();

    let vc: Value = serde_json::from_str(r#"{"a": 1234}"#).unwrap();
    let vd: Value = serde_json::from_str(r#"{"a": 5555}"#).unwrap();

    let start = SystemTime::now();
    cache.insert(String::from("adsasd0"), 010, vc.clone());
    cache.insert(String::from("adsasd1"), 123, vd.clone());
    cache.insert(String::from("adsasd2"), 123, vd.clone());
    cache.insert(String::from("adsasd3"), 123, vd.clone());
    cache.insert(String::from("adsasd4"), 123, vd.clone());
    cache.insert(String::from("adsasd5"), 123, vd.clone());
    cache.insert(String::from("adsasd6"), 123, vd.clone());
    cache.insert(String::from("adsasd7"), 123, vd.clone());

    println!("{{ \"v\": 11 }}");

    let mut threads = Vec::new();
    for i in 0..1000 {
        let fff = cache.clone();
        let ttt = thread::spawn(move || {
            //let trtr = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));
            let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

            for x in 0..10000 {
                let c = if i % 2 == 0 { 10000 - x } else { x };

                fff.insert(format!("1aaaddccc{}", c), 1, json.clone());
                fff.insert(format!("1aaa{}{}", i, c), 100, json.clone());

                match fff.get(&format!("1aaa{}{}", i, c)) {
                    Some(_) => {}
                    None => println!("000000000000000000"),
                };
            }
        });

        threads.push(ttt);
    }

    let t1 = thread::spawn(move || {
        //let trtr = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));
        let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

        for x in 0..10000 {
            let daaaa = json.clone();
            c.insert(format!("aaa{}", x), 1, daaaa);
        }
    });

    let t2 = thread::spawn(move || {
        //let trtr = vec!(String::from("asd"), String::from("hjk"), String::from("poi"));
        let json: Value = serde_json::from_str("{ \"v\": 11 }").unwrap();

        for x in 0..12000 {
            let daaaa = json.clone();
            d.insert(format!("aaa{}", x), 1, daaaa);
        }
    });

    match cache.get(&String::from("adsasd0")) {
        Some(vec) => {
            let value = &vec[0];
            if value.to_string() != vc.to_string() {
                return String::from("NOOOOOOOOOOOOOOOOOAAAAAAAAAAAAAAAAAAAAAAAAAAAA(*(********");
            }
        }
        Some(v) => {
            println!("{:?}", v);
            return String::from("NOOOOOOOOOOOOOOOOOAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        }
        _ => return String::from("NOOOOOOOOOOOOOOOOO"),
    }

    std::thread::sleep(Duration::from_secs(60));
    match cache.get(&String::from("adsasd0")) {
        Some(_) => return String::from("NOOOOOOOOOOOOOOOOO"),
        None => {}
    }

    t1.join();
    t2.join();

    println!("t1 & t2 completed");

    std::thread::sleep(Duration::from_secs(360));

    let t3 = thread::spawn(move || {
        let _ = vec![
            String::from("asd"),
            String::from("hjk"),
            String::from("poi"),
        ];

        for x in 0..6000 {
            let key = format!("aaa{}", x);

            match e.get(&key) {
                None => {}
                _ => println!("GAAAAAAAAAAAAAAAAAAAA1"),
            }
        }
    });
    let t4 = thread::spawn(move || {
        let _ = vec![
            String::from("asd"),
            String::from("hjk"),
            String::from("poi"),
        ];

        for x in 6000..12000 {
            let key = format!("aaa{}", x);

            match f.get(&key) {
                None => {}
                _ => println!("GAAAAAAAAAAAAAAAAAAAA2"),
            }
        }
    });
    t3.join();
    t4.join();

    println!("t3 & t4 completed");

    while threads.len() > 0 {
        threads.pop().unwrap().join();
    }

    println!("silly threads completed");
    std::thread::sleep(Duration::from_secs(5));

    for i in 0..1000 {
        let fff = cache.clone();
        let _ = vec![
            String::from("asd"),
            String::from("hjk"),
            String::from("poi"),
        ];

        for x in 0..10000 {
            let c = if i % 2 == 0 { 10000 - x } else { x };

            match fff.get(&format!("1aaaddccc{}", c)) {
                Some(_) => println!("NANANAANANANANANANANAANAN"),
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

            match fff.get(&format!("1aaaddccc{}", c)) {
                Some(_) => println!("NANANAANANANANANANANAANAN"),
                None => {}
            };
        }
    }
    println!("double clean up completed");

    std::thread::sleep(Duration::from_secs(200));
    let (read, expired, write) = cache.get_ops_count();
    println!("read: {} - expired: {} - write {}", read, expired, write);

    {
        let thestore = cache.store();
        let ccc = thestore.read().unwrap();
        println!(
            "cache capacity: {}, total keys: {}",
            ccc.capacity(),
            ccc.keys().len()
        );
    }
    std::thread::sleep(Duration::from_secs(30));

    String::from("Ok")
}

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

    let result: graphql_deserializer::GraphQLResponse = serde_json::from_str(data)?;
    //let mut hints = Vec::<(graphql_deserializer::CacheScope, u16, Value)>::new();
    let _ = std::collections::HashMap::<String, Value>::new();
    let cache = graphql::cache::create_cache();

    let (response_data, hints) = result.compress_cache_hints();

    let field_name: String = match response_data {
        Value::Object(m) => m.keys().into_iter().nth(0).unwrap().clone(),
        _ => panic!("AAAAAAAAAA"),
    };

    for (value, hint) in hints {
        match (hint.scope, hint.max_age) {
            (CacheScope::PUBLIC, duration) => {
                println!("{:#?}", hint);
                println!("{:#?}", value);

                cache.insert(field_name.clone(), duration, value);
            }
            _ => {}
        }
    }
    return Ok(());
}

async fn test_things() -> Result<(), graphql::parser::Error> {
    let cache = graphql::cache::create_cache();
    cache.insert(
        String::from("f1"),
        1000,
        json!({
            "f1": {
                "f2": 16,
                "f3": 88
            }
        }),
    );

    cache.insert(
        String::from(r#"f1+f4_Parameter { name: "id", value: Scalar("12") }"#),
        1000,
        json!(121212),
    );

    cache.insert(
        String::from(r#"f1+f4_Parameter { name: "id", value: Scalar("13") }"#),
        1000,
        json!(131313),
    );

    let query = "{f1{a: f2 f3 f4(id: 13)}}";
    let parsed_query = graphql::parser::parse_query(query)?;

    let fff = |d| send_request(d);
    match graphql::cache::process_query(parsed_query, cache, send_request).await {
        Ok(r) => println!("{:#?}", r),
        Err(e) => println!("{:?}", e),
    };

    Ok(())
}

async fn test_cache_update() -> Result<(), graphql::parser::Error> {
    let cache = graphql::cache::create_cache();
    let query1 = "{f1{f2 f3 a1: f4(id: 13) a2: f4(id: 11)}}";
    let query2 = "{f1{f2 f3 f4(id: 13) a2: f4(id: 11)}}";
    let query3 = "{f1{f2 f3 f4(id: 13)}}";
    let query4 = "{f1{f2 f3 a22222: f4(id: 11)}}";
    let parsed_query = graphql::parser::parse_query(query1)?;
    let parsed_query2 = graphql::parser::parse_query(query2)?;
    let parsed_query3 = graphql::parser::parse_query(query3)?;
    let parsed_query4 = graphql::parser::parse_query(query4)?;

    match graphql::cache::process_query(parsed_query, cache.clone(), send_request2).await {
        Ok(r) => println!("{:#?}", r),
        Err(e) => println!("{:?}", e),
    };

    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");

    match graphql::cache::process_query(parsed_query2, cache.clone(), send_request2).await {
        Ok(r) => println!("{:#?}", r),
        Err(e) => println!("{:?}", e),
    };

    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");

    match graphql::cache::process_query(parsed_query3, cache.clone(), send_request2).await {
        Ok(r) => println!("{:#?}", r),
        Err(e) => println!("{:?}", e),
    };

    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");
    println!("=================================================");

    match graphql::cache::process_query(parsed_query4, cache, send_request2).await {
        Ok(r) => println!("{:#?}", r),
        Err(e) => println!("{:?}", e),
    };

    Ok(())
}

/*

fn parse<'a, T, NomParser>(input: &'a str, parser: NomParser) -> ParseResult<'a, T> where NomParser: Fn(&'a [u8]) -> IResult<&[u8], T>
fn parse<'a, T, NomParser>(input: &'a str, parser: NomParser) -> ParseResult<'a, T> where NomParser: Fn(&[u8]) -> IResult<&[u8], T> {

*/

async fn send_request<'a>(
    document: graphql::parser::Document<'a>,
) -> (
    Result<Value, graphql::parser::Error>,
    graphql::parser::Document<'a>,
) {
    println!("{:#?}", document);
    tokio::time::delay_for(Duration::from_secs(4)).await;

    let result = Ok(json!(
        {
            "data": {
                "f1": {
                    "f3": 777,
                    "f4": 123
                }
            },
            "extensions": {
                "cacheControl": {
                    "version": 1,
                    "hints": [
                        {
                            "path": ["f1"],
                            "maxAge": 2000
                        },
                        {
                            "path": [ "f1", "f4" ],
                            "maxAge": 100
                        }
                    ]
                }
            }
        }
    ));

    (result, document)
}

async fn send_request2<'a>(
    document: graphql::parser::Document<'a>,
) -> (
    Result<Value, graphql::parser::Error>,
    graphql::parser::Document<'a>,
) {
    println!("{:#?}", document);

    let result = Ok(json!(
        {
            "data": {
                "f1": {
                    "f2": 55,
                    "f3": 777,
                    "a1": 123,
                    "a2": 111
                }
            },
            "extensions": {
                "cacheControl": {
                    "version": 1,
                    "hints": [
                        {
                            "path": ["f1"],
                            "maxAge": 2000
                        }
                    ]
                }
            }
        }
    ));

    (result, document)
}

fn test_borrow() -> String {
    let s1 = String::from("dadas");

    return t1(&s1);
}

fn t1(s: &String) -> String {
    return String::from("asd");
}

async fn stuff(
    param: String,
    agent: String,
    addr_opt: Option<SocketAddr>,
    body: HashMap<String, String>,
    cache: Arc<graphql::cache::cache::Cache<String, Value>>,
) -> Result<impl warp::Reply, Infallible> {
    let query = match body.get("query") {
        Some(q) => match graphql::parser::parse_query(&q) {
            Ok(r) => r,
            Err(_) => return Ok(format!("nein")),
        },
        None => return Ok(format!("no")),
    };

    let result = match graphql::cache::process_query(query, cache, send_request).await {
        Ok(r) => format!("{}", r.to_string()),
        Err(e) => format!("{:?}", e),
    };

    /*
       let result = match addr_opt {
            Some(addr) => format!("Hello {}, whose agent is {}, ip {}. {:?}", param, agent, addr, query),
            None       => format!("Hello {}, whose agent is {}. {:?}", param, agent, query)
        };
    */

    Ok(result)
}
