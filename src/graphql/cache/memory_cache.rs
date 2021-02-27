//use super::cache::Cache;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::hash::Hash;
use std::marker::Send;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::{thread, time};

pub struct MemoryCache {
    inner_cache: Arc<InnerCache<String, Value>>,
}

impl MemoryCache {
    pub fn new() -> MemoryCache {
        MemoryCache {
            inner_cache: InnerCache::new()
        }
    }
    
    pub fn insert(&self, key: String, duration_seconds: u16, value: Value) {
        self.inner_cache.insert(key, duration_seconds, value);
    }

    pub fn get(&self, key: &String) -> Option<Vec<Value>> {
        let r = self.inner_cache.get(key);

        match r {
            None => None,
            Some(v) => {
                let res = v.into_iter()
                           .map(|v| (*v).clone())
                           .collect::<Vec<Value>>();

                Some(res)
            }
        }
    }
}

impl Clone for MemoryCache {
    fn clone(&self) -> Self {
        MemoryCache {
            inner_cache: self.inner_cache.clone()
        }
    }
}

struct InnerCache<K: 'static + Hash + Eq + Send + Sync, T: 'static + Sync + Send> {
    store: Arc<RwLock<HashMap<Arc<K>, Vec<(DateTime<Utc>, Arc<T>)>>>>,
    read_ops: AtomicUsize,
    write_ops: AtomicUsize,
    expired_ops: AtomicUsize,
    added_entry_sender: Mutex<Sender<Arc<K>>>,
    thread_completed_receiver: Mutex<Receiver<()>>,
    stop_loop_sender: Mutex<Sender<()>>,
}

impl<K: 'static + Hash + Eq + Send + Sync, T: 'static + Sync + Send> Drop for InnerCache<K, T> {
    fn drop(&mut self) {
        println!("Dropping");

        match self.stop_loop_sender.lock().unwrap().send(()) {
            Ok(_) => {}
            Err(_) => println!("Error while signaling cleanup thread to stop"),
        }

        match self.thread_completed_receiver.lock().unwrap().recv() {
            Ok(_) => println!("Cleanup thread terminated"),
            Err(_) => println!("GAAAAAAAAAAAAAAAAAAAAAAAA"),
        }
    }
}

impl<K: 'static + Hash + Eq + Send + Sync, T: 'static + Sync + Send> InnerCache<K, T> {
    pub fn store(&self) -> Arc<RwLock<HashMap<Arc<K>, Vec<(DateTime<Utc>, Arc<T>)>>>> {
        self.store.clone()
    }

    pub fn new() -> Arc<InnerCache<K, T>> {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let (thread_completed_sender, thread_completed_receiver) = channel();
        let (added_entry_sender, added_entry_receiver) = channel();
        let (stop_loop_sender, stop_loop_receiver) = channel();

        let cache = InnerCache {
            store: store.clone(),
            read_ops: AtomicUsize::new(0),
            write_ops: AtomicUsize::new(0),
            expired_ops: AtomicUsize::new(0),
            added_entry_sender: Mutex::new(added_entry_sender),
            thread_completed_receiver: Mutex::new(thread_completed_receiver),
            stop_loop_sender: Mutex::new(stop_loop_sender),
        };

        let result = Arc::new(cache);

        InnerCache::start_cleanup_thread(
            store,
            stop_loop_receiver,
            thread_completed_sender,
            added_entry_receiver,
        );

        result
    }

    fn start_cleanup_thread(
        store: Arc<RwLock<HashMap<Arc<K>, Vec<(DateTime<Utc>, Arc<T>)>>>>,
        stop_loop_receiver: Receiver<()>,
        thread_completed_sender: Sender<()>,
        added_entry_receiver: Receiver<Arc<K>>,
    ) {
        thread::spawn(move || {
            {
                let mut keys = HashSet::<Arc<K>>::new();
                let mut rng = rand::thread_rng();

                loop {
                    if let Ok(_) = stop_loop_receiver.try_recv() {
                        break;
                    }

                    let now = Utc::now();
                    println!("Start clean up at {:?}", now);

                    //let end = Utc::now() + Duration::seconds(1);
                    //while end > Utc::now() {
                    loop {
                        match added_entry_receiver.try_recv() {
                            Ok(new_key) => keys.insert(new_key),
                            _ => break,
                        };
                    }

                    let total_keys = { store.read().unwrap().keys().len() };

                    println!("# of keys: {} (keys in hashmap {})", keys.len(), total_keys);
                    let wait = if keys.len() > 0 {
                        let now = Utc::now();
                        let (start_ix, keys_to_take) = if keys.len() <= 20 {
                            (0, keys.len())
                        } else {
                            let count = 19 + keys.len() / 20;
                            (rng.gen_range(0..(keys.len() - count)), count)
                        };

                        let mut remove_indeces = Vec::<Arc<K>>::new();
                        for key in keys.iter().skip(start_ix).take(keys_to_take) {
                            match store.read().unwrap().get(key) {
                                Some(entries) if entries.iter().any(|(d, _)| d < &now) => {
                                    remove_indeces.push(key.clone())
                                }
                                Some(entries) if entries.len() == 0 => {
                                    remove_indeces.push(key.clone())
                                }
                                None => remove_indeces.push(key.clone()),
                                _ => {}
                            }
                        }

                        let cleaned = remove_indeces.len();

                        for key in remove_indeces.iter() {
                            let mut s = store.write().unwrap();
                            match s.get_mut(key) {
                                Some(vec) => {
                                    vec.retain(|(d, _)| d > &now);
                                    if vec.len() == 0 {
                                        s.remove(key);
                                        keys.remove(key);
                                    }
                                }
                                None => {
                                    keys.remove(key);
                                }
                            };
                        }

                        println!(
                            "Clean up completed at {:?}. Removed {} entries",
                            now, cleaned
                        );

                        cleaned < keys_to_take / 20 || cleaned < 10
                    } else {
                        true
                    };

                    if wait {
                        thread::sleep(time::Duration::from_millis(5000));
                    } else {
                        println!("Cleaned more than 5% of keys, don't wait");

                        let capacity = store.read().unwrap().capacity();
                        if capacity > keys.len() * 12 / 10 {
                            println!("Shrinking map");

                            store.write().unwrap().shrink_to_fit();
                            keys.shrink_to_fit();
                        }
                    }
                }

                println!("Start memory free {:?}", Utc::now());

                match thread_completed_sender.send(()) {
                    Ok(_) => println!("Sent completed"),
                    Err(_) => println!("Failed to send completed"),
                };
            }

            println!("Memory freed 2 {:?}", Utc::now());
        });
    }

    pub fn insert(&self, key: K, duration_seconds: u16, value: T) {
        let now = Utc::now();
        let expiry_date = now + Duration::seconds(duration_seconds.try_into().unwrap());

        let mut store = self.store.write().unwrap();
        match store.get_mut(&key) {
            Some(vec) => {
                vec.retain(|(expiry_date, _)| expiry_date > &now);
                vec.push((expiry_date, Arc::new(value)));
            }
            None => {
                let k = Arc::new(key);
                store.insert(k.clone(), vec![(expiry_date, Arc::new(value))]);

                match self.added_entry_sender.lock().unwrap().send(k) {
                    Ok(_) => {}
                    Err(e) => println!("{:?}", e),
                };
            }
        };

        self.write_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self, key: &K) -> Option<Vec<Arc<T>>> {
        let now = Utc::now();

        let (result, cleanup) = match self.store.read().unwrap().get(key) {
            Some(vec) => {
                let r = vec
                    .iter()
                    .filter(|(d, _)| d > &now)
                    .map(|(_, v)| v.clone())
                    .collect::<Vec<_>>();

                let new_len = r.len();
                if new_len > 0 {
                    (Some(r), new_len < vec.len())
                } else {
                    (None, true)
                }
            }
            None => (None, false),
        };

        if cleanup {
            let lock = self.store.write();
            let mut cache = lock.unwrap();
            match cache.get_mut(key) {
                Some(vec) => {
                    vec.retain(|(expiry_date, _)| expiry_date > &now);
                    if vec.len() == 0 {
                        cache.remove(key);
                        self.expired_ops.fetch_add(1, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }

        self.read_ops.fetch_add(1, Ordering::Relaxed);

        result
    }

    pub fn get_ops_count(&self) -> (usize, usize, usize) {
        (
            self.read_ops.fetch_add(0, Ordering::Relaxed),
            self.expired_ops.fetch_add(0, Ordering::Relaxed),
            self.write_ops.fetch_add(0, Ordering::Relaxed),
        )
    }
}
