mod graphql_parser;

use warp::Filter;
use std::net::SocketAddr;
use std::time::Duration;
use std::convert::Infallible;


#[tokio::main]
async fn main() {
    let stri = String::from("                          {    field1  (p1 :                         1,         p2:\"as        \\\"      d              \"    )     {      subf1      subf2(  p3   :0)   { s     } }}      ");
    let rezult = graphql_parser::parse_query(stri);
    match rezult {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri2 = String::from("{ggggg: field1(p1:{v1:1,v2:\"2\",v3:{vv3:33},v4:[12,13,15]},p2:\"as        \\\"      d              \"){f1: subf1 subf2(p3:0){s}}, cccc: field1(p1:1){subf1}}");
    let rezult2 = graphql_parser::parse_query(stri2);
    match rezult2 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri3 = String::from("query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } }");
    let rezult3 = graphql_parser::parse_query(stri3);
    match rezult3 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri4 = String::from("query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } }");
    let rezult4 = graphql_parser::parse_query(stri4);
    match rezult4 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri5 = String::from("query TheQuery($p1: Int = 10){alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }");
    let rezult5 = graphql_parser::parse_query(stri5);
    match rezult5 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri6 = String::from("query TheQuery{alias1: field1(id: $p1) { id, name } alias2: field2(id: $p1, name: \"the name\") { id name } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) { id, name surname } alias2: field2(id: $p1, name: \"the second name\") { id name surname } }");
    let rezult6 = graphql_parser::parse_query(stri6);
    match rezult6 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let stri7 = String::from("query TheQuery{alias1: field1(id: $p1) { dob ...userFragment } alias2: field2(id: $p1, name: \"the name\") {...userFragment } } query TheSecondQuery($p1: Int = 20){alias1: field1(id: $p1) {...userFragment surname } alias2: field2(id: $p1, name: \"the second name\") {...userFragment surname } } fragment userFragment on User { id name }");
    let rezult7 = graphql_parser::parse_query(stri7);
    match rezult7 {
        Ok(ast) => println!("{:?}", ast),
        Err(e)  => println!("{:?}", e)
    }

    let option = graphql_parser::parse_query(String::from("{field1(p1:1,p2:\"as              d              \"){subf1 subf2(p3:0){s}}}"));

    match option {
        Ok(ast) => { println!("{:?}", ast); },
        Err(e)  => { println!("{:?}", e); }
    }

    let option2 = graphql_parser::parse_query(String::from("  {    field1 (    p1 : 1   ,  p2    :   \"asd\" ) {    subf1   subf2   (    p3 :  0 )   {   s  }  }    } "));

    match option2 {
        Ok(ast) => { println!("{:?}", ast); },
        Err(e)  => { println!("{:?}", e); }
    }

    let routes = warp::path("hello")
        .and(warp::path::param())
        .and(warp::header("user-agent"))
        .and(warp::addr::remote())
        .and(warp::body::bytes())
        .and_then(stuff);

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn stuff(param: String, agent: String, addr_opt: Option<SocketAddr>, body: bytes::Bytes) -> Result<impl warp::Reply, Infallible> {
    tokio::time::delay_for(Duration::from_secs(4)).await;

    let result = match addr_opt {
        Some(addr) => format!("Hello {}, whose agent is {}, ip {}. {:?} {}", param, agent, addr, body, body[0]),
        None       => format!("Hello {}, whose agent is {}. {:?} {}", param, agent, body, body[0])
    };

    Ok(result)
}