use std::collections::{HashMap,HashSet};
use chrono::{DateTime, Utc, Duration};
use std::sync::{Arc, RwLock, Mutex};
use std::hash::Hash;
use std::rc::Rc;
use std::convert::TryInto;
use std::marker::Send;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{thread, time};
use std::sync::mpsc::{Receiver, Sender, channel};
use rand::Rng;

pub struct Cache<K1: 'static + Hash + Eq + Send + Sync, K2: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> {
    store: Arc<RwLock<HashMap<Arc<K1>, Vec<(K2, DateTime<Utc>, Arc<T>)>>>>,
    read_ops: AtomicUsize,
    write_ops: AtomicUsize,
    expired_ops: AtomicUsize,
    added_entry_sender: Mutex<Sender<Arc<K1>>>,
    thread_completed_receiver: Mutex<Receiver<()>>,
    stop_loop_sender: Mutex<Sender<()>>
}

impl<K1: 'static + Hash + Eq + Send + Sync, K2: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> Drop for Cache<K1, K2, T> {
    fn drop(&mut self) {
        println!("Dropping");

        self.stop_loop_sender.lock().unwrap().send(());
        
        match self.thread_completed_receiver.lock().unwrap().recv() {
            Ok(_) => println!("Cleanup thread terminated"),
            Err(_) => println!("GAAAAAAAAAAAAAAAAAAAAAAAA")
        }
    }
}

impl<K1: 'static + Hash + Eq + Send + Sync, K2: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> Cache<K1, K2, T> {
    pub fn store(&self) -> Arc<RwLock<HashMap<Arc<K1>, Vec<(K2, DateTime<Utc>, Arc<T>)>>>> {
        self.store.clone()
    }

    pub fn new() -> Arc<Cache<K1, K2, T>> {
        let store = Arc::new(RwLock::new(HashMap::new()));
        let (thread_completed_sender, thread_completed_receiver) = channel();
        let (added_entry_sender, added_entry_receiver) = channel();
        let (stop_loop_sender, stop_loop_receiver) = channel();

        let cache = Cache {
            store: store.clone(),
            read_ops: AtomicUsize::new(0),
            write_ops: AtomicUsize::new(0),
            expired_ops: AtomicUsize::new(0),
            added_entry_sender: Mutex::new(added_entry_sender),
            thread_completed_receiver: Mutex::new(thread_completed_receiver),
            stop_loop_sender: Mutex::new(stop_loop_sender)
        };

        let result = Arc::new(cache);

        Cache::start_cleanup_thread(store, stop_loop_receiver, thread_completed_sender, added_entry_receiver);
        
        result
    }

    fn start_cleanup_thread(
            store: Arc<RwLock<HashMap<Arc<K1>, Vec<(K2, DateTime<Utc>, Arc<T>)>>>>,
            stop_loop_receiver: Receiver<()>,
            thread_completed_sender: Sender<()>,
            added_entry_receiver: Receiver<Arc<K1>>)
    {
        thread::spawn(move || {
            {
                let mut keys = HashSet::<Arc<K1>>::new();
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
                            _           => break
                        };
                    }

                    let total_keys = {
                        store.read().unwrap().keys().len()
                    };

                    println!("# of keys: {} (keys in hashmap {})", keys.len(), total_keys);
                    let wait = if keys.len() > 0 {
                        let now = Utc::now();
                        let (start_ix, keys_to_take) = if keys.len() <= 20 {
                            (0, keys.len())
                        } else {
                            let count = 19 + keys.len() / 20;
                            (rng.gen_range(0, keys.len() - count), count)
                        };

                        let mut remove_indeces = Vec::<Arc<K1>>::new();
                        for key in keys.iter().skip(start_ix).take(keys_to_take)
                        {
                            match store.read().unwrap().get(key) {
                                Some(entries) if entries.iter().any(|(_, d, _)| d < &now) => remove_indeces.push(key.clone()),
                                Some(entries) if entries.len() == 0 => remove_indeces.push(key.clone()),
                                None => remove_indeces.push(key.clone()),
                                _ => { }
                            }
                        }

                        let cleaned = remove_indeces.len();

                        for key in remove_indeces.iter() {
                            let mut s = store.write().unwrap();
                            match s.get_mut(key) {
                                Some(vec) => {
                                    vec.retain(|(_, d, _)| d > &now);
                                    if vec.len() == 0 {
                                        s.remove(key);
                                        keys.remove(key);
                                    }
                                },
                                None => { keys.remove(key); }
                            };
                        }

                        println!("Clean up completed at {:?}. Removed {} entries", now, cleaned);

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
                    Err(_) => println!("Failed to send completed")
                };
            }

            println!("Memory freed 2 {:?}", Utc::now());
        });
    }

    pub fn insert(&self, key: K1, sub_key: K2, duration_seconds: u16, value: T) {
        let now = Utc::now();
        let expiry_date = now + Duration::seconds(duration_seconds.try_into().unwrap());

        let mut store = self.store.write().unwrap();
        match store.get_mut(&key) {
            Some(vec) => {
                vec.retain(|(k2, expiry_date, _)| expiry_date > &now && k2 != &sub_key);
                vec.push((sub_key, expiry_date, Arc::new(value)));
            },
            None => {
                let k = Arc::new(key);
                store.insert(k.clone(), vec!((sub_key, expiry_date, Arc::new(value))));

                match self.added_entry_sender.lock().unwrap().send(k) {
                    Ok(_) => { },
                    Err(e) => println!("{:?}", e)
                };
            }
        };

        self.write_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self, key: &K1) -> Option<Vec<Arc<T>>> {
        let now = Utc::now();

        let (result, cleanup) = match self.store.read().unwrap().get(key) {
            Some(vec)  => {
                let r = vec.iter()
                    .filter(|(_, d, _)| d > &now)
                    .map(|(_, _, v)| v.clone())
                    .collect::<Vec<_>>();

                let new_len = r.len();
                if new_len > 0 {
                    (Some(r), new_len < vec.len())
                } else {
                    (None, true)
                }
            },
            None       => (None, false)
        };

        if cleanup {
            let lock = self.store.write();
            let mut cache = lock.unwrap();
            match cache.get_mut(key) {
                Some(vec) => {
                    vec.retain(|(_, expiry_date, _)| expiry_date > &now);
                    if vec.len() == 0 {
                        cache.remove(key);
                        self.expired_ops.fetch_add(1, Ordering::Relaxed);    
                    }
                }
                _ => { }
            }
        }

        self.read_ops.fetch_add(1, Ordering::Relaxed);

        result
    }

    pub fn get_ops_count(&self) -> (usize, usize, usize) {
        ( self.read_ops.fetch_add(0, Ordering::Relaxed), self.expired_ops.fetch_add(0, Ordering::Relaxed), self.write_ops.fetch_add(0, Ordering::Relaxed) )
    }
}