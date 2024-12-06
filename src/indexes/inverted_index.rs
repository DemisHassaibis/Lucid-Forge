use std::{
    fs::File,
    sync::{Arc, Mutex, RwLock},
};

use arcshift::ArcShift;

use crate::{
    models::{
        buffered_io::BufferManagerFactory,
        types::{DistanceMetric, MetaDb, QuantizationMetric},
        versioning::{Hash, VersionControl},
    },
    quantization::StorageType,
};

use super::inverted_index_item::InvertedIndexItem;

#[allow(dead_code)]
pub(crate) struct InvertedIndex {
    pub name: String,
    pub description: Option<String>,
    pub auto_create_index: bool,
    pub metadata_schema: Option<String>, //object (optional)
    pub max_vectors: Option<i32>,
    pub replication_factor: Option<i32>,
    pub root: Arc<Mutex<InvertedIndexItem>>,
    pub prop_file: Arc<RwLock<File>>,
    pub lmdb: MetaDb,
    pub current_version: ArcShift<Hash>,
    pub current_open_transaction: ArcShift<Option<Hash>>,
    pub quantization_metric: ArcShift<QuantizationMetric>,
    pub distance_metric: Arc<DistanceMetric>,
    pub storage_type: ArcShift<StorageType>,
    pub vcs: Arc<VersionControl>,
    pub vec_raw_manager: Arc<BufferManagerFactory>,
    pub index_manager: Arc<BufferManagerFactory>,
}

#[allow(dead_code)]
impl InvertedIndex {
    pub fn new(
        name: String,
        description: Option<String>,
        auto_create_index: bool,
        metadata_schema: Option<String>,
        max_vectors: Option<i32>,
        replication_factor: Option<i32>,
        prop_file: Arc<RwLock<File>>,
        lmdb: MetaDb,
        current_version: ArcShift<Hash>,
        quantization_metric: ArcShift<QuantizationMetric>,
        distance_metric: Arc<DistanceMetric>,
        storage_type: ArcShift<StorageType>,
        vcs: Arc<VersionControl>,
        vec_raw_manager: Arc<BufferManagerFactory>,
        index_manager: Arc<BufferManagerFactory>,
    ) -> Self {
        InvertedIndex {
            name,
            auto_create_index,
            description,
            max_vectors,
            metadata_schema,
            replication_factor,
            root: Arc::new(Mutex::new(InvertedIndexItem::new(0, false))),
            prop_file,
            lmdb,
            current_version,
            current_open_transaction: ArcShift::new(None),
            quantization_metric,
            distance_metric,
            storage_type,
            vcs,
            vec_raw_manager,
            index_manager,
        }
    }

    pub fn add_dim_index(&self, dim_index: u32, value: f32, vector_id: u32) -> Result<(), String> {
        self.root
            .lock()
            .unwrap()
            .insert_dim_index(dim_index, value, vector_id)
    }

    pub fn print_tree(&self) {
        self.root.lock().unwrap().print_tree(0);
    }

    // Get method
    pub fn get_current_version(&self) -> Hash {
        let mut arc = self.current_version.clone();
        arc.get().clone()
    }

    // Set method
    pub fn set_current_version(&self, new_version: Hash) {
        let mut arc = self.current_version.clone();
        arc.update(new_version);
    }
}
