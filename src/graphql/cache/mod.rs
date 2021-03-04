mod cache;
mod memory_cache;
mod redis_cache;

#[cfg(not(test))]
pub type Cache = RedisCache;
#[cfg(test)]
pub type Cache = MemoryCache;

pub use memory_cache::MemoryCache;
pub use redis_cache::RedisCache;
