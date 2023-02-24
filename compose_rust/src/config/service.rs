use super::*;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Service {
    //    pub deploy: Option<Deployment>,
    #[serde(deserialize_with = "super::build::optional_string_or_build")]
    pub build: Option<Build>,
    pub blkio_config: Option<BlkioConfig>,
    pub cap_add: Option<Vec<String>>,
    pub cap_drop: Option<Vec<String>>,
    pub cgroup_parent: Option<String>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub command: Option<Vec<String>>,
    #[serde(deserialize_with = "super::config::optional_array_of_strings_or_configs")]
    pub configs: Option<Vec<Config>>,
    pub container_name: Option<String>,
    pub cpu_count: Option<i32>,
    pub cpu_percent: Option<i32>,
    pub cpu_shares: Option<f32>,
    pub cpu_quota: Option<f32>,
    pub cpu_period: Option<f32>,
    pub cpu_rt_period: Option<f32>,
    pub cpu_rt_runtime: Option<f32>,
    pub cpus: Option<f32>,
    pub cpuset: Option<String>,
    pub credential_spec: Option<CredentialSpec>,
    #[serde(
        deserialize_with = "super::depends_on::optional_array_of_strings_or_ordered_hash_of_structs"
    )]
    pub depends_on: Option<IndexMap<String, DependsOn>>,
    pub device_cgroup_rules: Option<Vec<String>>,
    pub devices: Option<Vec<String>>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub dns: Option<Vec<String>>,
    pub dns_opt: Option<Vec<String>>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub dns_search: Option<Vec<String>>,
    pub domainname: Option<String>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub env_file: Option<Vec<String>>,
    #[serde(
        deserialize_with = "super::utils::optional_list_of_strings_or_hash_of_strings_option_strings"
    )]
    pub environment: Option<HashMap<String, Option<String>>>,
    #[serde(deserialize_with = "super::utils::optional_list_of_numbers_as_strings")]
    pub expose: Option<Vec<u16>>,
    //    #[serde(deserialize_with = "super::build::optional_string_or_extends")]
    //    pub extends: Option<Extends>,
    pub external_links: Option<Vec<String>>,
    pub extra_hosts: Option<Vec<String>>,
    pub group_add: Option<Vec<String>>,
    pub healthcheck: Option<Healthcheck>,
    pub hostname: Option<String>,
    pub image: Option<String>,
    pub init: bool,
    pub ipc: Option<String>,
    pub isolation: Option<String>,
    #[serde(
        deserialize_with = "super::utils::optional_list_of_strings_or_hash_of_strings_option_strings"
    )]
    pub labels: Option<HashMap<String, Option<String>>>,
    pub links: Option<Vec<String>>,
    pub logging: Option<Logging>,
    pub mac_address: Option<String>,
    pub mem_limit: Option<f32>,
    pub mem_reservation: Option<i32>,
    pub mem_swappiness: Option<i32>,
    pub memswap_limit: Option<f32>,
    pub network_mode: Option<String>,
    pub networks: Option<Vec<String>>,
    pub oom_kill_disable: bool,
    pub oom_score_adj: Option<i32>,
    pub pid: Option<String>,
    pub pids_limit: Option<f32>,
    pub platform: Option<String>,
    ports: Option<Vec<String>>,
    pub privileged: bool,
    pub profiles: Option<Vec<String>>,
    pub pull_policy: Option<String>,
    pub read_only: bool,
    pub restart: Option<String>,
    pub runtime: Option<String>,
    pub scale: Option<i32>,
    pub security_opt: Option<Vec<String>>,
    pub shm_size: Option<f32>,
    #[serde(deserialize_with = "super::secret_ref::optional_array_of_strings_or_secret_refs")]
    pub secrets: Option<Vec<SecretRef>>,

    #[serde(
        deserialize_with = "super::utils::optional_list_of_strings_or_hash_of_strings_option_strings"
    )]
    pub sysctls: Option<HashMap<String, Option<String>>>,
    pub stdin_open: bool,
    pub stop_grace_period: Option<String>,
    pub stop_signal: Option<String>,
    pub storage_opt: Option<HashMap<String, String>>,
    #[serde(deserialize_with = "super::utils::optional_string_or_list")]
    pub tmpfs: Option<Vec<String>>,
    pub tty: bool,
    // #[serde(deserialize_with = "super::ulimit::optional_integer_or_ulimit")]
    //    pub ulimits: Option<Vec<Ulimits>>,
    pub user: Option<String>,
    pub userns_mode: Option<String>,
    pub volumes: Option<Vec<String>>,
    pub volumes_from: Option<Vec<String>>,
    pub working_dir: Option<String>,
}

#[cfg(test)]
mod tests {

    use super::{Loader, LoaderError, Project, Service};

    fn process_file(contents: &str) -> Result<Project, LoaderError> {
        let sources = Vec::from([contents]);
        let loader = Loader::new(sources);

        loader.fetch_config()
    }

    fn fetch_service_from_yaml(contents: &str, service: &str) -> Service {
        let project = process_file(contents).unwrap();

        return project.services.get(service).unwrap().clone();
    }

    #[test]
    fn correctly_loads_configs_compose_file() {
        println!(
            "{:?}",
            process_file(include_str!("fixtures/service-with-configs.yaml")).unwrap()
        );
    }

    #[test]
    fn correctly_loads_secret_refs_compose_file() {
        println!(
            "{:?}",
            process_file(include_str!("fixtures/service-with-secrets.yaml")).unwrap()
        );
    }

    #[test]
    fn special_field_entrypoint() {
        let mut obtained_entrypoint = fetch_service_from_yaml(
            include_str!("fixtures/service-with-entrypoints.yaml"),
            "web",
        )
        .entrypoint
        .unwrap();
        assert!(obtained_entrypoint == ["/code/entrypoint.sh"]);
        obtained_entrypoint = fetch_service_from_yaml(
            include_str!("fixtures/service-with-entrypoints.yaml"),
            "redis",
        )
        .entrypoint
        .unwrap();
        assert!(obtained_entrypoint == ["test", "ok"]);
    }

    #[test]
    fn special_field_environments() {
        let redis_service = fetch_service_from_yaml(
            include_str!("fixtures/service-with-environments.yaml"),
            "redis",
        );
        let web_service = fetch_service_from_yaml(
            include_str!("fixtures/service-with-environments.yaml"),
            "web",
        );
        assert!(web_service.environment == redis_service.environment);
    }

    #[test]
    fn special_field_sysctls() {
        let data = include_str!("fixtures/service-with-sysctls.yaml");
        let db_service = fetch_service_from_yaml(data, "db");
        let web_service = fetch_service_from_yaml(data, "web");
        assert!(web_service.sysctls == db_service.sysctls);
    }

    #[test]
    fn correctly_loads_depends_on_compose_file() {
        println!(
            "{:?}",
            process_file(include_str!("fixtures/service-with-dependencies.yaml"))
                .unwrap()
                .services
                .get("web")
                .unwrap()
                .depends_on
        );
    }
}
