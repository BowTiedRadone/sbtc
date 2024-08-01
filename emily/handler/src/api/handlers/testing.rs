//! Handlers for Health endpoint endpoints.

use reqwest::StatusCode;
use serde_json::json;
use warp::reply::Reply;

use crate::common::error::Error;
use crate::context::EmilyContext;
use crate::database::accessors;


/// Get health handler.
#[utoipa::path(
    post,
    operation_id = "wipeDatabases",
    path = "/testing/wipe",
    tag = "testing",
    responses(
        // TODO(271): Add success body.
        (status = 204, description = "Successfully wiped databases."),
        (status = 400, description = "Invalid request body"),
        (status = 404, description = "Address not found"),
        (status = 405, description = "Method not allowed"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn wipe_databases(
    context: EmilyContext,
) -> impl warp::reply::Reply {
    // Internal handler so `?` can be used correctly while still returning a reply.
    async fn handler(
        context: EmilyContext,
    ) -> Result<impl warp::reply::Reply, Error> {
       accessors::wipe_all_tables(&context).await?;
        Ok(warp::reply::with_status(
            warp::reply::json(
                &json!({
                    "message": "wiped database"
                })
            ),
            StatusCode::NO_CONTENT,
        ))
    }

    // Handle and respond.
    handler(context)
        .await
        .map_or_else(Reply::into_response, Reply::into_response)
}
