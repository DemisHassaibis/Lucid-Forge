use crate::models::chunked_list::*;
use crate::models::common::*;
use crate::models::custom_buffered_writer::CustomBufferedWriter;
use crate::models::file_persist::*;
use crate::models::meta_persist::*;
use crate::models::types::*;
use bincode;
use dashmap::DashMap;
use futures::stream::Collect;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smallvec::SmallVec;
use std::borrow::BorrowMut;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Seek, SeekFrom, Write};
use std::ops::Deref;
use std::sync::RwLock;
use std::sync::{Arc, Mutex};

pub fn ann_search(
    vec_store: Arc<VectorStore>,
    vector_emb: VectorEmbedding,
    cur_entry: LazyItem<MergedNode>,
    cur_level: i8,
) -> Result<Option<Vec<(LazyItem<MergedNode>, f32)>>, WaCustomError> {
    if cur_level == -1 {
        return Ok(Some(vec![]));
    }

    let fvec = vector_emb.raw_vec.clone();
    let mut skipm = HashSet::new();
    skipm.insert(vector_emb.hash_vec.clone());

    let cur_node = match cur_entry.clone() {
        LazyItem::Ready(node, _) => node,
        LazyItem::LazyLoad(offset) => {
            return Err(WaCustomError::LazyLoadingError(format!(
                "Node at offset {} needs to be loaded",
                offset
            )))
        }
        LazyItem::Null => {
            return Err(WaCustomError::NodeError(
                "Current entry is null".to_string(),
            ))
        }
    };

    let prop_state = cur_node
        .prop
        .read()
        .map_err(|e| WaCustomError::LockError(format!("Failed to read prop: {}", e)))?;

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
        cur_entry.clone(),
        fvec.clone(),
        vector_emb.hash_vec.clone(),
        0,
        &mut skipm,
        cur_level,
        false,
    )?;

    let cs = cosine_similarity_qt(&fvec, &node_prop.value, vec_store.quant_dim)?;

    let z = if z.is_empty() {
        vec![(cur_entry.clone(), cs)]
    } else {
        z
    };

    let result = ann_search(
        vec_store.clone(),
        vector_emb.clone(),
        z[0].0.clone(),
        cur_level - 1,
    )?;

    Ok(add_option_vecs(&result, &Some(z)))
}

pub fn vector_fetch(
    vec_store: Arc<VectorStore>,
    vector_id: VectorId,
) -> Result<Vec<Option<(VectorId, Vec<(VectorId, f32)>)>>, WaCustomError> {
    let mut results = Vec::new();

    for lev in 0..vec_store.max_cache_level {
        let maybe_res = load_vector_id_lsmdb(lev, vector_id.clone());
        let neighbors = match maybe_res {
            LazyItem::Ready(vth, _loc) => {
                let neighbors_guard = vth.neighbors.read().map_err(|e| {
                    WaCustomError::LockError(format!("Failed to read neighbors: {}", e))
                })?;

                let nes: Vec<(VectorId, f32)> = neighbors_guard
                    .iter()
                    .filter_map(|ne| match ne {
                        LazyItem::Ready(nbr, _) => get_neighbor_info(nbr),
                        LazyItem::LazyLoad(xloc) => {
                            match load_neighbor_from_db(*xloc, &vec_store) {
                                Ok(Some(info)) => Some(info),
                                Ok(None) => None,
                                Err(e) => {
                                    eprintln!("Error loading neighbor: {}", e);
                                    None
                                }
                            }
                        }
                        LazyItem::Null => None,
                    })
                    .collect();
                Some((vector_id.clone(), nes))
            }
            LazyItem::LazyLoad(xloc) => match load_node_from_persist(xloc, &vec_store) {
                Ok(Some((id, neighbors))) => Some((id, neighbors)),
                Ok(None) => None,
                Err(e) => {
                    eprintln!("Error loading vector: {}", e);
                    None
                }
            },
            LazyItem::Null => None,
        };
        results.push(neighbors);
    }
    Ok(results)
}
fn load_node_from_persist(
    offset: FileOffset,
    vec_store: &Arc<VectorStore>,
) -> Result<Option<(VectorId, Vec<(VectorId, f32)>)>, WaCustomError> {
    // Placeholder function to load vector from database
    // TODO: Implement actual database loading logic
    Err(WaCustomError::LazyLoadingError(
        "Not implemented".to_string(),
    ))
}
fn get_neighbor_info(nbr: &Neighbour) -> Option<(VectorId, f32)> {
    let prop_state = match nbr.node.prop.read() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Lock error when reading prop: {}", e);
            return None;
        }
    };

    match &*prop_state {
        PropState::Ready(node_prop) => Some((node_prop.id.clone(), nbr.cosine_similarity)),
        PropState::Pending(_) => {
            eprintln!("Encountered pending prop state");
            None
        }
    }
}

fn load_neighbor_from_db(
    offset: FileOffset,
    vec_store: &Arc<VectorStore>,
) -> Result<Option<(VectorId, f32)>, WaCustomError> {
    // Placeholder function to load neighbor from database
    // TODO: Implement actual database loading logic
    Err(WaCustomError::LazyLoadingError(
        "Not implemented".to_string(),
    ))
}
pub fn insert_embedding(
    vec_store: Arc<VectorStore>,
    vector_emb: VectorEmbedding,
    cur_entry: LazyItem<MergedNode>,
    cur_level: i8,
    max_insert_level: i8,
) -> Result<(), WaCustomError> {
    if cur_level == -1 {
        return Ok(());
    }

    let fvec = vector_emb.raw_vec.clone();
    let mut skipm = HashSet::new();
    skipm.insert(vector_emb.hash_vec.clone());

    let cur_node = match &cur_entry {
        LazyItem::Ready(node, _) => node,
        LazyItem::LazyLoad(offset) => {
            return Err(WaCustomError::LazyLoadingError(format!(
                "Node at offset {} needs to be loaded",
                offset
            )))
        }
        LazyItem::Null => {
            return Err(WaCustomError::NodeError(
                "Current entry is null".to_string(),
            ))
        }
    };

    let prop_state = cur_node
        .prop
        .read()
        .map_err(|e| WaCustomError::LockError(format!("Failed to read prop: {}", e)))?;

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
        cur_entry.clone(),
        fvec.clone(),
        vector_emb.hash_vec.clone(),
        0,
        &mut skipm,
        cur_level,
        true,
    )?;

    let cs = cosine_similarity_qt(&fvec, &node_prop.value, vec_store.quant_dim)?;

    let z = if z.is_empty() {
        vec![(cur_entry.clone(), cs)]
    } else {
        z
    };

    let z_clone: Vec<_> = z.iter().map(|(first, _)| first.clone()).collect();

    if cur_level <= max_insert_level {
        insert_embedding(
            vec_store.clone(),
            vector_emb.clone(),
            z_clone[0].clone(),
            cur_level - 1,
            max_insert_level,
        )?;
        insert_node_create_edges(
            vec_store.clone(),
            fvec,
            vector_emb.hash_vec.clone(),
            z,
            cur_level,
        );
    } else {
        insert_embedding(
            vec_store.clone(),
            vector_emb.clone(),
            z_clone[0].clone(),
            cur_level - 1,
            max_insert_level,
        )?;
    }

    Ok(())
}

pub fn queue_node_prop_exec(
    lznode: LazyItem<MergedNode>,
    prop_file: Arc<File>,
) -> Result<(), WaCustomError> {
    let (node, location) = match &lznode {
        LazyItem::Ready(node, loc) => (node.clone(), *loc),
        LazyItem::LazyLoad(offset) => {
            return Err(WaCustomError::LazyLoadingError(format!(
                "Node at offset {} needs to be loaded",
                offset
            )))
        }
        LazyItem::Null => return Err(WaCustomError::NodeError("Node is null".to_string())),
    };

    // Write main node prop to file
    let prop_state = node
        .prop
        .read()
        .map_err(|e| WaCustomError::LockError(format!("Failed to read node prop: {}", e)))?;

    if let PropState::Ready(node_prop) = &*prop_state {
        let prop_location = write_prop_to_file(node_prop, &prop_file);
        node.set_prop_location(prop_location);
    } else {
        return Err(WaCustomError::NodeError(
            "Node prop is not ready".to_string(),
        ));
    }

    // Set persistence flag for the main node
    node.set_persistence(true);

    // Set persistence flag for all neighbors (without writing to prop file)
    let neighbors = node
        .neighbors
        .read()
        .map_err(|e| WaCustomError::LockError(format!("Failed to read neighbors: {}", e)))?;

    for nbr in neighbors.iter() {
        if let LazyItem::Ready(neighbor, _) = nbr {
            neighbor.set_persistence(true);
        }
    }

    Ok(())
}

pub fn link_prev_version(prev_loc: Option<u32>, offset: u32) {
    // todo , needs to happen in file persist
}
pub fn auto_commit_transaction(
    vec_store: Arc<VectorStore>,
    buf_writer: &mut CustomBufferedWriter,
) -> Result<(), WaCustomError> {
    // Retrieve exec_queue_nodes from vec_store
    let exec_queue_nodes = vec_store.exec_queue_nodes.read().map_err(|_| {
        WaCustomError::LockError("Failed to acquire read lock on exec_queue_nodes".to_string())
    })?;

    // Iterate through the exec_queue_nodes and persist each node
    for node in exec_queue_nodes.iter() {
        let mut node = node.clone(); // Clone to get a mutable version
        persist_node_update_loc(buf_writer, &mut node)?;
    }

    // Update version
    let ver = vec_store
        .get_current_version()
        .unwrap()
        .expect("No current version found");
    let new_ver = ver.version + 1;
    let vec_hash =
        store_current_version(vec_store.clone(), "main".to_string(), new_ver).map_err(|e| {
            WaCustomError::DatabaseError(format!("Failed to store current version: {:?}", e))
        })?;

    vec_store
        .set_current_version(Some(vec_hash))
        .map_err(|e| WaCustomError::LockError(format!("Failed to set current version: {:?}", e)))?;

    Ok(())
}

fn insert_node_create_edges(
    vec_store: Arc<VectorStore>,
    fvec: Arc<VectorQt>,
    hs: VectorId,
    nbs: Vec<(LazyItem<MergedNode>, f32)>,
    cur_level: i8,
) -> Result<(), WaCustomError> {
    let nd_p = NodeProp {
        id: hs.clone(),
        value: fvec.clone(),
        location: None,
    };
    let nn = Arc::new(MergedNode::new(0, cur_level as u8)); // Assuming MergedNode::new exists
    nn.set_prop_ready(Arc::new(nd_p));

    // Convert LazyItem<MergedNode> to Arc<MergedNode>
    let ready_neighbors: Vec<(Arc<MergedNode>, f32)> = nbs
        .iter()
        .filter_map(|(nbr, cs)| {
            if let LazyItem::Ready(node, _) = nbr {
                Some((node.clone(), *cs))
            } else {
                None
            }
        })
        .collect();

    nn.add_ready_neighbors(ready_neighbors);

    for (nbr1, cs) in nbs.into_iter() {
        if let LazyItem::Ready(nbr1_node, _) = nbr1 {
            let mut neighbor_list: Vec<(Arc<MergedNode>, f32)> = nbr1_node
                .neighbors
                .read()
                .map_err(|_| WaCustomError::LockError("Failed to read neighbors".to_string()))?
                .iter()
                .filter_map(|nbr2| {
                    if let LazyItem::Ready(neighbor, _) = nbr2 {
                        Some((neighbor.node.clone(), neighbor.cosine_similarity))
                    } else {
                        None
                    }
                })
                .collect();

            neighbor_list.push((nn.clone(), cs));
            neighbor_list
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            let mut seen = HashSet::new();
            neighbor_list.retain(|(node, _)| seen.insert(Arc::as_ptr(node) as *const _));
            neighbor_list.truncate(20);

            nbr1_node.add_ready_neighbors(neighbor_list);
        }
    }

    queue_node_prop_exec(
        LazyItem::Ready(nn.clone(), None),
        vec_store.prop_file.clone(),
    )?;

    Ok(())
}

fn traverse_find_nearest(
    vec_store: Arc<VectorStore>,
    vtm: LazyItem<MergedNode>,
    fvec: Arc<VectorQt>,
    hs: VectorId,
    hops: u8,
    skipm: &mut HashSet<VectorId>,
    cur_level: i8,
    skip_hop: bool,
) -> Result<Vec<(LazyItem<MergedNode>, f32)>, WaCustomError> {
    let mut tasks: SmallVec<[Vec<(LazyItem<MergedNode>, f32)>; 24]> = SmallVec::new();

    let node = match vtm {
        LazyItem::Ready(node, _) => node,
        LazyItem::LazyLoad(_) => {
            return Err(WaCustomError::LazyLoadingError(
                "Node needs to be loaded".to_string(),
            ))
        }
        LazyItem::Null => return Err(WaCustomError::NodeError("Node is null".to_string())),
    };

    let neighbors_lock = node.neighbors.read().map_err(|_| {
        WaCustomError::LockError("Failed to acquire read lock on neighbors".to_string())
    })?;

    for (index, nref) in neighbors_lock.iter().enumerate() {
        if let LazyItem::Ready(neighbor, _) = nref {
            let prop_state = neighbor.node.prop.read().map_err(|_| {
                WaCustomError::LockError("Failed to read neighbor prop".to_string())
            })?;

            let node_prop = match &*prop_state {
                PropState::Ready(prop) => prop,
                PropState::Pending(_) => {
                    return Err(WaCustomError::NodeError(
                        "Neighbor prop is in pending state".to_string(),
                    ))
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
                let cs = cosine_similarity_qt(&fvec, &node_prop.value, vec_store.quant_dim)?;

                let full_hops = 30;
                if hops <= tapered_total_hops(full_hops, cur_level as u8, vec_store.max_cache_level)
                {
                    let mut z = traverse_find_nearest(
                        vec_store.clone(),
                        LazyItem::Ready(neighbor.node.clone(), None),
                        fvec.clone(),
                        hs.clone(),
                        hops + 1,
                        skipm,
                        cur_level,
                        skip_hop,
                    )?;
                    z.push((LazyItem::Ready(neighbor.node.clone(), None), cs));
                    tasks.push(z);
                } else {
                    tasks.push(vec![(LazyItem::Ready(neighbor.node.clone(), None), cs)]);
                }
            }
        }
    }

    let mut nn: Vec<_> = tasks.into_iter().flatten().collect();
    nn.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let mut seen = HashSet::new();
    nn.retain(|(lazy_node, _)| {
        if let LazyItem::Ready(node, _) = lazy_node {
            let prop_state = node.prop.read().unwrap();
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
