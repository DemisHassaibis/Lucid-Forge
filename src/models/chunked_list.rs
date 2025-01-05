use super::types::{FileOffset, Item};
use std::sync::{Arc, RwLock};

pub const CHUNK_SIZE: usize = 5;

pub trait SyncPersist {
    fn set_persistence(&self, flag: bool);
    fn needs_persistence(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct LazyItem<T: Clone> {
    data: Option<Arc<RwLock<T>>>,
    offset: Option<FileOffset>,
    decay_counter: usize,
}

impl<T: Clone> LazyItem<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: Some(Arc::new(RwLock::new(data))),
            offset: None,
            decay_counter: 0,
        }
    }

    pub fn with_offset(data: T, offset: FileOffset) -> Self {
        Self {
            data: Some(Arc::new(RwLock::new(data))),
            offset: Some(offset),
            decay_counter: 0,
        }
    }

    pub fn get_data(&self) -> Option<T> {
        self.data.as_ref().map(|arc| arc.read().unwrap().clone())
    }

    pub fn set_data(&mut self, data: T) {
        self.data = Some(Arc::new(RwLock::new(data)));
    }

    pub fn set_offset(&mut self, offset: Option<FileOffset>) {
        self.offset = offset;
    }

    pub fn increment_decay(&mut self) {
        self.decay_counter += 1;
    }

    pub fn reset_decay(&mut self) {
        self.decay_counter = 0;
    }
}

#[derive(Debug, Clone)]
pub struct LazyItemRef<T: Clone> {
    item: Arc<RwLock<LazyItem<T>>>,
}

impl<T: Clone> LazyItemRef<T> {
    pub fn new(data: T) -> Self {
        Self {
            item: Arc::new(RwLock::new(LazyItem::new(data))),
        }
    }

    pub fn with_offset(data: T, offset: FileOffset) -> Self {
        Self {
            item: Arc::new(RwLock::new(LazyItem::with_offset(data, offset))),
        }
    }

    pub fn get_data(&self) -> Option<T> {
        self.item.read().unwrap().get_data()
    }

    pub fn set_data(&self, data: T) {
        self.item.write().unwrap().set_data(data);
    }

    pub fn set_offset(&self, offset: Option<FileOffset>) {
        self.item.write().unwrap().set_offset(offset);
    }

    pub fn increment_decay(&self) {
        self.item.write().unwrap().increment_decay();
    }

    pub fn reset_decay(&self) {
        self.item.write().unwrap().reset_decay();
    }
}

#[derive(Debug, Clone)]
pub struct LazyItems<T: Clone> {
    items: Arc<RwLock<Vec<LazyItem<T>>>>,
}

impl<T: Clone> LazyItems<T> {
    pub fn new() -> Self {
        Self {
            items: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn push(&self, item: LazyItem<T>) {
        self.items.write().unwrap().push(item);
    }

    pub fn get(&self, index: usize) -> Option<LazyItem<T>> {
        self.items.read().unwrap().get(index).cloned()
    }

    pub fn len(&self) -> usize {
        self.items.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.read().unwrap().is_empty()
    }

    pub fn iter(&self) -> Vec<LazyItem<T>> {
        self.items.read().unwrap().clone()
    }
}
