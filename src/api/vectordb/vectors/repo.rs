use std::sync::{atomic::Ordering, Arc};

use crate::{
    api::vectordb::collections,
    api_service::{run_upload, run_upload_in_transaction, run_upload_sparse_vector},
    app_context::AppContext,
    convert_vectors,
    models::{
        rpc::VectorIdValue,
        types::{DenseIndexTransaction, VectorId},
    },
    vector_store::get_embedding_by_id,
};

use super::{
    dtos::{
        CreateVectorDto, CreateVectorDtox, CreateVectorResponseDto, FindSimilarVectorsDto,
        SimilarVector, UpdateVectorDto, UpdateVectorResponseDto, UpsertDto,
    },
    error::VectorsError,
};

pub(crate) async fn create_vector(
    ctx: Arc<AppContext>,
    collection_id: &str,
    create_vector_dto: CreateVectorDtox,
) -> Result<CreateVectorResponseDto, VectorsError> {
    match create_vector_dto {
        CreateVectorDtox::Dense { id, values } => {
            create_dense_vector(ctx, collection_id, id, values).await
        }
        CreateVectorDtox::Sparse { id, values } => {
            create_sparse_vector(ctx, collection_id, id, values).await
        }
    }
}

/// Creates a sparse vector for inverted index
///
pub(crate) async fn create_sparse_vector(
    ctx: Arc<AppContext>,
    collection_id: &str,
    vector_id: VectorIdValue,
    values: Vec<(f32, u32)>,
) -> Result<CreateVectorResponseDto, VectorsError> {
    let inverted_index = collections::service::get_inverted_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToCreateVector(e.to_string()))?;

    if inverted_index.current_open_transaction.is_some() {
        return Err(VectorsError::FailedToCreateVector(
            "there is an ongoing transaction!".into(),
        ));
    }

    run_upload_sparse_vector(
        ctx,
        inverted_index,
        vec![(vector_id.clone(), values.clone())],
    )
    .map_err(VectorsError::WaCustom)?;

    // Ok(CreateVectorResponseDto {
    //     id: vector_id,
    //     values,
    // })

    Err(VectorsError::NotImplemented)
}

/// Creates a vector for dense index
///
pub(crate) async fn create_dense_vector(
    ctx: Arc<AppContext>,
    collection_id: &str,
    vector_id: VectorIdValue,
    values: Vec<f32>,
    // create_vector_dto: CreateVectorDto,
) -> Result<CreateVectorResponseDto, VectorsError> {
    let dense_index = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToCreateVector(e.to_string()))?;

    if !dense_index
        .current_open_transaction
        .load(Ordering::SeqCst)
        .is_null()
    {
        return Err(VectorsError::FailedToCreateVector(
            "there is an ongoing transaction!".into(),
        ));
    }

    // TODO: handle the error
    run_upload(ctx, dense_index, vec![(vector_id.clone(), values.clone())])
        .map_err(VectorsError::WaCustom)?;
    Ok(CreateVectorResponseDto {
        id: vector_id,
        values,
    })
}

pub(crate) async fn create_vector_in_transaction(
    ctx: Arc<AppContext>,
    collection_id: &str,
    transaction: &DenseIndexTransaction,
    create_vector_dto: CreateVectorDto,
) -> Result<CreateVectorResponseDto, VectorsError> {
    let dense_index = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToCreateVector(e.to_string()))?;

    run_upload_in_transaction(
        ctx.clone(),
        dense_index,
        transaction,
        vec![(
            create_vector_dto.id.clone(),
            create_vector_dto.values.clone(),
        )],
    )
    .map_err(VectorsError::WaCustom)?;

    Ok(CreateVectorResponseDto {
        id: create_vector_dto.id,
        values: create_vector_dto.values,
    })
}

pub(crate) async fn get_vector_by_id(
    ctx: Arc<AppContext>,
    collection_id: &str,
    vector_id: VectorId,
) -> Result<CreateVectorResponseDto, VectorsError> {
    let vec_store = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|_| VectorsError::NotFound)?;

    let embedding = get_embedding_by_id(vec_store, &vector_id)
        .map_err(|e| VectorsError::DatabaseError(e.to_string()))?;

    let id = match embedding.hash_vec {
        VectorId::Int(v) => VectorIdValue::IntValue(v),
        VectorId::Str(v) => VectorIdValue::StringValue(v),
    };

    Ok(CreateVectorResponseDto {
        id,
        values: (*embedding.raw_vec).clone(),
    })
}

pub(crate) async fn update_vector(
    ctx: Arc<AppContext>,
    collection_id: &str,
    vector_id: VectorIdValue,
    update_vector_dto: UpdateVectorDto,
) -> Result<UpdateVectorResponseDto, VectorsError> {
    let dense_index = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToUpdateVector(e.to_string()))?;

    if !dense_index
        .current_open_transaction
        .load(Ordering::SeqCst)
        .is_null()
    {
        return Err(VectorsError::FailedToUpdateVector(
            "there is an ongoing transaction!".into(),
        ));
    }

    run_upload(
        ctx,
        dense_index,
        vec![(vector_id.clone(), update_vector_dto.values.clone())],
    )
    .map_err(VectorsError::WaCustom)?;

    Ok(UpdateVectorResponseDto {
        id: vector_id,
        values: update_vector_dto.values,
    })
}

pub(crate) async fn find_similar_vectors(
    find_similar_vectors: FindSimilarVectorsDto,
) -> Result<Vec<SimilarVector>, VectorsError> {
    if find_similar_vectors.vector.len() == 0 {
        return Err(VectorsError::FailedToFindSimilarVectors(
            "Vector shouldn't be empty".to_string(),
        ));
    }
    Ok(vec![SimilarVector {
        id: VectorIdValue::IntValue(find_similar_vectors.k),
        score: find_similar_vectors.vector[0],
    }])
}

pub(crate) async fn delete_vector_by_id(
    ctx: Arc<AppContext>,
    collection_id: &str,
    vector_id: VectorIdValue,
) -> Result<(), VectorsError> {
    let collection = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToDeleteVector(e.to_string()))?;

    // TODO(a-rustacean): uncomment
    // crate::vector_store::delete_vector_by_id(collection, convert_value(vector_id.clone()))
    //     .map_err(|e| VectorsError::WaCustom(e))?;

    Ok(())
}

pub(crate) async fn upsert_in_transaction(
    ctx: Arc<AppContext>,
    collection_id: &str,
    transaction: &DenseIndexTransaction,
    upsert_dto: UpsertDto,
) -> Result<(), VectorsError> {
    let dense_index = collections::service::get_dense_index_by_id(ctx.clone(), collection_id)
        .await
        .map_err(|e| VectorsError::FailedToCreateVector(e.to_string()))?;

    run_upload_in_transaction(
        ctx.clone(),
        dense_index,
        transaction,
        convert_vectors(upsert_dto.vectors),
    )
    .map_err(VectorsError::WaCustom)?;

    Ok(())
}
