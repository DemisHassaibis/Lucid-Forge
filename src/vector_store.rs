use crate::distance::DistanceFunction;
use crate::models::buffered_io::{BufIoError, BufferManager, BufferManagerFactory};
use crate::models::cache_loader::NodeRegistry;
use crate::models::common::*;
use crate::models::file_persist::*;
use crate::models::identity_collections::IdentitySet;
use crate::models::lazy_load::*;
use crate::models::types::*;
use crate::models::versioning::Hash;
use crate::quantization::Quantization;
use crate::storage::Storage;
use arcshift::ArcShift;
use lmdb::Transaction;
use lmdb::WriteFlags;
use rayon::iter::IntoParallelIterator;
use rayon::iter::ParallelIterator;
use smallvec::SmallVec;
use std::array::TryFromSliceError;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::SeekFrom;
use std::sync::Arc;

pub fn ann_search(
    vec_store: Arc<VectorStore>,
    cache: Arc<NodeRegistry>,
    vector_emb: QuantizedVectorEmbedding,
    cur_entry: LazyItem<MergedNode>,
    cur_level: HNSWLevel,
) -> Result<Option<Vec<(LazyItem<MergedNode>, MetricResult)>>, WaCustomError> {
    let fvec = vector_emb.quantized_vec.clone();
    let mut skipm = HashSet::new();
    skipm.insert(vector_emb.hash_vec.clone());

    let mut cur_node_arc = match cur_entry.clone() {
        LazyItem::Valid {
            data: Some(node), ..
        } => node,
        LazyItem::Valid {
            data: None,
            mut file_index,
            ..
        } => {
            if let Some(file_index) = file_index.get() {
                return Err(WaCustomError::LazyLoadingError(format!(
                    "Node at offset {} needs to be loaded",
                    file_index
                )));
            } else {
                return Err(WaCustomError::NodeError(
                    "Current entry is null".to_string(),
                ));
            }
        }
        _ => {
            return Err(WaCustomError::NodeError(
                "Current entry is null".to_string(),
            ))
        }
    };

    let cur_node = cur_node_arc.get();

    let mut prop_arc = cur_node.prop.clone();
    let prop_state = prop_arc.get();

    let node_prop = match &*prop_state {
        PropState::Ready(prop) => prop,
        PropState::Pending(_) => {
            return Err(WaCustomError::NodeError(
                "Node prop is in pending state".to_string(),
            ))
        }
    };

    let z = traverse_find_nearest(
        vec_store.clone(),
        cache.clone(),
        cur_entry.clone(),
        fvec.clone(),
        vector_emb.hash_vec.clone(),
        0,
        &mut skipm,
        cur_level,
        false,
    )?;

    let dist = vec_store
        .distance_metric
        .calculate(&fvec, &node_prop.value)?;

    let z = if z.is_empty() {
        vec![(cur_entry.clone(), dist)]
    } else {
        z
    };

    let result = if cur_level.0 == 0 {
        Some(vec![])
    } else {
        ann_search(
            vec_store.clone(),
            cache,
            vector_emb.clone(),
            z[0].0.clone(),
            HNSWLevel(cur_level.0 - 1),
        )?
    };

    Ok(add_option_vecs(&result, &Some(z)))
}

pub fn vector_fetch(
    vec_store: Arc<VectorStore>,
    vector_id: VectorId,
) -> Result<Vec<Option<(VectorId, Vec<(VectorId, MetricResult)>)>>, WaCustomError> {
    let mut results = Vec::new();

    for lev in 0..vec_store.max_cache_level {
        let maybe_res = load_vector_id_lsmdb(HNSWLevel(lev), vector_id.clone());
        let neighbors = match maybe_res {
            LazyItem::Valid {
                data: Some(vth), ..
            } => {
                let mut vth = vth.clone();
                let nes: Vec<(VectorId, MetricResult)> = vth
                    .get()
                    .neighbors
                    .iter()
                    .filter_map(|ne| match ne.1.clone() {
                        LazyItem::Valid {
                            data: Some(node), ..
                        } => get_vector_id_from_node(node.clone().get()).map(|id| (id, ne.0)),
                        LazyItem::Valid {
                            data: None,
                            mut file_index,
                            ..
                        } => {
                            if let Some(xloc) = file_index.get() {
                                match load_neighbor_from_db(xloc.clone(), &vec_store) {
                                    Ok(Some(info)) => Some(info),
                                    Ok(None) => None,
                                    Err(e) => {
                                        eprintln!("Error loading neighbor: {}", e);
                                        None
                                    }
                                }
                            } else {
                                None
                                // NonePaste, drop, or click to add files Create pull request
                            }
                        }
                        _ => None,
                    })
                    .collect();
                Some((vector_id.clone(), nes))
            }
            LazyItem::Valid {
                data: None,
                mut file_index,
                ..
            } => {
                if let Some(xloc) = file_index.get() {
                    match load_node_from_persist(xloc.clone(), &vec_store) {
                        Ok(Some((id, neighbors))) => Some((id, neighbors)),
                        Ok(None) => None,
                        Err(e) => {
                            eprintln!("Error loading vector: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        results.push(neighbors);
    }
    Ok(results)
}
fn load_node_from_persist(
    _offset: FileIndex,
    _vec_store: &Arc<VectorStore>,
) -> Result<Option<(VectorId, Vec<(VectorId, MetricResult)>)>, WaCustomError> {
    // Placeholder function to load vector from database
    // TODO: Implement actual database loading logic
    Err(WaCustomError::LazyLoadingError(
        "Not implemented".to_string(),
    ))
}

// fn get_neighbor_info(nbr: &Neighbour) -> Option<(VectorId, f32)> {
//     let Some(node) = nbr.node.data.clone() else {
//         eprintln!("Neighbour node not initialized");
//         return None;
//     };
//     let guard = node.read().unwrap();

//     let prop_state = match guard.prop.read() {
//         Ok(guard) => guard,
//         Err(e) => {
//             eprintln!("Lock error when reading prop: {}", e);
//             return None;
//         }
//     };

//     match &*prop_state {
//         PropState::Ready(node_prop) => Some((node_prop.id.clone(), nbr.cosine_similarity)),
//         PropState::Pending(_) => {
//             eprintln!("Encountered pending prop state");
//             None
//         }
//     }
// }

fn get_vector_id_from_node(node: &MergedNode) -> Option<VectorId> {
    let mut prop_arc = node.prop.clone();
    match prop_arc.get() {
        PropState::Ready(node_prop) => Some(node_prop.id.clone()),
        PropState::Pending(_) => None,
    }
}

fn load_neighbor_from_db(
    _offset: FileIndex,
    _vec_store: &Arc<VectorStore>,
) -> Result<Option<(VectorId, MetricResult)>, WaCustomError> {
    // Placeholder function to load neighbor from database
    // TODO: Implement actual database loading logic
    Err(WaCustomError::LazyLoadingError(
        "Not implemented".to_string(),
    ))
}

pub fn write_embedding(
    bufman: Arc<BufferManager>,
    emb: &RawVectorEmbedding,
) -> Result<u32, WaCustomError> {
    // TODO: select a better value for `N` (number of bytes to pre-allocate)
    let serialized = rkyv::to_bytes::<_, 256>(emb)
        .map_err(|e| WaCustomError::SerializationError(e.to_string()))?;

    let len = serialized.len() as u32;
    let cursor = bufman.open_cursor()?;

    let start = bufman.seek_with_cursor(cursor, SeekFrom::End(0))? as u32;
    bufman.write_u32_with_cursor(cursor, len)?;
    bufman.write_with_cursor(cursor, &serialized)?;

    bufman.close_cursor(cursor)?;

    Ok(start)
}

pub struct EmbeddingOffset {
    pub version: Hash,
    pub offset: u32,
}

impl EmbeddingOffset {
    pub fn serialize(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(8);

        result.extend_from_slice(&self.version.to_le_bytes());
        result.extend_from_slice(&self.offset.to_le_bytes());

        result
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != 8 {
            return Err("Input must be exactly 8 bytes");
        }

        let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let offset = u32::from_le_bytes(bytes[4..8].try_into().unwrap());

        Ok(Self {
            version: Hash::from(version),
            offset,
        })
    }
}

fn read_embedding(
    bufman: Arc<BufferManager>,
    offset: u32,
) -> Result<(RawVectorEmbedding, u32), WaCustomError> {
    let cursor = bufman.open_cursor()?;

    bufman
        .seek_with_cursor(cursor, SeekFrom::Start(offset as u64))
        .map_err(|e| WaCustomError::DeserializationError(e.to_string()))?;

    let len = bufman
        .read_u32_with_cursor(cursor)
        .map_err(|e| WaCustomError::DeserializationError(e.to_string()))?;

    let mut buf = vec![0; len as usize];

    bufman
        .read_with_cursor(cursor, &mut buf)
        .map_err(|e| WaCustomError::DeserializationError(e.to_string()))?;

    let emb = unsafe { rkyv::from_bytes_unchecked(&buf) }.map_err(|e| {
        WaCustomError::DeserializationError(format!("Failed to deserialize VectorEmbedding: {}", e))
    })?;

    let next = bufman
        .cursor_position(cursor)
        .map_err(|e| WaCustomError::DeserializationError(e.to_string()))? as u32;

    bufman.close_cursor(cursor)?;

    Ok((emb, next))
}

pub fn insert_embedding(
    bufman: Arc<BufferManager>,
    vec_store: Arc<VectorStore>,
    emb: &RawVectorEmbedding,
    current_version: Hash,
) -> Result<(), WaCustomError> {
    let env = vec_store.lmdb.env.clone();
    let embedding_db = vec_store.lmdb.embeddings_db.clone();
    let metadata_db = vec_store.lmdb.metadata_db.clone();

    let mut txn = env
        .begin_rw_txn()
        .map_err(|e| WaCustomError::DatabaseError(format!("Failed to begin transaction: {}", e)))?;

    let count_unindexed = match txn.get(*metadata_db, &"count_unindexed") {
        Ok(bytes) => {
            let bytes = bytes.try_into().map_err(|e: TryFromSliceError| {
                WaCustomError::DeserializationError(e.to_string())
            })?;
            u32::from_le_bytes(bytes)
        }
        Err(lmdb::Error::NotFound) => 0,
        Err(err) => return Err(WaCustomError::DatabaseError(err.to_string())),
    };

    let offset = write_embedding(bufman, emb)?;

    let offset = EmbeddingOffset {
        version: current_version,
        offset,
    };
    let offset_serialized = offset.serialize();

    txn.put(
        *embedding_db,
        &emb.hash_vec.to_string(),
        &offset_serialized,
        WriteFlags::empty(),
    )
    .map_err(|e| WaCustomError::DatabaseError(format!("Failed to put data: {}", e)))?;
    let current_version_bytes = current_version.to_le_bytes();

    let should_update_next_version = match txn.get(*metadata_db, &"next_version") {
        Ok(bytes) => bytes != &current_version_bytes,
        Err(lmdb::Error::NotFound) => true,
        Err(err) => {
            return Err(WaCustomError::DatabaseError(err.to_string()));
        }
    };

    if should_update_next_version {
        txn.put(
            *metadata_db,
            &"next_version",
            &current_version_bytes,
            WriteFlags::empty(),
        )
        .map_err(|e| WaCustomError::DatabaseError(format!("Failed to put data: {}", e)))?;
    }

    txn.put(
        *metadata_db,
        &"count_unindexed",
        &(count_unindexed + 1).to_le_bytes(),
        WriteFlags::empty(),
    )
    .map_err(|e| {
        WaCustomError::DatabaseError(format!("Failed to update `count_unindexed`: {}", e))
    })?;

    txn.commit().map_err(|e| {
        WaCustomError::DatabaseError(format!("Failed to commit transaction: {}", e))
    })?;

    Ok(())
}

pub fn index_embeddings(
    vec_store: Arc<VectorStore>,
    cache: Arc<NodeRegistry>,
    upload_process_batch_size: usize,
) -> Result<(), WaCustomError> {
    let env = vec_store.lmdb.env.clone();
    let metadata_db = vec_store.lmdb.metadata_db.clone();

    let txn = env
        .begin_ro_txn()
        .map_err(|e| WaCustomError::DatabaseError(format!("Failed to begin transaction: {}", e)))?;

    let mut count_indexed = match txn.get(*metadata_db, &"count_indexed") {
        Ok(bytes) => {
            let bytes = bytes.try_into().map_err(|e: TryFromSliceError| {
                WaCustomError::DeserializationError(e.to_string())
            })?;
            u32::from_le_bytes(bytes)
        }
        Err(lmdb::Error::NotFound) => 0,
        Err(err) => return Err(WaCustomError::DatabaseError(err.to_string())),
    };

    let mut count_unindexed = match txn.get(*metadata_db, &"count_unindexed") {
        Ok(bytes) => {
            let bytes = bytes.try_into().map_err(|e: TryFromSliceError| {
                WaCustomError::DeserializationError(e.to_string())
            })?;
            u32::from_le_bytes(bytes)
        }
        Err(lmdb::Error::NotFound) => 0,
        Err(err) => return Err(WaCustomError::DatabaseError(err.to_string())),
    };

    let version =
        Hash::from(match txn.get(*metadata_db, &"next_version") {
            Ok(bytes) => u32::from_le_bytes(bytes.try_into().map_err(|e: TryFromSliceError| {
                WaCustomError::DeserializationError(e.to_string())
            })?),
            Err(err) => return Err(WaCustomError::DatabaseError(err.to_string())),
        });
    let version_hash = vec_store
        .vcs
        .get_version_hash(&version)
        .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?
        .expect("Current version hash not found");
    let version_number = *version_hash.version;

    txn.abort();

    let mut index = |embeddings: Vec<RawVectorEmbedding>| -> Result<(), WaCustomError> {
        let results: Vec<()> = embeddings
            .into_par_iter()
            .map(|raw_emb| {
                let lp = &vec_store.levels_prob;
                let iv = get_max_insert_level(rand::random::<f32>().into(), lp.clone());
                let quantized_vec = Arc::new(
                    vec_store
                        .quantization_metric
                        .quantize(&raw_emb.raw_vec, vec_store.storage_type)
                        .expect("Quantization failed"),
                );
                let embedding = QuantizedVectorEmbedding {
                    quantized_vec,
                    hash_vec: raw_emb.hash_vec,
                };

                index_embedding(
                    vec_store.clone(),
                    cache.clone(),
                    None,
                    embedding,
                    vec_store.root_vec.item.clone().get().clone(),
                    HNSWLevel(vec_store.max_cache_level),
                    HNSWLevel(iv.try_into().unwrap()),
                    version,
                    version_number,
                )
                .expect("index_embedding failed");
            })
            .collect();

        let batch_size = results.len() as u32;
        count_indexed += batch_size;
        count_unindexed -= batch_size;

        let mut txn = env.begin_rw_txn().map_err(|e| {
            WaCustomError::DatabaseError(format!("Failed to begin transaction: {}", e))
        })?;

        txn.put(
            *metadata_db,
            &"count_indexed",
            &count_indexed.to_le_bytes(),
            WriteFlags::empty(),
        )
        .map_err(|e| {
            WaCustomError::DatabaseError(format!("Failed to update `count_indexed`: {}", e))
        })?;

        txn.put(
            *metadata_db,
            &"count_unindexed",
            &count_unindexed.to_le_bytes(),
            WriteFlags::empty(),
        )
        .map_err(|e| {
            WaCustomError::DatabaseError(format!("Failed to update `count_unindexed`: {}", e))
        })?;

        txn.commit().map_err(|e| {
            WaCustomError::DatabaseError(format!("Failed to commit transaction: {}", e))
        })?;

        Ok(())
    };

    let file = OpenOptions::new()
        .read(true)
        .open(format!("{}.vec_raw", *version))
        .map_err(|e| WaCustomError::FsError(e.to_string()))?;
    let bufman = Arc::new(BufferManager::new(file).map_err(BufIoError::Io)?);

    let mut i = 0;
    let cursor = bufman.open_cursor()?;
    let file_len = bufman.seek_with_cursor(cursor, SeekFrom::End(0))? as u32;
    bufman.seek_with_cursor(cursor, SeekFrom::Start(0))?;

    let mut embeddings = Vec::new();

    loop {
        if i == file_len {
            index(embeddings)?;
            bufman.close_cursor(cursor)?;
            break;
        }

        let (embedding, next) = read_embedding(bufman.clone(), i)?;
        embeddings.push(embedding);
        i = next;

        if embeddings.len() == upload_process_batch_size {
            index(embeddings)?;
            embeddings = Vec::new();
        }
    }

    Ok(())
}

pub fn index_embedding(
    vec_store: Arc<VectorStore>,
    cache: Arc<NodeRegistry>,
    parent: Option<LazyItem<MergedNode>>,
    vector_emb: QuantizedVectorEmbedding,
    cur_entry: LazyItem<MergedNode>,
    cur_level: HNSWLevel,
    max_insert_level: HNSWLevel,
    version: Hash,
    version_number: u32,
) -> Result<(), WaCustomError> {
    let fvec = vector_emb.quantized_vec.clone();
    let mut skipm = HashSet::new();
    skipm.insert(vector_emb.hash_vec.clone());

    let mut cur_node_arc = cur_entry.get_latest_version().get_data(cache.clone());
    let cur_node = cur_node_arc.get();

    let mut prop_arc = cur_node.prop.clone();
    let prop_state = prop_arc.get();

    let node_prop = match &*prop_state {
        PropState::Ready(prop) => prop,
        PropState::Pending(_) => {
            return Err(WaCustomError::NodeError(
                "Node prop is in pending state".to_string(),
            ))
        }
    };

    let z = traverse_find_nearest(
        vec_store.clone(),
        cache.clone(),
        cur_entry.clone(),
        fvec.clone(),
        vector_emb.hash_vec.clone(),
        0,
        &mut skipm,
        cur_level,
        true,
    )?;

    let dist = vec_store
        .distance_metric
        .calculate(&fvec, &node_prop.value)?;

    let z = if z.is_empty() {
        vec![(cur_entry.clone(), dist)]
    } else {
        z
    };

    let z_clone: Vec<_> = z.iter().map(|(first, _)| first.clone()).collect();

    let parent = if cur_level.0 <= max_insert_level.0 {
        let parent = insert_node_create_edges(
            vec_store.clone(),
            cache.clone(),
            parent,
            fvec,
            vector_emb.hash_vec.clone(),
            z,
            cur_level,
            version,
            version_number,
        )
        .expect("Failed insert_node_create_edges");
        Some(parent)
    } else {
        None
    };

    if cur_level.0 != 0 {
        index_embedding(
            vec_store.clone(),
            cache,
            parent,
            vector_emb.clone(),
            z_clone[0].clone(),
            HNSWLevel(cur_level.0 - 1),
            max_insert_level,
            version,
            version_number,
        )?;
    }

    Ok(())
}

pub fn queue_node_prop_exec(
    lznode: LazyItem<MergedNode>,
    prop_file: Arc<File>,
    vec_store: Arc<VectorStore>,
) -> Result<(), WaCustomError> {
    let (mut node_arc, _location) = match &lznode {
        LazyItem::Valid {
            data: Some(node),
            file_index,
            ..
        } => (node.clone(), file_index.clone().get().clone()),
        LazyItem::Valid {
            data: None,
            file_index,
            ..
        } => {
            if let Some(file_index) = file_index.clone().get().clone() {
                match file_index {
                    FileIndex::Valid { offset, .. } => {
                        return Err(WaCustomError::LazyLoadingError(format!(
                            "Node at offset {} needs to be loaded",
                            offset.0
                        )));
                    }
                    FileIndex::Invalid => {
                        return Err(WaCustomError::NodeError("Node is null".to_string()));
                    }
                }
            } else {
                return Err(WaCustomError::NodeError("Node is null".to_string()));
            }
        }
        _ => return Err(WaCustomError::NodeError("Node is null".to_string())),
    };

    let node = node_arc.get();
    let mut prop_arc = node.prop.clone();

    let prop_state = prop_arc.get();

    if let PropState::Ready(node_prop) = &*prop_state {
        let prop_location = write_prop_to_file(node_prop, &prop_file);
        node.set_prop_location(prop_location);
    } else {
        return Err(WaCustomError::NodeError(
            "Node prop is not ready".to_string(),
        ));
    }

    for neighbor in node.neighbors.iter() {
        neighbor.1.set_persistence(true);
    }

    // Add the node to exec_queue_nodes
    let mut exec_queue = vec_store.exec_queue_nodes.clone();
    exec_queue
        .transactional_update(|queue| {
            let mut new_queue = queue.clone();
            new_queue.push(ArcShift::new(lznode.clone()));
            new_queue
        })
        .unwrap();

    Ok(())
}

pub fn _link_prev_version(_prev_loc: Option<u32>, _offset: u32) {
    // todo , needs to happen in file persist
}

pub fn auto_commit_transaction(
    vec_store: Arc<VectorStore>,
    bufmans: Arc<BufferManagerFactory>,
) -> Result<(), WaCustomError> {
    // Retrieve exec_queue_nodes from vec_store
    let mut exec_queue_nodes_arc = vec_store.exec_queue_nodes.clone();
    let mut exec_queue_nodes = exec_queue_nodes_arc.get().clone();

    for node in exec_queue_nodes.iter_mut() {
        println!("auto_commit_txn");
        persist_node_update_loc(bufmans.clone(), node)?;
    }

    exec_queue_nodes_arc.update(Vec::new());

    Ok(())
}

fn create_node_extract_neighbours(
    version: Hash,
    hnsw_level: HNSWLevel,
    prop: ArcShift<PropState>,
    parent: LazyItemRef<MergedNode>,
) -> (
    LazyItem<MergedNode>,
    EagerLazyItemSet<MergedNode, MetricResult>,
) {
    let neighbours = EagerLazyItemSet::new();
    let node = LazyItem::from_data(
        version,
        MergedNode {
            hnsw_level,
            prop,
            neighbors: neighbours.clone(),
            parent,
            child: LazyItemRef::new_invalid(),
        },
    );
    (node, neighbours)
}

fn insert_node_create_edges(
    vec_store: Arc<VectorStore>,
    cache: Arc<NodeRegistry>,
    parent: Option<LazyItem<MergedNode>>,
    fvec: Arc<Storage>,
    hs: VectorId,
    nbs: Vec<(LazyItem<MergedNode>, MetricResult)>,
    cur_level: HNSWLevel,
    version: Hash,
    version_number: u32,
) -> Result<LazyItem<MergedNode>, WaCustomError> {
    let prop = PropState::Ready(Arc::new(NodeProp {
        id: hs.clone(),
        value: fvec.clone(),
        location: None,
    }));
    let (node, neighbours) = create_node_extract_neighbours(
        version,
        cur_level,
        ArcShift::new(prop),
        parent.clone().map_or_else(
            || LazyItemRef::new_invalid(),
            |parent| LazyItemRef::from_lazy(parent),
        ),
    );
    if let Some(parent) = parent {
        parent.get_lazy_data().unwrap().set_child(node.clone());
    }

    for (nbr1, dist) in nbs.into_iter() {
        if let LazyItem::Valid {
            data: Some(mut old_neighbour),
            ..
        } = nbr1.clone()
        {
            let (new_neighbor, mut new_neighbor_neighbors, mut neighbor_list) =
                if let Some(version) = nbr1
                    .get_version(vec_store.vcs.clone(), version_number)
                    .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?
                {
                    let mut node = version.get_data(cache.clone());
                    let neighbor_list: Vec<(LazyItem<MergedNode>, MetricResult)> = node
                        .get()
                        .neighbors
                        .iter()
                        .map(|nbr2| (nbr2.1, nbr2.0))
                        .collect();
                    (version, node.get().neighbors.clone(), neighbor_list)
                } else {
                    let prop_arc = old_neighbour.get().prop.clone();
                    let parent = old_neighbour.get().parent.clone();
                    let (new_neighbour, new_neighbour_neighbours) =
                        create_node_extract_neighbours(version, cur_level, prop_arc, parent);
                    nbr1.add_version(vec_store.vcs.clone(), version_number, new_neighbour.clone())
                        .map_err(|e| WaCustomError::DatabaseError(e.to_string()))?;
                    let neighbor_list: Vec<(LazyItem<MergedNode>, MetricResult)> = old_neighbour
                        .get()
                        .neighbors
                        .iter()
                        .map(|nbr2| (nbr2.1, nbr2.0))
                        .collect();
                    let mut exec_queue = vec_store.exec_queue_nodes.clone();
                    exec_queue
                        .transactional_update(|queue| {
                            let mut new_queue = queue.clone();
                            new_queue.push(ArcShift::new(new_neighbour.clone()));
                            new_queue
                        })
                        .unwrap();

                    (new_neighbour, new_neighbour_neighbours, neighbor_list)
                };

            neighbor_list.push((node.clone(), dist));

            neighbor_list.sort_by(|a, b| {
                b.1.get_value()
                    .partial_cmp(&a.1.get_value())
                    .unwrap_or(Ordering::Equal)
            });

            neighbor_list.truncate(20);
            let new_neighbour_neighbours_set = IdentitySet::from_iter(
                neighbor_list
                    .into_iter()
                    .map(|(node, dist)| EagerLazyItem(dist, node)),
            );
            new_neighbor_neighbors
                .items
                .update(new_neighbour_neighbours_set);
            neighbours.insert(EagerLazyItem(dist, new_neighbor));
        }
    }

    queue_node_prop_exec(node.clone(), vec_store.prop_file.clone(), vec_store)?;

    Ok(node)
}

fn traverse_find_nearest(
    vec_store: Arc<VectorStore>,
    cache: Arc<NodeRegistry>,
    vtm: LazyItem<MergedNode>,
    fvec: Arc<Storage>,
    hs: VectorId,
    hops: u8,
    skipm: &mut HashSet<VectorId>,
    cur_level: HNSWLevel,
    skip_hop: bool,
) -> Result<Vec<(LazyItem<MergedNode>, MetricResult)>, WaCustomError> {
    let mut tasks: SmallVec<[Vec<(LazyItem<MergedNode>, MetricResult)>; 24]> = SmallVec::new();

    let mut node_arc = vtm.get_latest_version().get_data(cache.clone());

    let node = node_arc.get();

    for (index, nref) in node.neighbors.iter().enumerate() {
        if let Some(mut neighbor_arc) = nref.1.get_lazy_data() {
            let neighbor = neighbor_arc.get();
            let mut prop_arc = neighbor.prop.clone();
            let prop_state = prop_arc.get();

            let node_prop = match prop_state {
                PropState::Ready(prop) => prop.clone(),
                PropState::Pending(loc) => {
                    return Err(WaCustomError::NodeError(format!(
                        "Neighbor prop is in pending state at loc: {:?}",
                        loc
                    )))
                }
            };

            let nb = node_prop.id.clone();

            if index % 2 != 0 && skip_hop && index > 4 {
                continue;
            }

            let vec_store = vec_store.clone();
            let fvec = fvec.clone();
            let hs = hs.clone();

            if skipm.insert(nb.clone()) {
                let dist = vec_store
                    .distance_metric
                    .calculate(&fvec, &node_prop.value)?;

                let full_hops = 30;
                if hops <= tapered_total_hops(full_hops, cur_level.0, vec_store.max_cache_level) {
                    let mut z = traverse_find_nearest(
                        vec_store.clone(),
                        cache.clone(),
                        nref.1.clone(),
                        fvec.clone(),
                        hs.clone(),
                        hops + 1,
                        skipm,
                        cur_level,
                        skip_hop,
                    )?;
                    z.push((nref.1.clone(), dist));
                    tasks.push(z);
                } else {
                    tasks.push(vec![(nref.1.clone(), dist)]);
                }
            }
        }
    }

    let mut nn: Vec<_> = tasks.into_iter().flatten().collect();
    nn.sort_by(|a, b| b.1.get_value().partial_cmp(&a.1.get_value()).unwrap());
    let mut seen = HashSet::new();
    nn.retain(|(lazy_node, _)| {
        if let LazyItem::Valid {
            data: Some(node_arc),
            ..
        } = &lazy_node
        {
            let mut node_arc = node_arc.clone();
            let node = node_arc.get();
            let mut prop_arc = node.prop.clone();
            let prop_state = prop_arc.get();
            if let PropState::Ready(node_prop) = &*prop_state {
                seen.insert(node_prop.id.clone())
            } else {
                false
            }
        } else {
            false
        }
    });

    Ok(nn.into_iter().take(5).collect())
}

#[cfg(test)]
mod tests {
    use super::{read_embedding, write_embedding, RawVectorEmbedding};
    use crate::models::{buffered_io::BufferManager, types::VectorId};
    use rand::{distributions::Uniform, rngs::ThreadRng, thread_rng, Rng};
    use std::sync::Arc;
    use tempfile::tempfile;

    fn get_random_embedding(rng: &mut ThreadRng) -> RawVectorEmbedding {
        let range = Uniform::new(-1.0, 1.0);

        let raw_vec: Vec<f32> = (0..rng.gen_range(100..200))
            .into_iter()
            .map(|_| rng.sample(&range))
            .collect();

        RawVectorEmbedding {
            raw_vec,
            hash_vec: VectorId::Int(rng.gen()),
        }
    }

    #[test]
    fn test_embedding_serialization() {
        let mut rng = thread_rng();
        let embedding = get_random_embedding(&mut rng);
        let tempfile = tempfile().unwrap();

        let bufman = Arc::new(BufferManager::new(tempfile).unwrap());
        let offset = write_embedding(bufman.clone(), &embedding).unwrap();

        let (deserialized, _) = read_embedding(bufman.clone(), offset).unwrap();

        assert_eq!(embedding, deserialized);
    }

    #[test]
    fn test_embeddings_serialization() {
        let mut rng = thread_rng();
        let embeddings: Vec<_> = (0..20).map(|_| get_random_embedding(&mut rng)).collect();
        let tempfile = tempfile().unwrap();

        let bufman = Arc::new(BufferManager::new(tempfile).unwrap());

        for embedding in &embeddings {
            write_embedding(bufman.clone(), embedding).unwrap();
        }

        let mut offset = 0;

        for embedding in embeddings {
            let (deserialized, next) = read_embedding(bufman.clone(), offset).unwrap();
            offset = next;

            assert_eq!(embedding, deserialized);
        }
    }
}
