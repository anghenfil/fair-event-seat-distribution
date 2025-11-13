use rocket_dyn_templates::Template;

#[get("/login/admin")]
pub fn login_page() -> Template {
    Template::render("login", ())
}
