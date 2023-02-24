use jsonschema::{Draft, JSONSchema};
mod config;
use std::env;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let schema_json = serde_json::from_str(include_str!("schema/compose-spec.json"))?;

    let compose_file = match std::fs::read_to_string(&args[1]) {
        Ok(data) => data,
        Err(e) => {
            match e.kind() {
                std::io::ErrorKind::NotFound => println!("Error, File ({}) not found", &args[1]),
                _ => println!("Error trying to open the file"),
            };
            return Err(Box::new(e));
        }
    };

    let compose_yaml = serde_yaml::from_str(compose_file.as_str())?;
    let project: config::project::Project = serde_yaml::from_str(compose_file.as_str())?;
    project.summary();
    println!("{:?}", project);

    let compiled = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema_json)
        .unwrap();

    let result = compiled.validate(&compose_yaml);
    if let Err(errors) = result {
        for error in errors {
            println!("Validation error: {}", error);
            println!("Instance path: {}", error.instance_path);
        }
    }

    println!("{:?}", project);

    Ok(())
}
