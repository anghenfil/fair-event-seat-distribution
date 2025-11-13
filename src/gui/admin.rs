use rocket::form::{Form, FromForm};
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;

use crate::backend::auth::{Session, SessionUserType};
use crate::backend::data::{Event, EventState, Slot, Session as EventSession};
use crate::backend::state::AppState;
use uuid::Uuid;

#[derive(Serialize)]
struct AdminIndexContext {
    events: Vec<Event>,
}

#[derive(FromForm)]
pub struct CreateEventForm {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize)]
struct AdminEventContext {
    event: Event,
}

#[derive(FromForm)]
pub struct SetStateForm { pub state: String }

#[derive(FromForm)]
pub struct CreateSlotForm { pub name: String, pub description: Option<String> }

#[derive(FromForm)]
pub struct EditSlotForm { pub name: String, pub description: Option<String> }

#[derive(FromForm)]
pub struct CreateSessionForm { pub name: String, pub description: Option<String>, pub seats: usize }

#[derive(FromForm)]
pub struct EditSessionForm { pub name: String, pub description: Option<String>, pub seats: usize }

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

#[get("/admin/events/<event_id>")]
pub fn event_view(session: Session, state: &State<AppState>, event_id: Uuid) -> Result<Template, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let storage = state.storage.read().expect("storage poisoned");
            match storage.events.get(&event_id) {
                Some(ev) => {
                    let ctx = AdminEventContext { event: ev.clone() };
                    Ok(Template::render("admin/event", &ctx))
                }
                None => Err(Status::NotFound)
            }
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events", data = "<form>")]
pub fn create_event(session: Session, state: &State<AppState>, form: Form<CreateEventForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let form = form.into_inner();
            let mut storage = state.storage.write().expect("storage poisoned");
            let name = form.name.trim().to_string();
            if name.is_empty() { return Err(Status::BadRequest); }
            let event = Event::new(name, form.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            let id = event.uuid;
            storage.events.insert(id, event);
            Ok(Redirect::to("/admin"))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/delete")]
pub fn delete_event(session: Session, state: &State<AppState>, event_id: Uuid) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            storage.events.remove(&event_id);
            Ok(Redirect::to("/admin"))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/state", data = "<form>")]
pub fn set_event_state(session: Session, state: &State<AppState>, event_id: Uuid, form: Form<SetStateForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let desired = form.into_inner().state;
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let target = match desired.as_str() {
                "NotOpenedYet" => EventState::NotOpenedYet,
                "OpenForRegistration" => EventState::OpenForRegistration,
                _ => return Err(Status::BadRequest),
            };
            // Allow transitions only between these two states or no-op
            let allowed_transition = matches!((ev.state.clone(), target.clone()),
                (EventState::NotOpenedYet, EventState::OpenForRegistration) |
                (EventState::OpenForRegistration, EventState::NotOpenedYet)
            ) || std::mem::discriminant(&ev.state) == std::mem::discriminant(&target);

            if allowed_transition {
                ev.state = target;
                Ok(Redirect::to(format!("/admin/events/{}", event_id)))
            } else {
                Err(Status::BadRequest)
            }
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots", data = "<form>")]
pub fn create_slot(session: Session, state: &State<AppState>, event_id: Uuid, form: Form<CreateSlotForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let form = form.into_inner();
            let name = form.name.trim().to_string();
            if name.is_empty() { return Err(Status::BadRequest); }
            let mut slot = Slot::new(name, form.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            let slot_uuid = slot.uuid;
            // slot.sessions already empty
            ev.slots.push(slot);
            Ok(Redirect::to(format!("/admin/events/{}#slot-{}", event_id, slot_uuid)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots/<slot_id>/edit", data = "<form>")]
pub fn edit_slot(session: Session, state: &State<AppState>, event_id: Uuid, slot_id: Uuid, form: Form<EditSlotForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let Some(slot) = ev.slots.iter_mut().find(|s| s.uuid == slot_id) else { return Err(Status::NotFound); };
            let form = form.into_inner();
            let name = form.name.trim().to_string();
            if name.is_empty() { return Err(Status::BadRequest); }
            slot.name = name;
            slot.description = form.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            Ok(Redirect::to(format!("/admin/events/{}#slot-{}", event_id, slot_id)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots/<slot_id>/delete")]
pub fn delete_slot(session: Session, state: &State<AppState>, event_id: Uuid, slot_id: Uuid) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            ev.slots.retain(|s| s.uuid != slot_id);
            Ok(Redirect::to(format!("/admin/events/{}", event_id)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots/<slot_id>/sessions", data = "<form>")]
pub fn create_session(session: Session, state: &State<AppState>, event_id: Uuid, slot_id: Uuid, form: Form<CreateSessionForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let Some(slot) = ev.slots.iter_mut().find(|s| s.uuid == slot_id) else { return Err(Status::NotFound); };
            let form = form.into_inner();
            let name = form.name.trim().to_string();
            if name.is_empty() || form.seats < 1 || form.seats > 10000 { return Err(Status::BadRequest); }
            let sess = EventSession::new(name, form.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()), form.seats);
            slot.sessions.push(sess);
            Ok(Redirect::to(format!("/admin/events/{}#slot-{}", event_id, slot_id)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots/<slot_id>/sessions/<session_id>/edit", data = "<form>")]
pub fn edit_session(session: Session, state: &State<AppState>, event_id: Uuid, slot_id: Uuid, session_id: Uuid, form: Form<EditSessionForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let Some(slot) = ev.slots.iter_mut().find(|s| s.uuid == slot_id) else { return Err(Status::NotFound); };
            let Some(sess) = slot.sessions.iter_mut().find(|s| s.uuid == session_id) else { return Err(Status::NotFound); };
            let form = form.into_inner();
            let name = form.name.trim().to_string();
            if name.is_empty() || form.seats < 1 || form.seats > 10000 { return Err(Status::BadRequest); }
            sess.name = name;
            sess.description = form.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            sess.seats = form.seats;
            Ok(Redirect::to(format!("/admin/events/{}#slot-{}", event_id, slot_id)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/slots/<slot_id>/sessions/<session_id>/delete")]
pub fn delete_session(session: Session, state: &State<AppState>, event_id: Uuid, slot_id: Uuid, session_id: Uuid) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            let Some(slot) = ev.slots.iter_mut().find(|s| s.uuid == slot_id) else { return Err(Status::NotFound); };
            slot.sessions.retain(|s| s.uuid != session_id);
            Ok(Redirect::to(format!("/admin/events/{}#slot-{}", event_id, slot_id)))
        }
        _ => Err(Status::Forbidden),
    }
}
