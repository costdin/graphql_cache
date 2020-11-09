use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use std::sync::{Arc, RwLock, Mutex};
use std::hash::Hash;
use std::convert::TryInto;
use std::marker::Send;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::{thread, time};
use std::sync::mpsc::{Receiver, Sender, channel};
use rand::Rng;
use rand::seq::SliceRandom;
use std::cmp;

pub struct Cache<K: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> {
    store: Arc<RwLock<HashMap<K, (DateTime<Utc>, Arc<T>)>>>,
    read_ops: AtomicUsize,
    write_ops: AtomicUsize,
    expired_ops: AtomicUsize,
    added_entry_sender: Mutex<Sender<K>>,
    thread_completed_receiver: Mutex<Receiver<()>>,
    stop_loop_sender: Mutex<Sender<()>>
}

impl<K: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> Drop for Cache<K, T> {
    fn drop(&mut self) {
        println!("Dropping");

        self.stop_loop_sender.lock().unwrap().send(());
        
        match self.thread_completed_receiver.lock().unwrap().recv() {
            Ok(_) => println!("Cleanup thread terminated"),
            Err(_) => println!("GAAAAAAAAAAAAAAAAAAAAAAAA")
        }
    }
}

impl<K: 'static + Hash + Eq + Sync + Send + Clone, T: 'static + Sync + Send> Cache<K, T> {

    pub fn new() -> Arc<Cache<K, T>> {
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
            _store: Arc<RwLock<HashMap<K, (DateTime<Utc>, Arc<T>)>>>,
            stop_loop_receiver: Receiver<()>,
            thread_completed_sender: Sender<()>,
            added_entry_receiver: Receiver<K>)
    {
        thread::spawn(move || {
            {
                let store = _store;
                let mut keys = Vec::<K>::new();
                let mut rng = rand::thread_rng();

                loop {
                    if let Ok(_) = stop_loop_receiver.try_recv() {
                        break;
                    }

                    let now = Utc::now();
                    println!("Start clean up at {:?}", now);

                    let end = Utc::now() + Duration::seconds(1);
                    while Utc::now() < end {
                        match added_entry_receiver.try_recv() {
                            Ok(new_key) => keys.push(new_key),
                            _           => break
                        };
                    }

                    println!("# of keys: {}", keys.len());
                    let wait = if keys.len() > 0 {
                        let now = Utc::now();
                        let keys_to_take = cmp::min(keys.len() / 10, 1000);
                        let start_ix = rng.gen_range(0, keys.len() - keys_to_take);

                        let mut remove_indeces = Vec::<(bool, usize)>::new();
                        for pos in start_ix..(start_ix + keys_to_take)
                        {
                            match store.read().unwrap().get(&keys[pos]) {
                                Some((d, _)) if d < &now => remove_indeces.push((true, pos)),
                                None                     => remove_indeces.push((false, pos)),
                                _    => { }
                            }
                        }

                        let cleaned = remove_indeces.len();

                        for &(remove_from_store, index) in remove_indeces.iter().rev() {
                            if remove_from_store {
                                store.write().unwrap().remove(&keys[index]);
                            }

                            keys.remove(index);
                        }

                        println!("Clean up completed at {:?}. Removed {} entries", now, cleaned);

                        cleaned < keys_to_take / 20
                    } else {
                        false
                    };

                    if wait {
                        thread::sleep(time::Duration::from_millis(1000));
                    } else {
                        println!("Cleaned more than 5% of keys, don't wait");
                    }
                }

                println!("Start memory free {:?}", Utc::now());
                //keys.clear();
                //store.write().unwrap().clear();
                //println!("Memory freed 1 {:?}", Utc::now());

                match thread_completed_sender.send(()) {
                    Ok(_) => println!("Sent completed"),
                    Err(_) => println!("Failed to send completed")
                };
            }

            println!("Memory freed 2 {:?}", Utc::now());
        });
    }

    pub fn insert(&self, key: K, duration_seconds: u16, value: T) -> Option<Arc<T>> {
        let expiry_date = Utc::now() + Duration::seconds(duration_seconds.try_into().unwrap());
        match self.added_entry_sender.lock().unwrap().send(key.clone()) {
            Ok(_) => { },
            Err(e) => println!("{:?}", e)
        };
        let old_value = self.store.write().unwrap().insert(key, (expiry_date, Arc::new(value)));

        self.write_ops.fetch_add(1, Ordering::Relaxed);

        match old_value {
            Some((_, v)) => Some(v),
            None       => None
        }
    }

    pub fn get(&self, key: &K) -> Option<Arc<T>> {
        let (result, cleanup) = match self.store.read().unwrap().get(key) {
            Some((d, _)) if d < &mut Utc::now() => (None, true),
            Some((_, v))                        => (Some(v.clone()), false),
            None                                => (None, false)
        };

        if cleanup {
            let lock = self.store.write();
            let mut cache = lock.unwrap();
            match cache.get(key) {
                Some((d, _)) if d < &mut Utc::now() => { 
                    cache.remove(key);
                    self.expired_ops.fetch_add(1, Ordering::Relaxed);
                },
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