use rocket::form::{Form, FromForm};
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;

use crate::backend::auth::{Session, SessionUserType};
use crate::backend::data::{Event, EventState, Slot, Session as EventSession, Invitation};
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

#[derive(Serialize, Clone)]
struct AdminViewSession {
    uuid: Uuid,
    name: String,
    description: Option<String>,
    seats: usize,
    assigned_names: Vec<String>,
}

#[derive(Serialize, Clone)]
struct AdminViewSlot {
    uuid: Uuid,
    name: String,
    description: Option<String>,
    sessions: Vec<AdminViewSession>,
}

#[derive(Serialize)]
struct AdminEventContext {
    event: Event,
    invite_codes: Vec<String>,
    view_slots: Vec<AdminViewSlot>,
    can_close_and_distribute: bool,
    is_finished: bool,
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

#[derive(FromForm)]
pub struct BulkInvitesForm { pub codes: String }

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
                    let invite_codes: Vec<String> = storage
                        .invitations_codes
                        .iter()
                        .filter_map(|(code, inv)| if inv.event_id == event_id { Some(code.clone()) } else { None })
                        .collect();
                    // Build view model with assigned names (only non-empty after Finished)
                    let mut view_slots: Vec<AdminViewSlot> = Vec::new();
                    // We need access to participants map for name lookup
                    let participants = &ev.participants;
                    for slot in &ev.slots {
                        let mut v_sessions: Vec<AdminViewSession> = Vec::new();
                        for sess in &slot.sessions {
                            let assigned_names: Vec<String> = if matches!(ev.state, EventState::Finished) {
                                sess.participants.iter()
                                    .filter_map(|pid| participants.get(pid).map(|p| p.name.clone()))
                                    .collect()
                            } else { Vec::new() };
                            v_sessions.push(AdminViewSession {
                                uuid: sess.uuid,
                                name: sess.name.clone(),
                                description: sess.description.clone(),
                                seats: sess.seats,
                                assigned_names,
                            });
                        }
                        view_slots.push(AdminViewSlot {
                            uuid: slot.uuid,
                            name: slot.name.clone(),
                            description: slot.description.clone(),
                            sessions: v_sessions,
                        })
                    }
                    let can_close_and_distribute = matches!(ev.state, EventState::OpenForRegistration);
                    let is_finished = matches!(ev.state, EventState::Finished);
                    let ctx = AdminEventContext { event: ev.clone(), invite_codes, view_slots, can_close_and_distribute, is_finished };
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

#[post("/admin/events/<event_id>/close_and_distribute")]
pub fn close_and_distribute(session: Session, state: &State<AppState>, event_id: Uuid) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            let Some(ev) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound); };
            // Only allow when open for registration
            if !matches!(ev.state, EventState::OpenForRegistration) {
                return Err(Status::BadRequest);
            }
            // Move to assigning
            ev.state = EventState::AssigningSeats;
            // Rank all applications first
            let ev_clone_for_ref = ev.clone();
            for slot in ev.slots.iter_mut() {
                for sess in slot.sessions.iter_mut() {
                    sess.rank_applications(&ev_clone_for_ref);
                }
            }
            // Allocate
            ev.allocate_participants();
            // Finish
            ev.state = EventState::Finished;
            Ok(Redirect::to(format!("/admin/events/{}", event_id)))
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

#[post("/admin/events/<event_id>/invites/bulk", data = "<form>")]
pub fn add_invites_bulk(session: Session, state: &State<AppState>, event_id: Uuid, form: Form<BulkInvitesForm>) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let BulkInvitesForm { codes } = form.into_inner();
            let mut storage = state.storage.write().expect("storage poisoned");
            if !storage.events.contains_key(&event_id) { return Err(Status::NotFound); }
            for line in codes.lines() {
                let code = line.trim();
                if code.is_empty() { continue; }
                if storage.invitations_codes.contains_key(code) { continue; }
                let inv = Invitation { code: code.to_string(), event_id, participant_id: None };
                storage.invitations_codes.insert(code.to_string(), inv);
            }
            Ok(Redirect::to(format!("/admin/events/{}", event_id)))
        }
        _ => Err(Status::Forbidden),
    }
}

#[post("/admin/events/<event_id>/invites/<code>/delete")]
pub fn delete_invite(session: Session, state: &State<AppState>, event_id: Uuid, code: &str) -> Result<Redirect, Status> {
    match session.user_type {
        SessionUserType::Admin => {
            let mut storage = state.storage.write().expect("storage poisoned");
            // Look up the invite first to validate event and capture participant id
            if let Some(inv) = storage.invitations_codes.get(code).cloned() {
                if inv.event_id == event_id {
                    // If a participant was registered via this invite, remove them and their data from the event
                    if let Some(participant_id) = inv.participant_id {
                        if let Some(ev) = storage.events.get_mut(&event_id) {
                            // Remove from event participants map
                            ev.participants.remove(&participant_id);
                            // Remove from all sessions: assigned seats and applications
                            for slot in ev.slots.iter_mut() {
                                for sess in slot.sessions.iter_mut() {
                                    // remove from assigned participants
                                    sess.participants.retain(|p| *p != participant_id);
                                    // remove any applications by this participant
                                    sess.applications.retain(|a| a.participant != participant_id);
                                }
                            }
                        }
                    }
                    // Finally remove the invite code itself
                    storage.invitations_codes.remove(code);
                }
            }
            Ok(Redirect::to(format!("/admin/events/{}", event_id)))
        }
        _ => Err(Status::Forbidden),
    }
}
