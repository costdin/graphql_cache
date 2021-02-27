extern crate r2d2_redis;

use std::ops::DerefMut;
use r2d2::Pool;
use r2d2_redis::{r2d2, redis, RedisConnectionManager};
use r2d2_redis::redis::{Commands, RedisError, FromRedisValue, RedisResult, ToRedisArgs};
//use super::cache::Cache;
use chrono::{Duration, Utc, DateTime};
//use redis::{RedisError, FromRedisValue, RedisResult, ToRedisArgs};
use serde_json::{json, Value, to_string, from_str};
use std::cmp::Ordering;
use std::sync::Arc;
use std::convert::TryInto;
use serde::{Serialize, Deserialize};
//use redis::AsyncCommands;
//use redis::aio::Connection;
/*use redis::Commands;
use redis::Connection;
use redis::Client;
*/
pub struct RedisCache {
    inner_cache: InternalRedisCache
}

impl RedisCache {
    pub fn new(url: &str) -> Result<RedisCache, RedisCacheError> {
        let manager = RedisConnectionManager::new(url).unwrap();
        let pool = r2d2::Pool::builder()
            .build(manager)
            .unwrap();

        //let client = redis::Client::open(url)?; //"redis://127.0.0.1/")?;
        let inner_cache = InternalRedisCache {
            connection_pool: pool.clone()
        };

        Ok(RedisCache { inner_cache: inner_cache })
    }


    pub fn insert(&self, key: String, duration_seconds: u16, value: Value) {
        self.inner_cache.insert(key, duration_seconds, value);
    }

    pub fn get(&self, key: &String) -> Option<Vec<Value>> {

        match self.inner_cache.get(key) {
            Ok(r) => match r {
                None => None,
                Some(v) => {
                    /*let res = v.into_iter()
                               .map(|v| *v.clone())
                               .collect::<Vec<_>>();
                    */
                    Some(v)
                }
            },
            _ => None
        }
    }
}

impl Clone for RedisCache {
    fn clone(&self) -> RedisCache {
        RedisCache {
            inner_cache: InternalRedisCache { 
                connection_pool: self.inner_cache.connection_pool.clone()
            }
        }
    }
}

struct InternalRedisCache {
    pub connection_pool: Pool<RedisConnectionManager>,
}

#[derive(Serialize, Deserialize)]
struct InternalCacheItem {
    pub expiry_date_utc: u64,
    pub value: Value
}

/*
impl ToRedisArgs for InternalCacheItem {
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + RedisWrite,
    {

    }
}

impl FromRedisValue for InternalCacheItem {
    fn from_redis_value(v: &Value) -> RedisResult<InternalCacheItem> {
        match *v {
            Value::Object(map) => {
                let expiry : u64 = match map["expiry_date_utc"] {
                    Value::Number(n) => n.as_u64().unwrap_or(0),
                    _ => 0
                };

                Ok(InternalCacheItem{
                    expiry_date_utc: expiry,
                    value: map["value"]
                })
            },
            _ => Ok(InternalCacheItem {
                expiry_date_utc: 0,
                value: Value::Null
            }),
        }
    }
}*/

pub enum RedisCacheError {
    CreateError
}

impl From<RedisError> for RedisCacheError {
    fn from(_err: RedisError) -> RedisCacheError {
        RedisCacheError::CreateError
    }
}

impl InternalRedisCache {
    fn insert(&self, key: String, duration_seconds: u16, value: Value) -> Result<(), RedisCacheError> {
        if value == Value::default() {
            return Ok(());
        }
        
        let now: u64 = Utc::now().timestamp().try_into().unwrap();
        let offset: u64 = duration_seconds.try_into().unwrap();

        let item = InternalCacheItem {
            expiry_date_utc: now + offset,
            value: value
        };
        let json = serde_json::to_string(&item).unwrap();

        let mut conn = match self.connection_pool.get() {
            Ok(c) => c,
            Err(e) => { println!("{}", e); return Ok(()); }
        };

        //let res: RedisResult<redis::Value> = redis::pipe()
        //    .cmd("AUTH").arg("pass")
        //    .cmd("LPUSH").arg(key).arg(json)
        //    .query(conn.deref_mut());
        //redis::cmd("AUTH").arg("pass").execute(&mut conn);

        let res: RedisResult<redis::Value> = conn.lpush(key, json);

        match res {
            Ok(r) => { },
            Err(e) => println!("{}", e)
        };

        Ok(())
    }

    /*
    fn insert(&self, key: String, duration_seconds: u16, value: Value) -> Result<(), RedisCacheError> {
        let now: u64 = Utc::now().timestamp().try_into().unwrap();
        let offset: u64 = duration_seconds.try_into().unwrap();

        let item = InternalCacheItem {
            expiry_date_utc: now + offset,
            value: value
        };
        let json = serde_json::to_string(&item).unwrap();

        let conn = self.client.get_connection();
        let mut c = match conn {
            Ok(c) => c,
            Err(e) => { println!("{:#?}", e); return Err(RedisCacheError::CreateError); }
        };

        let res = c.lpush(key, json);
        
        match res {
            Ok(_) => (),
            Err(e) => println!("{:#?}", e)
        };

        Ok(())
    }*/

    fn get(&self, key: &String) -> Result<Option<Vec<Value>>, RedisCacheError>{
        let now = Utc::now().timestamp().try_into().unwrap();
        let mut conn = self.connection_pool.get().unwrap();

        //let vec: Vec<String> = redis::pipe()
        //    .cmd("AUTH").arg("pass")
        //    .cmd("LRANGE").arg(key).arg(0).arg(1)
        //    .query(connection.deref_mut())?;

        let vec: Vec<String> = conn.lrange(key, 0, -1)?;
        let dvec = vec.iter().map(|e| serde_json::from_str(e).unwrap()).collect::<Vec<InternalCacheItem>>();

        let mut remove_ix = Vec::new();
        let mut vcccc = Vec::new();
        for (ix, item) in dvec.into_iter().enumerate() {
            if item.expiry_date_utc > now {
                vcccc.push(item);
            } else {
                remove_ix.push(ix);
            }
        }

        if vcccc.len() == 0 {
            conn.del(key)?;
            Ok(None)
        } else {
            if remove_ix.len() > 0 {
                let mut p = redis::pipe();
                p.cmd("AUTH").arg("pass");

                for ix in remove_ix {
                    p.cmd("LSET").arg(key).arg(ix).arg("{}");
                }
                p.cmd("LREM").arg(key).arg(0).arg("{}");

                p.query(conn.deref_mut())?;
            }

            let res = vcccc.into_iter()
                .map(|item| item.value)
                .collect::<Vec<_>>();

            Ok(Some(res))
        }
    }
}
