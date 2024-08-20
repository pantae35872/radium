use core::fmt::Debug;
use core::{borrow::Borrow, hash::Hash};

use alloc::vec::Vec;

use core::hash::Hasher;

pub const INITIAL_CAPACITY: usize = 8;

pub struct DJB2Hasher {
    state: u64,
}

impl DJB2Hasher {
    pub fn new() -> Self {
        Self { state: 5381 }
    }

    pub fn finish(&self) -> u64 {
        return self.state;
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state = self.state.wrapping_mul(33).wrapping_add(u64::from(byte));
        }
    }
}

impl Hasher for DJB2Hasher {
    fn finish(&self) -> u64 {
        self.finish()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write(bytes);
    }
}

#[derive(PartialEq)]
pub struct HashMap<K, V> {
    buckets: Vec<Vec<(K, V)>>,
    capacity: usize,
    size: usize,
    load_factor: f64,
}

impl<K: Debug, V: Debug> Debug for HashMap<K, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "HashMap: {{ Length: {}, Values: {{ ", self.size())?;
        for bucket in &self.buckets {
            for (key, value) in bucket {
                write!(f, "Key: {:?}, Value: {:?}", key, value)?;
            }
        }
        write!(f, "}} }}")?;
        return Ok(());
    }
}

impl<K, V> HashMap<K, V> {
    pub fn new() -> Self {
        let mut buckets = Vec::with_capacity(INITIAL_CAPACITY);

        for _ in 0..INITIAL_CAPACITY {
            buckets.push(Vec::new());
        }
        Self {
            buckets,
            capacity: INITIAL_CAPACITY,
            size: 0,
            load_factor: 0.75,
        }
    }

    pub fn size(&self) -> usize {
        return self.size;
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut buckets = Vec::with_capacity(capacity);

        for _ in 0..capacity {
            buckets.push(Vec::new());
        }
        Self {
            buckets,
            capacity,
            size: 0,
            load_factor: 0.75,
        }
    }
}

impl<K: Hash, V> HashMap<K, V> {
    pub fn hash<Q: ?Sized>(&self, key: &Q) -> usize
    where
        K: Borrow<Q>,
        Q: Hash,
    {
        let mut hasher = DJB2Hasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.capacity
    }
}

impl<K: Eq + Hash, V> HashMap<K, V> {
    pub fn insert(&mut self, key: K, value: V) {
        if self.size >= (self.capacity as f64 * self.load_factor) as usize {
            self.resize();
        }

        let bucket_index = self.hash(&key);
        let bucket = &mut self.buckets[bucket_index];
        for &mut (ref existing_key, ref mut existing_value) in bucket.iter_mut() {
            if *existing_key == key {
                *existing_value = value;
                return;
            }
        }

        bucket.push((key, value));
        self.size += 1;
    }
}

impl<K: Eq + Hash, V> HashMap<K, V> {
    pub fn exists<Q: ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.get(key).is_some()
    }

    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let bucket_index = self.hash(key);
        let bucket = &self.buckets[bucket_index];

        for &(ref existing_key, ref value) in bucket.iter() {
            if existing_key.borrow() == key {
                return Some(value);
            }
        }
        return None;
    }

    pub fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let bucket_index = self.hash(key);
        let bucket = &mut self.buckets[bucket_index];

        for &mut (ref existing_key, ref mut value) in bucket.iter_mut() {
            if existing_key.borrow() == key {
                return Some(value);
            }
        }
        return None;
    }
}

impl<K: Eq + Hash + Clone, V> HashMap<K, V> {
    pub fn get_or_insert_mut<F>(&mut self, key: &K, insert: F) -> &mut V
    where
        F: Fn() -> V,
    {
        if self.exists(key) {
            return self.get_mut(key).unwrap();
        } else {
            self.insert(key.clone(), insert());
            return self.get_mut(key).unwrap();
        }
    }
}

impl<K: Eq + Hash, V> HashMap<K, V> {
    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let bucket_index = self.hash(key);
        let bucket = &mut self.buckets[bucket_index];

        if let Some(pos) = bucket
            .iter()
            .position(|&(ref existing_key, _)| existing_key.borrow() == key)
        {
            self.size -= 1;
            return Some(bucket.swap_remove(pos).1);
        }
        return None;
    }
}

impl<K: Clone, V: Clone> Clone for HashMap<K, V> {
    fn clone(&self) -> Self {
        HashMap {
            buckets: self.buckets.clone(),
            capacity: self.capacity,
            size: self.size,
            load_factor: self.load_factor,
        }
    }
}

impl<K: Eq + Hash, V> HashMap<K, V> {
    pub fn resize(&mut self) {
        let new_capacity = self.capacity * 2;
        let mut new_buckets = Vec::with_capacity(new_capacity);

        for _ in 0..new_capacity {
            new_buckets.push(Vec::new());
        }

        for bucket in self.buckets.iter_mut() {
            for (key, value) in bucket.drain(..) {
                let mut hasher = DJB2Hasher::new();
                key.hash(&mut hasher);
                let new_index = (hasher.finish() as usize) % new_capacity;
                new_buckets[new_index].push((key, value));
            }
        }

        self.buckets = new_buckets;
        self.capacity = new_capacity;
    }
}
