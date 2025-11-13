use rocket_dyn_templates::Template;

#[get("/login/admin")]
pub fn admin_login_page() -> Template {
    Template::render("admin/login", ())
}

#[get("/")]
pub fn start_page() -> Template {
    Template::render("index", ())
}
