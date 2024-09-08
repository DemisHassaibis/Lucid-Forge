use actix_web::{
    web::{self},
    HttpResponse, Result,
};

use super::{
    dtos::{CreateCollectionDto, CreateCollectionDtoResponse, FindCollectionDto},
    service,
};

pub(crate) async fn create_collection(
    web::Json(create_collection_dto): web::Json<CreateCollectionDto>,
) -> Result<HttpResponse> {
    let lower_bound = create_collection_dto.min_val;
    let upper_bound = create_collection_dto.max_val;

    let collection = service::create_collection(create_collection_dto).await?;

    Ok(HttpResponse::Ok().json(CreateCollectionDtoResponse {
        id: collection.database_name.clone(), // will use the vector store name , till it does have a unique id
        dimensions: collection.quant_dim,
        max_val: lower_bound,
        min_val: upper_bound,
        name: collection.database_name.clone(),
    }))
}

pub(crate) async fn get_collection_by_id(collection_id: web::Path<String>) -> Result<HttpResponse> {
    let collection = service::get_collection_by_id(&collection_id)?;
    Ok(HttpResponse::Ok().json(FindCollectionDto {
        id: collection.database_name.clone(),
        dimensions: collection.quant_dim,
        vector_db_name: collection.database_name.clone(),
    }))
}
