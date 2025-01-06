use serde::{Deserialize, Serialize};

use super::{Quantization, QuantizationError, StorageType};
use crate::storage::Storage;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProductQuantization {

    pub centroids: Option<Centroid>,

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Centroid {

    pub number_of_centroids: u16,
    pub centroids: Vec<u16>,

}

#[allow(unused_variables)]
impl Quantization for ProductQuantization {

    fn quantize(
        &self,
        vector: &[f32],
        storage_type: StorageType,
        range: (f32, f32),

    ) -> Result<Storage, QuantizationError> {

        unimplemented!("Product quantization is not implemented yet");

    }

    fn train(&mut self, vectors: &[&[f32]]) -> Result<(), QuantizationError> {

        unimplemented!("K-means clustering for product quantization is not implemented yet");
        
    }
}
