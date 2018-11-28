#![feature(proc_macro_hygiene, decl_macro)]
#![feature(plugin, custom_derive)]
#![feature(custom_attribute)]

extern crate chrono;
extern crate diesel;
extern crate lil_lib;
#[macro_use]
extern crate rocket;
extern crate rocket_codegen;
extern crate rocket_contrib;
extern crate tera;

use diesel::prelude::*;
use lil_lib::models::*;
use lil_lib::simpleauth::dummy::DummyAuthenticator;
use lil_lib::simpleauth::status::{LoginRedirect, LoginStatus};
use lil_lib::simpleauth::userpass::UserPass;
use lil_lib::view_model::*;
use lil_lib::*;
use rocket::http::uri::Uri;
use rocket::http::Cookies;
use rocket::request::Form;
use rocket::response::NamedFile;
use rocket::response::Redirect;
use rocket::Request;
use rocket_contrib::templates::Template;
use std::path::{Path, PathBuf};
use tera::Context;

const PATH: &str = "http://localhost:8000";

fn main() {
    rocket::ignite()
        .register(catchers![not_found])
        .register(catchers![internal])
        .manage(create_db_pool()) // Register connection pool with Managed State
        .mount("/", routes![admin, login, login_post, logout])
        .mount("/", routes![index])
        .mount("/", routes![get_post])
        .mount("/", routes![new_post, post_to_db, edit_post, update_post])
        .mount("/", routes![static_files])
        .attach(Template::fairing())
        .launch();
}

// Check out our DbConn Request Guard!
// Our route now has access to a database connection.
// It's dereferrenced when passed into the `load()` method.
#[get("/")]
fn index(connection: DbConn) -> Template {
    use schema::posts::dsl::*;
    use schema::users::dsl::*;

    let mut context = Context::new();

    // `load()` returns all the records from each table it is called on.
    // the `posts::dsl::*` enables us to use `posts` instead of `posts::table`
    // the types <Post> and <User> are imported by `lib_blog::models::*`
    let post_list = posts
        .filter(published.eq(true))
        .order(date.desc())
        .load::<Post>(&*connection)
        .expect("Error loading posts");
    let user_list = users
        .load::<User>(&*connection)
        .expect("Error loading users");
    let post_list: Vec<PostView> = post_list.iter().map(|x| post_view(x)).collect();

    context.insert("posts", &post_list);
    context.insert("users", &user_list);
    context.insert("PATH", &PATH);

    Template::render("home", &context)
}

#[get("/post/<post_uri>")]
fn get_post(connection: DbConn, post_uri: String) -> Template {
    use schema::posts::dsl::*;

    // find current post
    let post = &posts
        .filter(title.eq(Uri::percent_decode_lossy(&post_uri.as_bytes()).to_string()))
        .filter(published.eq(true))
        .load::<Post>(&*connection)
        .expect("Error loading post");

    let mut context = Context::new();
    context.insert("PATH", &PATH);

    if post.len() > 0 {
        // find next post
        let next = &posts
            .filter(published.eq(true))
            .filter(date.gt(&post[0].date))
            .order(date.asc())
            .load::<Post>(&*connection)
            .expect("Error loading post");

        let mut nexts = String::new();
        if next.len() > 0 {
            nexts = ((&(next[0])).title).clone();
        }

        //find previous post
        let previo = &posts
            .filter(published.eq(true))
            .filter(date.lt(&post[0].date))
            .order(date.desc())
            .load::<Post>(&*connection)
            .expect("Error loading post");

        let mut previous = String::new();
        if previo.len() > 0 {
            previous = ((&(previo[0])).title).clone();
        }

        let post = post_view(&(post[0]));
        context.insert("post", &post);
        context.insert("next", &nexts);
        context.insert("previous", &previous);

        Template::render("post", &context)
    } else {
        let error = format!(
            "Sorry, '{}' is not a valid post",
            Uri::percent_decode_lossy(&post_uri.as_bytes()).to_string()
        );
        context.insert("error", &error);
        Template::render("error", &context)
    }
}

// Authetication handling
#[get("/bossing_around")]
fn admin(connection: DbConn, info: UserPass<String>) -> Template {
    use schema::posts::dsl::*;

    let mut context = Context::new();
    context.insert("PATH", &PATH);
    context.insert("info", &info.user);
    let post_list = posts
        .order(date.desc())
        .load::<Post>(&*connection)
        .expect("Error loading posts");
    let post_list: Vec<PostView> = post_list.iter().map(|x| post_view(x)).collect();
    context.insert("posts", &post_list);
    Template::render("privilege", &context)
}

#[get("/gimme_privilege", rank = 2)]
fn login() -> Template {
    let mut context = Context::new();
    context.insert("PATH", &PATH);
    Template::render("login", &context)
}

#[post("/gimme_privilege", data = "<form>")]
fn login_post(form: Form<LoginStatus<DummyAuthenticator>>, cookies: Cookies) -> LoginRedirect {
    // creates a response with either a cookie set (in case of a succesfull login)
    // or not (in case of a failure). In both cases a "Location" header is send.
    // the first parameter indicates the redirect URL when successful login,
    // the second a URL for a failed login
    form.into_inner()
        .redirect("/bossing_around", "/gimme_privilege", cookies)
}

#[get("/logout")]
fn logout(mut info: UserPass<String>) -> Redirect {
    info.logout();
    Redirect::to("/")
}

//CUD (because the read is already up there)
#[get("/bossing_around/post/new")]
fn new_post(_info: UserPass<String>) -> Template {
    let mut context = Context::new();
    context.insert("PATH", &PATH);
    Template::render("new_post", &context)
}

#[post("/bossing_around/post/new", data = "<form>")]
fn post_to_db(connection: DbConn, info: UserPass<String>, form: Form<PostForm>) -> Redirect {
    use schema::posts::dsl::*;
    let user:User = serde_json::from_str(&info.user).unwrap();
    let new_post = NewPost {
        user_id: user.id,
        title: (form.title).clone(),
        content: (form.content).clone(),
        published: form.published,
    };
    diesel::insert_into(posts)
        .values(new_post.clone())
        .execute(&*connection)
        .expect("Error inserting posts");
    Redirect::to("/bossing_around")
}

#[get("/bossing_around/post/<post_uri>/edit")]
fn edit_post(_info: UserPass<String>, connection: DbConn, post_uri: String) -> Template {
    use schema::posts::dsl::*;
    // find current post
    let post = &posts
        .filter(title.eq(Uri::percent_decode_lossy(&post_uri.as_bytes()).to_string()))
        .load::<Post>(&*connection)
        .expect("Error loading post");

    let mut context = Context::new();
    context.insert("PATH", &PATH);
    context.insert("post", &post[0]);
    Template::render("edit_post", &context)
}

#[post("/bossing_around/post/<post_uri>/edit", data = "<form>")]
fn update_post (connection: DbConn, info: UserPass<String>, form: Form<PostForm>, post_uri: String) -> Redirect {
    use schema::posts::dsl::*;
    let post = &posts
        .filter(title.eq(Uri::percent_decode_lossy(&post_uri.as_bytes()).to_string()))
        .load::<Post>(&*connection)
        .expect("Error loading post");


    let updated_row = diesel::update(posts
        .filter(title.eq(Uri::percent_decode_lossy(post_uri.as_bytes()).to_string())))
        .set((title.eq(form.title.clone()),
         content.eq(form.content.clone()),
         published.eq(form.published)))
        .execute(&*connection)
        .expect("Error inserting posts");

        Redirect::to("/bossing_around")

}

// #[get("/post/<post_uri>/delete")]

// ERROR handling
#[catch(404)]
fn not_found(req: &Request) -> Template {
    let mut context = Context::new();
    context.insert("PATH", &PATH);
    let error = format!("Sorry, '{}' is not a valid path", req.uri());
    context.insert("error", &error);
    Template::render("error", &context)
}

#[catch(500)]
fn internal(_req: &Request) -> Template {
    let mut context = Context::new();
    context.insert("PATH", &PATH);
    let error = format!("500: Internal Server Error :<");
    context.insert("error", &error);
    Template::render("error", &context)
}

#[get("/file/<file..>")]
fn static_files(file: PathBuf) -> Option<NamedFile> {
    NamedFile::open(Path::new("static/").join(file)).ok()
}
