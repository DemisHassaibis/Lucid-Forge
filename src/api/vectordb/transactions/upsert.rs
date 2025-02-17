use crate::app_context::AppContext;
use actix_web::{web, HttpResponse};

// Route: `/vectordb/{database_name}/transactions/{transaction_id}/upsert`
pub(crate) async fn upsert(
    path_data: web::Path<(String, String)>,
    ctx: web::Data<AppContext>,
) -> HttpResponse {
    let (database_name, transaction_id) = path_data.into_inner();
    let Some(vec_store) = ctx.ain_env.collections_map.get(&database_name) else {
        return HttpResponse::NotFound().body("Vector store not found");
    };

    todo!()
}
