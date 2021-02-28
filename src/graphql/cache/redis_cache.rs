use ::redis::aio::MultiplexedConnection;
use chrono::Utc;
use redis::AsyncCommands;
use redis::{RedisError, RedisResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value};
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
                Some(v) => {
                    /*let res = v.into_iter()
                               .map(|v| *v.clone())
                               .collect::<Vec<_>>();
                    */
                    Some(v)
                }
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
        if value == Value::default() {
            return Ok(());
        }

        if let Value::Object(map) = &value {
            if map.len() == 0 {
                return Ok(());
            }
        }

        let now: u64 = Utc::now().timestamp().try_into().unwrap();
        let offset: u64 = duration_seconds.try_into().unwrap();

        let item = InternalCacheItem {
            expiry_date_utc: now + offset,
            value: value,
        };
        let json = serde_json::to_string(&item).unwrap();

        let res: RedisResult<redis::Value> = self.connection.clone().lpush(key, json).await;

        match res {
            Ok(r) => {}
            Err(e) => println!("{}", e),
        };

        Ok(())
    }

    async fn get(&self, key: &String) -> Result<Option<Vec<Value>>, RedisCacheError> {
        let now = Utc::now().timestamp().try_into().unwrap();

        let vec: Vec<String> = self.connection.clone().lrange(key, 0, -1).await?;
        let dvec = vec
            .iter()
            .map(|e| serde_json::from_str(e).unwrap())
            .collect::<Vec<InternalCacheItem>>();

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
            self.connection.clone().del(key).await?;
            Ok(None)
        } else {
            if remove_ix.len() > 0 {
                let mut p = redis::pipe();
                p.cmd("AUTH").arg("pass");

                for ix in remove_ix {
                    p.cmd("LSET").arg(key).arg(ix).arg("{}");
                }
                p.cmd("LREM").arg(key).arg(0).arg("{}");

                p.query_async(&mut self.connection.clone()).await?;
            }

            let res = vcccc.into_iter().map(|item| item.value).collect::<Vec<_>>();

            Ok(Some(res))
        }
    }
}
