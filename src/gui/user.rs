use rocket::serde::json::Json;
use rocket::State;

use crate::backend::auth::Session;
use crate::backend::state::AppState;

#[get("/me")]
pub fn me(session: Session, _state: &State<AppState>) -> Json<String> {
    Json(format!("you are logged in with sid={} as {:?}", session.id, session.user_type))
}
