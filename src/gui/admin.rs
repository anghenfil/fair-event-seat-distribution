use rocket::http::Status;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;

use crate::backend::auth::{Session, SessionUserType};
use crate::backend::data::Event;
use crate::backend::state::AppState;

#[derive(Serialize)]
struct AdminIndexContext {
    events: Vec<Event>,
}

#[get("/admin")]
pub fn admin_index(session: Session, state: &State<AppState>) -> Result<Template, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let storage = state.storage.read().expect("storage poisoned");
            let events : Vec<Event> = storage.events.values().cloned().collect();
            let ctx = AdminIndexContext { events };
            Ok(Template::render("admin/index", &ctx))
        }
        _ => Err(Status::Forbidden),
    }
}
