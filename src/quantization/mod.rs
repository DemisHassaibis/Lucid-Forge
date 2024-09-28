pub mod product;
pub mod scalar;

use crate::storage::Storage;

pub trait Quantization: std::fmt::Debug + Send + Sync {
    fn quantize(
        &self,
        vector: &[f32],
        storage_type: StorageType,
    ) -> Result<Storage, QuantizationError>;
    fn train(&mut self, vectors: &[&[f32]]) -> Result<(), QuantizationError>;
}

#[derive(Debug, Clone, Copy)]
pub enum StorageType {
    UnsignedByte,
    SubByte(u8),
    HalfPrecisionFP,
}

#[derive(Debug)]
pub enum QuantizationError {
    InvalidInput(String),
    TrainingFailed,
}
