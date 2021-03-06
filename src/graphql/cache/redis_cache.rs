use ::redis::aio::MultiplexedConnection;
use chrono::Utc;
use redis::AsyncCommands;
use redis::{RedisError, RedisResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::TryInto;

pub struct RedisCache {
    inner_cache: InternalRedisCache,
}

impl RedisCache {
    pub async fn new(url: &str) -> Result<RedisCache, RedisCacheError> {
        let client = redis::Client::open(url)?;
        let connection = client.get_multiplexed_async_connection().await?;

        let inner_cache = InternalRedisCache {
            connection: connection,
        };

        Ok(RedisCache {
            inner_cache: inner_cache,
        })
    }

    pub async fn insert(&self, key: String, duration_seconds: u16, value: Value) {
        self.inner_cache.insert(key, duration_seconds, value).await;
    }

    pub async fn get(&self, key: &String) -> Option<Vec<Value>> {
        match self.inner_cache.get(key).await {
            Ok(r) => match r {
                None => None,
                s @ Some(_) => s,
            },
            _ => None,
        }
    }
}

impl Clone for RedisCache {
    fn clone(&self) -> RedisCache {
        RedisCache {
            inner_cache: InternalRedisCache {
                connection: self.inner_cache.connection.clone(),
            },
        }
    }
}

struct InternalRedisCache {
    pub connection: MultiplexedConnection,
}

#[derive(Serialize, Deserialize)]
struct InternalCacheItem {
    pub expiry_date_utc: u64,
    pub value: Value,
}

pub enum RedisCacheError {
    CreateError,
}

impl From<RedisError> for RedisCacheError {
    fn from(_err: RedisError) -> RedisCacheError {
        RedisCacheError::CreateError
    }
}

impl InternalRedisCache {
    async fn insert(
        &self,
        key: String,
        duration_seconds: u16,
        value: Value,
    ) -> Result<(), RedisCacheError> {
        let now: isize = Utc::now().timestamp().try_into().unwrap();
        let offset: isize = duration_seconds.try_into().unwrap();
        let score = now + offset;
        let json = serde_json::to_string(&value).unwrap();

        let res: RedisResult<redis::Value> = self.connection.clone().zadd(key, json, score).await;

        match res {
            Ok(_) => {}
            Err(e) => println!("{}", e),
        };

        Ok(())
    }

    async fn get(&self, key: &String) -> Result<Option<Vec<Value>>, RedisCacheError> {
        let now: isize = Utc::now().timestamp().try_into().unwrap();
        let (_del_result, get_result): (redis::Value, Vec<String>) = redis::pipe()
            .zrembyscore(key, 0isize, now)
            .zrangebyscore(key, now, "+inf")
            .query_async(&mut self.connection.clone())
            .await?;

        if get_result.len() > 0 {
            let result = get_result
                .iter()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect::<Vec<Value>>();
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }
}
