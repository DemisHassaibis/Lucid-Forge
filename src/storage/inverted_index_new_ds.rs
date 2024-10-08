use rayon::prelude::*;
use std::array::from_fn;
use std::path::Path;
use std::sync::Arc;

use crate::models::buffered_io::BufferManagerFactory;
use crate::models::cache_loader::NodeRegistry;
use crate::models::lazy_load::IncrementalSerializableGrowableData;
use crate::models::lazy_load::LazyItem;
use crate::models::lazy_load::LazyItemArray;
use crate::models::lazy_load::LazyItemVec;
use crate::models::types::SparseVector;
use arcshift::ArcShift;

// TODO: Add more powers for larger jumps
// TODO: Or switch to dynamic calculation of power of max power of 4
const POWERS_OF_4: [u32; 8] = [1, 4, 16, 64, 256, 1024, 4096, 16384];

/// Returns the largest power of 4 that is less than or equal to `n`.
/// Iteratively multiplies by 4 until the result exceeds `n`.
pub fn largest_power_of_4_below(n: u32) -> (usize, u32) {
    assert_ne!(n, 0, "Cannot find largest power of 4 below 0");
    POWERS_OF_4
        .into_iter()
        .enumerate()
        .rev()
        .find(|&(_, pow4)| pow4 <= n)
        .unwrap()
}

/// Calculates the path from `current_dim_index` to `target_dim_index`.
/// Decomposes the difference into powers of 4 and returns the indices.
fn calculate_path(target_dim_index: u32, current_dim_index: u32) -> Vec<usize> {
    let mut path = Vec::new();
    let mut remaining = target_dim_index - current_dim_index;

    while remaining > 0 {
        let (child_index, pow_4) = largest_power_of_4_below(remaining);
        path.push(child_index);
        remaining -= pow_4;
    }

    path
}

#[derive(Clone)]
pub struct InvertedIndexNewDSNode {
    pub dim_index: u32,
    pub implicit: bool,
    pub data: Arc<[IncrementalSerializableGrowableData; 63]>,
    pub lazy_children: LazyItemArray<InvertedIndexNewDSNode, 16>,
}

impl InvertedIndexNewDSNode {
    pub fn new(dim_index: u32, implicit: bool) -> Self {
        let data = Arc::new(from_fn(|_| IncrementalSerializableGrowableData::new()));
        InvertedIndexNewDSNode {
            dim_index,
            implicit,
            data,
            lazy_children: LazyItemArray::new(),
        }
    }

    /// Finds or creates the node where the data should be inserted.
    /// Traverses the tree iteratively and returns a reference to the node.
    fn find_or_create_node(
        node: ArcShift<InvertedIndexNewDSNode>,
        path: &[usize],
        cache: Arc<NodeRegistry>,
    ) -> ArcShift<InvertedIndexNewDSNode> {
        let mut current_node = node;
        for &child_index in path {
            let new_dim_index = current_node.dim_index + POWERS_OF_4[child_index];
            let new_child =
                LazyItem::new(0.into(), InvertedIndexNewDSNode::new(new_dim_index, true));
            loop {
                if let Some(child) = current_node
                    .lazy_children
                    .checked_insert(child_index, new_child.clone())
                {
                    current_node = child.get_data(cache.clone());
                    break;
                }
            }
        }

        current_node
    }

    pub fn quantize(value: f32) -> u8 {
        ((value * 63.0).clamp(0.0, 63.0) as u8).min(63)
    }

    pub fn insert(node: ArcShift<InvertedIndexNewDSNode>, value: f32, vector_id: u32) {
        let quantized_value = Self::quantize(value);
        let mut node = node.shared_get().clone();

        if let Some(growable_data) = Arc::make_mut(&mut node.data).get_mut(quantized_value as usize)
        {
            growable_data.insert(vector_id);
        };
    }

    /// Retrieves a value from the index at the specified dimension index.
    /// Calculates the path and delegates to `get_value`.
    pub fn get(&self, dim_index: u32, vector_id: u32, cache: Arc<NodeRegistry>) -> Option<u8> {
        let path = calculate_path(dim_index, self.dim_index);
        self.get_value(&path, vector_id, cache)
    }

    /// Retrieves a value from the index following the specified path.
    /// Recursively traverses child nodes or searches the data vector.
    fn get_value(&self, path: &[usize], vector_id: u32, cache: Arc<NodeRegistry>) -> Option<u8> {
        match path.get(0) {
            Some(child_index) => self
                .lazy_children
                .get(*child_index)
                .map(|data| {
                    data.get_data(cache.clone())
                        .get_value(&path[1..], vector_id, cache)
                })
                .flatten(),
            None => {
                for (index, growable_data) in self.data.iter().enumerate() {
                    if growable_data.items.iter().any(|item| {
                        let mut p = item.get_data(cache.clone()).shared_get().clone();
                        p.get().data.contains(&vector_id)
                    }) {
                        return Some(index as u8);
                    }
                }
                None
            }
        }
    }
}

#[derive(Clone)]
pub struct InvertedIndexSparseAnnNewDS {
    pub root: ArcShift<InvertedIndexNewDSNode>,
    pub cache: Arc<NodeRegistry>,
}

impl InvertedIndexSparseAnnNewDS {
    pub fn new() -> Self {
        let bufmans = Arc::new(BufferManagerFactory::new(
            Path::new(".").into(),
            |root, ver| root.join(format!("{}.index", **ver)),
        ));
        let cache = Arc::new(NodeRegistry::new(1000, bufmans));
        InvertedIndexSparseAnnNewDS {
            root: ArcShift::new(InvertedIndexNewDSNode::new(0, false)),
            cache,
        }
    }

    /// Finds the node at a given dimension
    /// Traverses the tree iteratively and returns a reference to the node.
    pub fn find_node(&self, dim_index: u32) -> Option<ArcShift<InvertedIndexNewDSNode>> {
        let mut current_node = self.root.clone();
        let path = calculate_path(dim_index, self.root.dim_index);
        for child_index in path {
            let child = current_node.lazy_children.get(child_index)?;
            current_node = child.get_data(self.cache.clone());
        }

        Some(current_node)
    }

    //Fetches quantized u8 value for a dim_index and vector_Id present at respective node in index
    pub fn get(&self, dim_index: u32, vector_id: u32) -> Option<u8> {
        self.root
            .shared_get()
            .get(dim_index, vector_id, self.cache.clone())
    }

    //Inserts vec_id, quantized value u8 at particular node based on path
    pub fn insert(&self, dim_index: u32, value: f32, vector_id: u32) {
        let path = calculate_path(dim_index, self.root.dim_index);
        let node = InvertedIndexNewDSNode::find_or_create_node(
            self.root.clone(),
            &path,
            self.cache.clone(),
        );
        //value will be quantized while being inserted into the Node.
        InvertedIndexNewDSNode::insert(node, value, vector_id)
    }

    /// Adds a sparse vector to the index.
    pub fn add_sparse_vector(&self, vector: SparseVector) -> Result<(), String> {
        let vector_id = vector.vector_id;
        vector.entries.par_iter().for_each(|(dim_index, value)| {
            if *value != 0.0 {
                self.insert(*dim_index, *value, vector_id);
            }
        });
        Ok(())
    }
}
