use lmdb::{Database, Environment, Transaction, WriteFlags};
use serde::{Deserialize, Serialize};
use serde_cbor::{from_slice, to_vec};
use siphasher::sip::SipHasher24;
use std::hash::Hasher;

use super::common::WaCustomError;

#[derive(Deserialize, Clone, Serialize, Debug)]
pub(crate) struct DenseVectorOptions {
    pub enabled: bool,
    pub auto_create_index: bool,
    pub dimension: i32,
}

#[derive(Deserialize, Clone, Serialize, Debug)]
pub(crate) struct SparseVectorOptions {
    pub enabled: bool,
    pub auto_create_index: bool,
}

#[derive(Deserialize, Clone, Serialize, Debug)]
pub(crate) struct CollectionConfig {
    pub max_vectors: Option<i32>,
    pub replication_factor: Option<i32>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Collection {
    pub name: String,
    pub description: Option<String>,
    pub dense_vector: DenseVectorOptions,
    pub sparse_vector: SparseVectorOptions,
    pub metadata_schema: Option<String>, //object (optional)
    pub config: CollectionConfig,
}

impl Collection {
    pub fn new(
        name: String,
        description: Option<String>,
        dense_vector_options: DenseVectorOptions,
        sparse_vector_options: SparseVectorOptions,
        metadata_schema: Option<String>,
        config: CollectionConfig,
    ) -> Self {
        Collection {
            name,
            description,
            dense_vector: dense_vector_options,
            sparse_vector: sparse_vector_options,
            metadata_schema,
            config,
        }
    }

    /// Computes the SipHash of the collection name
    pub fn get_hash(&self) -> u64 {
        let mut hasher = SipHasher24::new();
        hasher.write(self.name.as_bytes());
        let hash = hasher.finish();
        hash
    }

    /// computes the key used to store the collection in the database
    pub fn get_key(&self) -> [u8; 8] {
        let hash = self.get_hash();
        let key = hash.to_le_bytes();
        key
    }

    /// serializes the collection
    pub fn serialize(&self) -> Result<Vec<u8>, WaCustomError> {
        to_vec(self).map_err(|e| WaCustomError::SerializationError(e.to_string()))
    }

    /// perists the collection instance on disk (lmdb -> collections database)
    pub fn persist(&self, env: &Environment, db: Database) -> Result<(), WaCustomError> {
        let key = self.get_key();
        let value = self.serialize()?;

        let mut txn = env
            .begin_rw_txn()
            .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?;

        txn.put(db, &key, &value, WriteFlags::empty())
            .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?;
        txn.commit()
            .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}
