use pgrx::prelude::*;
use liquid;
use handlebars::Handlebars;
use std::cell::RefCell;
use std::collections::HashMap;
use serde_json::json;
use md5;

pgrx::pg_module_magic!();

struct TemplateData {
    code: String,
    engine: String,
}

// The names of the registered templates, with the code and engine of each. 
thread_local! {
   static TEMPLATES: RefCell<HashMap<String, TemplateData>> = RefCell::new(HashMap::new());
}

// create the liquid registry
// Thread-safe lazy global variable
thread_local! {
   static LIQUID: RefCell<HashMap<String, liquid::Template>> = RefCell::new(HashMap::new());
}


// create the handlebars registry
// Thread-safe lazy global variable
thread_local! {
   static HANDLEBARS: RefCell<Handlebars<'static>> = RefCell::new(Handlebars::new());
}



fn pg_render_detect_template_engine(code: &str) -> String {
    // 1. Unieke Liquid kenmerken
    if code.contains("{%") 
        || code.contains("{#") // Liquid inline comments
    {
        return "liquid".to_string();
    }

    // 2. Unieke Handlebars kenmerken
    if code.contains("{{#") 
        || code.contains("{{/") 
        || code.contains("{{{") 
        || code.contains("{{!") // Handlebars comments
    {
        return "handlebars".to_string();
    }
    return "could not detect template engine".to_string();
}



#[pg_extern]
fn pg_render_is_registered(name: &str) -> bool {
    TEMPLATES.with(|map| {
        let map = map.borrow();
        return map.contains_key(name);
    });
}

#[pg_extern]
fn pg_render_register_template(code: &str, engine: &str, given_name: &str) -> String {
    let hash_string: String;
    let mut name: &str = given_name ;
    if "" == given_name {
        hash_string = format!("{:x}", md5::compute(code));
        name = &hash_string;
    }
    TEMPLATES.with(|map| {
        let mut map = map.borrow_mut();
        if map.contains_key(name) {
            error!("Template with name {} already registered", name);
        }
        match engine {
            "liquid" => {
                let template = liquid::ParserBuilder::with_stdlib()
                    .build().unwrap()
                    .parse(code).unwrap();
                LIQUID.with(|state| {
                    state.borrow_mut().insert(name.to_string(), template);
                });
            },
            "handlebars" => {
                HANDLEBARS.with(|state| {
                    let mut state = state.borrow_mut();
                    if let Err(e) = state.register_template_string(name, code) {
                        // register the template. The template string will be verified and compiled.
                        error!("Invalid handlebars syntax: {}", e);
                    }
                });
            },
            _ => {
                error!("Unknown template engine: {}", engine);
            }
        }
        let template_data = TemplateData {
            code: code.to_string(),
            engine: engine.to_string(),
        };
        map.insert(name.to_string(), template_data);
    });
    return name.to_string();
}

// We return a static str because we know it's returned to Postgres-land and lifetime is not an issue in Rust anymore.
// The input is a ref to Postgres memory location. When we return a String, it gets converted to Postgres text type again in pgrx.
#[pg_extern]
fn pg_render_registered_template(name: &str, data: pgrx::Json) -> String {
    let mut rv: String = "".to_string();
    TEMPLATES.with(|map| {
        let map = map.borrow();
        if let Some(template_data) = map.get(name) {
            if template_data.engine == "liquid" {
                let input = liquid::to_object(&data).unwrap();
                LIQUID.with(|state| {
                    let state = state.borrow();
                    if let Some(template) = state.get(name) {
                        let output = template.render(&input).unwrap();
                        rv = output;
                    } else {
                        error!("Cannot find liquid template named {}", name)
                    }
                });
            } else if template_data.engine == "handlebars" {
                HANDLEBARS.with(|state| {
                    let state = state.borrow();
                    match state.render(name, &json!(data)) {
                        Ok(rendered) => rv = rendered,
                        Err(e) => error!("Handlebars render error: {}", e),
                    }
                });
            } else {
                error!("Template {} has unknown engine {} ", name, template_data.engine);
            }
        } else {
            error!("Template with name {} not registered", name);
        }
    });
    return rv;
}





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





#[pg_extern]
fn pg_render_handlebars(template_code: &str, data: pgrx::Json, given_template_name: &str) -> String {
    let hash_string: String;
    let mut template_name: &str = given_template_name;

    if given_template_name == "" {
        hash_string = format!("{:x}", md5::compute(template_code));
        template_name = &hash_string; 
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
    // Now the template is always registered
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
