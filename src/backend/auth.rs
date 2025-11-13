use rocket::form::Form;
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use rocket::State;
use std::time::{Duration, SystemTime};
use rocket::response::Redirect;
use uuid::Uuid;

use crate::backend::state::AppState;

#[derive(FromForm)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(FromForm)]
pub struct UserLoginRequest {
    pub code: String,
}


#[derive(Clone, Debug)]
pub struct Session{
    pub id: uuid::Uuid,
    pub valid_until: SystemTime,
    pub user_type: SessionUserType
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionUserType{
    Admin,
    User { code: String }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Session {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let jar = match req.guard::<&CookieJar>().await {
            Outcome::Success(j) => j,
            _ => return Outcome::Error((Status::Unauthorized, ())),
        };

        let Some(sid_cookie) = jar.get("sid") else { return Outcome::Error((Status::Unauthorized, ())); };
        let sid : Uuid = match sid_cookie.value().to_string().parse() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("couldn't parse sid cookie: {}", e);
                return Outcome::Error((Status::Unauthorized, ()));
            }
        };

        let state = match req.guard::<&State<AppState>>().await {
            Outcome::Success(s) => s,
            _ => return Outcome::Error((Status::InternalServerError, ())),
        };

        let sessions = state.sessions.read().expect("sessions poisoned");
        if let Some(sess) = sessions.get(&sid) {
            // validate expiry
            if sess.valid_until > SystemTime::now() {
                return Outcome::Success(sess.clone());
            }
        }
        Outcome::Error((Status::Unauthorized, ()))
    }
}

impl Session {
    pub fn new(user_type: SessionUserType, ttl: Duration) -> Self {
        Session { id: uuid::Uuid::new_v4(), user_type, valid_until: SystemTime::now() + ttl }
    }
}

#[post("/login/admin", data = "<form>")]
pub fn login_admin(form: Form<LoginRequest>, jar: &CookieJar, state: &State<AppState>) -> Result<Redirect, Status> {
    let form = form.into_inner();
    let ok = {
        let storage = state.storage.read().expect("storage poisoned");
        storage.verify_admin(&form.username, &form.password)
    };
    if !ok {
        return Err(Status::Unauthorized);
    }

    let sess = Session::new(SessionUserType::Admin, Duration::from_secs(24*60*60));
    let sid = sess.id.clone();
    {
        let mut sessions = state.sessions.write().expect("sessions poisoned");
        sessions.insert(sess.id.clone(), sess);
    }
    let cookie = Cookie::build(Cookie::new("sid", sid.to_string()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    jar.add(cookie);
    Ok(Redirect::to("/admin"))
}

#[post("/login", data = "<form>")]
pub fn login_user(form: Form<UserLoginRequest>, jar: &CookieJar, state: &State<AppState>) -> Result<Redirect, Status> {
    let form = form.into_inner();

    // Validate invitation code exists
    let is_valid = {
        let storage = state.storage.read().expect("storage poisoned");
        storage.invitations_codes.contains_key(&form.code)
    };

    if !is_valid {
        return Err(Status::Unauthorized);
    }

    // Create user session and set cookie, include invite code in session type
    let sess = Session::new(SessionUserType::User { code: form.code.clone() }, Duration::from_secs(24*60*60));
    let sid = sess.id.clone();
    {
        let mut sessions = state.sessions.write().expect("sessions poisoned");
        sessions.insert(sess.id.clone(), sess);
    }
    let cookie = Cookie::build(Cookie::new("sid", sid.to_string()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    jar.add(cookie);

    Ok(Redirect::to("/event"))
}

#[post("/logout")]
pub fn logout(jar: &CookieJar, state: &State<AppState>, session: Option<Session>) -> Redirect {
    if let Some(sess) = session {
        let mut sessions = state.sessions.write().expect("sessions poisoned");
        sessions.remove(&sess.id);
    }
    jar.remove(Cookie::from("sid"));
    Redirect::to("/")
}

/// Allow direct access via link: GET /invitation/<code>
/// If the code exists, create a user session, set cookie, and redirect to /event.
#[get("/invitation/<code>")]
pub fn invitation_login(code: &str, jar: &CookieJar, state: &State<AppState>) -> Result<Redirect, Status> {
    // Validate invitation code exists
    let is_valid = {
        let storage = state.storage.read().expect("storage poisoned");
        storage.invitations_codes.contains_key(code)
    };

    if !is_valid { return Err(Status::Unauthorized); }

    // Create user session and set cookie
    let sess = Session::new(SessionUserType::User { code: code.to_string() }, Duration::from_secs(24*60*60));
    let sid = sess.id.clone();
    {
        let mut sessions = state.sessions.write().expect("sessions poisoned");
        sessions.insert(sess.id.clone(), sess);
    }
    let cookie = Cookie::build(Cookie::new("sid", sid.to_string()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    jar.add(cookie);

    Ok(Redirect::to("/event"))
}