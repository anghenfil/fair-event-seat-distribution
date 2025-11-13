use rocket::form::{Form, FromForm};
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::State;
use rocket_dyn_templates::Template;
use serde::Serialize;
use uuid::Uuid;
use std::collections::HashMap;

use crate::backend::auth::{Session, SessionUserType};
use crate::backend::data::{Application, ApplicationPriority, Event, EventState, Invitation, Participant, Slot};
use crate::backend::state::AppState;

#[derive(Serialize, Clone)]
pub struct UserEventContext {
    pub event: Event,
    pub participant: Participant,
    pub is_open: bool,
    /// per-slot selections list (optional)
    pub selections: Vec<SlotSelection>,
    /// map slot_id (string) -> selection for easier templating
    pub selections_map: std::collections::HashMap<String, SlotSelectionStr>,
    /// View-friendly slots including sessions and the user's selection per slot
    pub view_slots: Vec<ViewSlot>,
}

#[derive(Serialize, Clone)]
pub struct SlotSelection {
    pub slot_id: Uuid,
    pub first: Option<Uuid>,
    pub second: Option<Uuid>,
    pub third: Option<Uuid>,
}

#[derive(Serialize, Clone, Default)]
pub struct SlotSelectionStr {
    pub first: Option<String>,
    pub second: Option<String>,
    pub third: Option<String>,
    // Resolved human-friendly names for the selected sessions (if any)
    pub first_name: Option<String>,
    pub second_name: Option<String>,
    pub third_name: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ViewSession {
    pub uuid: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub seats: usize,
}

#[derive(Serialize, Clone)]
pub struct ViewSlot {
    pub uuid: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub sessions: Vec<ViewSession>,
    pub selection: SlotSelectionStr,
}

#[derive(FromForm)]
pub struct SaveNameForm { pub name: String }

#[derive(FromForm)]
pub struct PreferencesForm {
    pub first: Option<Uuid>,
    pub second: Option<Uuid>,
    pub third: Option<Uuid>,
}

#[get("/event")]
pub fn event_view(session: Session, state: &State<AppState>) -> Result<Template, Status> {
    let code = match &session.user_type {
        SessionUserType::User { code } => code.clone(),
        _ => return Err(Status::Forbidden),
    };

    // Acquire write lock because we may create a participant the first time
    let mut storage = state.storage.write().map_err(|_| Status::InternalServerError)?;
    let inv = match storage.invitations_codes.get(&code).cloned() {
        Some(inv) => inv,
        None => return Err(Status::Unauthorized),
    };

    let ev = match storage.events.get(&inv.event_id).cloned() {
        Some(ev) => ev,
        None => return Err(Status::NotFound),
    };

    // Ensure participant exists for this invitation, without overlapping borrows
    let participant = {
        let mut new_pid: Option<Uuid> = None;
        let pid = if let Some(pid) = inv.participant_id { pid } else {
            let p = Participant { uuid: Uuid::new_v4(), name: String::new(), points_from_previous_rounds: 0 };
            if let Some(ev_mut) = storage.events.get_mut(&inv.event_id) {
                ev_mut.participants.insert(p.uuid, p.clone());
            }
            new_pid = Some(p.uuid);
            p.uuid
        };
        // Now update invitation outside of the event mutable borrow
        if let Some(new_pid) = new_pid {
            let mut inv_new = inv.clone();
            inv_new.participant_id = Some(new_pid);
            storage.invitations_codes.insert(inv_new.code.clone(), inv_new);
        }
        // Return participant (fetch from storage)
        if let Some(ev_ro) = storage.events.get(&inv.event_id) {
            if let Some(p) = ev_ro.participants.get(&pid) {
                p.clone()
            } else {
                // Should not happen, but create a default fallback
                Participant { uuid: pid, name: String::new(), points_from_previous_rounds: 0 }
            }
        } else {
            return Err(Status::NotFound);
        }
    };

    // Build selections per slot from applications and collect session names for display
    let mut selections: Vec<SlotSelection> = Vec::new();
    let mut session_name_map: HashMap<Uuid, String> = HashMap::new();
    if let Some(ev_mut) = storage.events.get(&inv.event_id) {
        for slot in &ev_mut.slots {
            let mut sel = SlotSelection { slot_id: slot.uuid, first: None, second: None, third: None };
            for sess in &slot.sessions {
                // cache names
                session_name_map.insert(sess.uuid, sess.name.clone());
                for app in &sess.applications {
                    if app.participant == participant.uuid {
                        match app.priority {
                            ApplicationPriority::FirstPreference => sel.first = Some(sess.uuid),
                            ApplicationPriority::SecondPreference => sel.second = Some(sess.uuid),
                            ApplicationPriority::ThirdPreference => sel.third = Some(sess.uuid),
                            ApplicationPriority::NoPreference => {}
                        }
                    }
                }
            }
            selections.push(sel);
        }
    }

    // Build selections_map as strings for template convenience (also resolve names)
    let mut selections_map: HashMap<String, SlotSelectionStr> = HashMap::new();
    for sel in &selections {
        let first_str = sel.first.map(|u| u.to_string());
        let second_str = sel.second.map(|u| u.to_string());
        let third_str = sel.third.map(|u| u.to_string());
        selections_map.insert(
            sel.slot_id.to_string(),
            SlotSelectionStr {
                first: first_str,
                second: second_str,
                third: third_str,
                first_name: sel.first.and_then(|u| session_name_map.get(&u).cloned()),
                second_name: sel.second.and_then(|u| session_name_map.get(&u).cloned()),
                third_name: sel.third.and_then(|u| session_name_map.get(&u).cloned()),
            },
        );
    }
    let is_open = matches!(ev.state, EventState::OpenForRegistration);

    // Build view-friendly slots to avoid template helpers like `lookup`
    let mut view_slots: Vec<ViewSlot> = Vec::new();
    if let Some(ev_ro) = storage.events.get(&inv.event_id) {
        for slot in &ev_ro.slots {
            let sessions: Vec<ViewSession> = slot.sessions.iter().map(|s| ViewSession {
                uuid: s.uuid,
                name: s.name.clone(),
                description: s.description.clone(),
                seats: s.seats,
            }).collect();
            let selection = selections_map
                .get(&slot.uuid.to_string())
                .cloned()
                .unwrap_or_default();
            view_slots.push(ViewSlot {
                uuid: slot.uuid,
                name: slot.name.clone(),
                description: slot.description.clone(),
                sessions,
                selection,
            });
        }
    }

    let ctx = UserEventContext { event: ev, participant, is_open, selections, selections_map, view_slots };
    Ok(Template::render("user/event", &ctx))
}

#[post("/event/name", data = "<form>")]
pub fn save_name(session: Session, state: &State<AppState>, form: Form<SaveNameForm>) -> Result<Redirect, Status> {
    let code = match &session.user_type {
        SessionUserType::User { code } => code.clone(),
        _ => return Err(Status::Forbidden),
    };
    let SaveNameForm { name } = form.into_inner();
    let mut storage = state.storage.write().map_err(|_| Status::InternalServerError)?;
    let inv = match storage.invitations_codes.get(&code).cloned() { Some(i) => i, None => return Err(Status::Unauthorized) };
    let event_id = inv.event_id;
    let mut new_pid: Option<Uuid> = None;
    let pid: Uuid;
    // Scope the event mutable borrow
    {
        let Some(ev_mut) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound) };
        pid = if let Some(existing) = inv.participant_id { existing } else {
            let p = Participant { uuid: Uuid::new_v4(), name: String::new(), points_from_previous_rounds: 0 };
            ev_mut.participants.insert(p.uuid, p.clone());
            new_pid = Some(p.uuid);
            p.uuid
        };
        if let Some(p) = ev_mut.participants.get_mut(&pid) { p.name = name.trim().to_string(); }
    }
    // Update invitation mapping after releasing event borrow
    if let Some(npid) = new_pid {
        let mut inv_new = inv.clone();
        inv_new.participant_id = Some(npid);
        storage.invitations_codes.insert(inv_new.code.clone(), inv_new);
    }
    Ok(Redirect::to("/event"))
}

#[post("/event/slots/<slot_id>/preferences", data = "<form>")]
pub fn save_preferences(session: Session, state: &State<AppState>, slot_id: Uuid, form: Form<PreferencesForm>) -> Result<Redirect, Status> {
    let code = match &session.user_type {
        SessionUserType::User { code } => code.clone(),
        _ => return Err(Status::Forbidden),
    };

    let PreferencesForm { first, second, third } = form.into_inner();

    // Validate distinctness if multiple present
    let mut picks: Vec<Uuid> = Vec::new();
    for opt in [first, second, third] {
        if let Some(id) = opt { picks.push(id); }
    }
    // check duplicates
    for i in 0..picks.len() {
        for j in (i+1)..picks.len() {
            if picks[i] == picks[j] { return Err(Status::BadRequest); }
        }
    }

    let mut storage = state.storage.write().map_err(|_| Status::InternalServerError)?;
    let inv = match storage.invitations_codes.get(&code).cloned() { Some(i) => i, None => return Err(Status::Unauthorized) };
    let event_id = inv.event_id;

    // Participant must already exist and have a non-empty name
    let pid = match inv.participant_id { Some(pid) => pid, None => return Err(Status::BadRequest) };

    {
        let Some(ev_mut) = storage.events.get_mut(&event_id) else { return Err(Status::NotFound) };

        // Verify participant exists in event and has a name
        let participant_has_name = ev_mut
            .participants
            .get(&pid)
            .map(|p| !p.name.trim().is_empty())
            .unwrap_or(false);
        if !participant_has_name { return Err(Status::BadRequest); }

        // Find target slot and validate that chosen sessions belong to it
        let Some(slot) = ev_mut.slots.iter_mut().find(|s| s.uuid == slot_id) else { return Err(Status::NotFound) };
        let valid_session_ids: Vec<Uuid> = slot.sessions.iter().map(|s| s.uuid).collect();
        for id in &picks { if !valid_session_ids.contains(id) { return Err(Status::BadRequest); } }

        // Remove previous applications by this participant in this slot
        for sess in slot.sessions.iter_mut() {
            sess.applications.retain(|a| a.participant != pid);
        }

        // Insert new applications with priorities
        let mut maybe_push = |sess_id_opt: Option<Uuid>, prio: ApplicationPriority| {
            if let Some(sess_id) = sess_id_opt {
                if let Some(target) = slot.sessions.iter_mut().find(|s| s.uuid == sess_id) {
                    target.applications.push(Application { uuid: Uuid::new_v4(), session_uuid: sess_id, participant: pid, priority: prio, calculated_points: None });
                }
            }
        };
        maybe_push(first, ApplicationPriority::FirstPreference);
        maybe_push(second, ApplicationPriority::SecondPreference);
        maybe_push(third, ApplicationPriority::ThirdPreference);
    }

    Ok(Redirect::to("/event"))
}
