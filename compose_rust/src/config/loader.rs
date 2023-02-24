use super::Project;
use jsonschema::{Draft, JSONSchema};
use thiserror::Error;

#[derive(Debug)]
pub struct Loader<'a> {
    pub sources: Vec<&'a str>,
}

#[derive(Error, Debug)]
pub enum LoaderError {
    #[error(
        "Could not load json schema data, this should not happen as it is read at compile time."
    )]
    UnableToLoadSchema(serde_json::Error),
    #[error("Could not load yaml data")]
    InvalidYaml(serde_yaml::Error),
    #[error("Invalid schema")]
    InvalidSchema(Vec<String>),
    #[error("Unable to deserialize config")]
    ConfigError(serde_yaml::Error),
}

impl<'a> Loader<'a> {
    pub fn fetch_config(&self) -> Result<Project, LoaderError> {
        match serde_yaml::from_str(self.sources[0]) {
            Ok(project) => Ok(project),
            Err(error) => Err(LoaderError::ConfigError(error)),
        }
    }

    pub fn new(sources: Vec<&'a str>) -> Self {
        Self { sources }
    }

    pub fn validate_schema(&self) -> Result<(), LoaderError> {
        // @TODO: memoize
        let schema_json = match serde_json::from_str(include_str!("../schema/compose-spec.json")) {
            Ok(schema_json) => schema_json,
            Err(error) => return Err(LoaderError::UnableToLoadSchema(error)),
        };

        for source in self.sources.iter() {
            let source_yaml: serde_json::Value = match serde_yaml::from_str(*source) {
                Ok(source_yaml) => source_yaml,
                Err(error) => return Err(LoaderError::InvalidYaml(error)),
            };

            let compiled: JSONSchema = JSONSchema::options()
                .with_draft(Draft::Draft7)
                .compile(&schema_json)
                .unwrap();

            let result = compiled.validate(&source_yaml);
            let mut schema_errors: Vec<String> = Vec::new();

            if let Err(errors) = result {
                for error in errors {
                    // @TODO: better error reporting for schema
                    schema_errors.push(error.to_string());
                }
                return Err(LoaderError::InvalidSchema(schema_errors));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Loader, LoaderError};

    fn process_file(source: &str) -> Result<(), LoaderError> {
        let sources = Vec::from([source]);
        let loader = Loader::new(sources);

        loader.validate_schema()
    }

    #[test]
    fn correctly_loads_valid_compose_file() {
        process_file(include_str!("fixtures/simple-project.yaml")).unwrap();
    }

    #[test]
    #[should_panic]
    fn detect_yaml_errors() {
        process_file(include_str!("fixtures/invalid-yaml.yaml")).unwrap();
    }

    #[test]
    #[should_panic]
    fn detect_schema_errors() {
        process_file(include_str!("fixtures/invalid-schema.yaml")).unwrap();
    }
}
