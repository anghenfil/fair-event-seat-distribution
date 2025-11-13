#[macro_use] extern crate rocket;

pub mod gui;
pub mod backend;

use crate::gui::user::me;
use crate::gui::admin::{admin_index, create_event, event_view, delete_event, set_event_state, create_slot, edit_slot, delete_slot, create_session, edit_session, delete_session};
use crate::gui::login::{admin_login_page, start_page};
use backend::auth::{logout, login_admin, login_user};
use backend::state::AppState;
use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use rocket_dyn_templates::Template;
use std::path::PathBuf;
use std::time::Duration;

#[launch]
fn rocket() -> _ {
    let state_path = PathBuf::from("data/state.json");
    let app_state = AppState::load_or_new(&state_path).unwrap_or_else(|_| AppState::new());

    let state_path_for_liftoff = state_path.clone();
    let state_path_for_shutdown = state_path.clone();

    rocket::build()
        .attach(Template::fairing())
        .manage(app_state)
        .mount("/static", FileServer::from("static"))
        .attach(AdHoc::on_liftoff("autosave", move |rocket| {
            let state_path = state_path_for_liftoff.clone();
            Box::pin(async move {
                if let Some(state) = rocket.state::<AppState>() {
                    // Start async autosave every 30 seconds within Tokio runtime
                    let _handle = state.start_autosave_async(state_path.clone(), Duration::from_secs(30));
                    let _ = _handle; // detached
                }
            })
        }))
        .attach(AdHoc::on_shutdown("save_state", move |rocket| {
            let state_path = state_path_for_shutdown.clone();
            Box::pin(async move {
                if let Some(state) = rocket.state::<AppState>() {
                    let _ = state.save_to_async(&state_path).await;
                    println!("Successfully saved state to file");
                }
            })
        }))
        .mount("/", routes![
                    me,
                    start_page,
                    admin_index,
                    create_event,
                    event_view,
                    delete_event,
                    set_event_state,
                    create_slot,
                    edit_slot,
                    delete_slot,
                    create_session,
                    edit_session,
                    delete_session,
                    admin_login_page,
                    login_admin,
                    login_user,
                    logout
                ])
}
