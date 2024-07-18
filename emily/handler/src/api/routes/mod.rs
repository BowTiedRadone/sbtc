//! Route definitiions for the Emily API.

use crate::context::EmilyContext;

use super::handlers;
use warp::Filter;

/// Chainstate routes.
mod chainstate;
/// Deposit routes.
mod deposit;
/// Health routes.
mod health;
/// Withdrawal routes.
mod withdrawal;

/// This function sets up the Warp filters for handling all requests.
pub fn routes(
    context: EmilyContext,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    // TODO(273):  Remove the "local" prefix once we figure out why all local
    // testing calls seem to forcibly start with `local`.
    warp::path("local").and(
        health::routes()
            .or(chainstate::routes(context.clone()))
            .or(deposit::routes(context.clone()))
            .or(withdrawal::routes(context)),
    )
}
