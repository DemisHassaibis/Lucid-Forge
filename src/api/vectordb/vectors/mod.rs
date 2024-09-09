use actix_web::{web, Scope};

mod controller;
mod dtos;
mod error;
mod repo;
mod service;

pub(crate) fn vectors_module() -> Scope {
    let vectors_module = web::scope("/collections/{collection_id}/vectors")
        .route("", web::post().to(controller::create_vector))
        .route(
            "/{vector_id}",
            web::get().to(controller::get_vector_by_id),
        );

    vectors_module
}
