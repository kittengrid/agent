use crate::config::{Config, Network, Secret, Service, Volume};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug)]
pub struct Project {
    pub services: HashMap<String, Service>,
    version: Option<String>,
    networks: Option<HashMap<String, Network>>,
    volumes: Option<HashMap<String, Volume>>,
    configs: Option<HashMap<String, Config>>,
    secrets: Option<HashMap<String, Secret>>,
}

impl Project {
    pub fn summary(&self) {
        println!("The project");
    }
}
