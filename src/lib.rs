use pgrx::prelude::*;
use liquid;
use handlebars::Handlebars;
use std::cell::RefCell;
use serde_json::json;
use md5;

pgrx::pg_module_magic!();

// We return a static str because we know it's returned to Postgres-land and lifetime is not an issue in Rust anymore.
// The input is a ref to Postgres memory location. When we return a String, it gets converted to Postgres text type again in pgrx.

#[pg_extern]
fn pg_render_liquid(template_code: &str, data: pgrx::Json) -> String {
    // Panic causes Postgres ERROR, so we can just unwrap here.
    // Will cancel the query, not crash the postgres process.
    let template = liquid::ParserBuilder::with_stdlib()
        .build().unwrap()
        .parse(template_code).unwrap();
    let input = liquid::to_object(&data).unwrap();
    let output = template.render(&input).unwrap();
    return output;
}

// create the handlebars registry
// Thread-safe lazy global variable
thread_local! {
   static HANDLEBARS: RefCell<Handlebars<'static>> = RefCell::new(Handlebars::new());
}


#[pg_extern]
fn pg_render_handlebars(template_code: &str, data: pgrx::Json, given_template_name: &str) -> String {
    // Panic causes Postgres ERROR, so we can just unwrap here.
    // Will cancel the query, not crash the postgres process.

    let hash_string: String;
    let mut template_name: &str = given_template_name;
    if given_template_name == "" {
        hash_string = format!("{:x}", md5::compute(template_code));
        template_name = &hash_string; // format!("{:x}", md5::compute(template_code)); // &hash_string;
    } 


    let mut template_present = false; // only an optimization: first check with RO access. If not present, take rw access and try again. 

    HANDLEBARS.with(|state| {
        let state = state.borrow();
        template_present = state.has_template(template_name);                            
    });
    if !template_present {
        HANDLEBARS.with(|state| {
            let mut state = state.borrow_mut();
            // Second check, for if someone else registered a template with this name after the first check!
            if !state.has_template(template_name) {
               if let Err(e) = state.register_template_string(template_name, template_code) {
                  // register the template. The template string will be verified and compiled.
                  error!("Invalid handlebars syntax: {}", e);
               }
            }
        });    
    }
    // Now the template is always        registered
    HANDLEBARS.with(|state| {
        let state = state.borrow();
        match state.render(template_name, &json!(data)) {
            Ok(rendered) => rendered,
            Err(e) => error!("Handlebars render error: {}", e),
        }

    })
}



use pgrx::extension_sql_file;
extension_sql_file!(
    "../sql/pg_render.sql",
);
