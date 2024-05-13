use crate::kittengrid_api::Endpoint;
use base64::{engine::general_purpose, Engine as _};
use futures::stream::TryStreamExt;
use log::error;
use rtnetlink::new_connection;
use std::{net::ToSocketAddrs, thread};

use super::kittengrid_api::Peer;

use wireguard_rs::platform::tun::Status;
use wireguard_rs::platform::uapi::BindUAPI;
use wireguard_rs::platform::uapi::PlatformUAPI;
use wireguard_rs::wireguard::WireGuard as WireGuardRs;

use wireguard_rs::configuration::Configuration;
use wireguard_rs::platform::tun::PlatformTun;

const PORT_BASE: u16 = 51820;

pub struct WireGuard {
    device: WireGuardRs<wireguard_rs::platform::plt::Tun, wireguard_rs::platform::plt::UDP>,
    index: usize,
    config: wireguard_rs::configuration::WireGuardConfig<
        wireguard_rs::platform::plt::Tun,
        wireguard_rs::platform::plt::UDP,
    >,
}

impl WireGuard {
    pub fn name(&self) -> String {
        format!("wg{}", self.index).to_string()
    }

    pub fn wait(&self) {
        self.device.wait();
    }

    pub async fn new(index: usize) -> Result<WireGuard, Box<dyn std::error::Error>> {
        let device_name = format!("wg{}", index);
        let uapi = wireguard_rs::platform::plt::UAPI::bind(device_name.as_str())?;

        // create TUN device
        let (mut readers, writer, mut status) =
            wireguard_rs::platform::plt::Tun::create(device_name.as_str())?;

        // create WireGuard device
        let device: WireGuardRs<
            wireguard_rs::platform::plt::Tun,
            wireguard_rs::platform::plt::UDP,
        > = WireGuardRs::new(writer);

        // add all Tun readers
        while let Some(reader) = readers.pop() {
            device.add_tun_reader(reader);
        }

        let cfg = wireguard_rs::configuration::WireGuardConfig::new(device.clone());
        tokio::spawn({
            let cfg = cfg.clone();
            async move {
                // accept and handle UAPI config connections
                match uapi.connect() {
                    Ok(mut stream) => {
                        thread::spawn(move || {
                            wireguard_rs::configuration::uapi::handle(&mut stream, &cfg);
                        });
                    }
                    Err(err) => {
                        log::info!("UAPI connection error: {}", err);
                    }
                }
            }
        });

        tokio::spawn({
            let cfg = cfg.clone();
            async move {
                match status.event() {
                    Err(e) => {
                        log::info!("Tun device error {}", e);
                    }
                    Ok(wireguard_rs::platform::tun::TunEvent::Up(mtu)) => {
                        log::info!("Tun up (mtu = {})", mtu);
                        let _ = cfg.up(mtu); // TODO: handle
                    }
                    Ok(wireguard_rs::platform::tun::TunEvent::Down) => {
                        log::info!("Tun down");
                        cfg.down();
                    }
                }
            }
        });

        Ok(WireGuard {
            device,
            index,
            config: cfg,
        })
    }

    pub async fn set_config(
        &self,
        peer: &Peer,
        endpoint: &Endpoint,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let privkey_bytes = general_purpose::STANDARD
            .decode(peer.private_key())
            .unwrap();
        let bytes: [u8; 32] = privkey_bytes.as_slice().try_into().unwrap();
        self.config
            .set_private_key(Some(x25519_dalek::StaticSecret::from(bytes)));
        self.config.set_listen_port(PORT_BASE + self.index as u16)?;
        let endpoint_pub_key = general_purpose::STANDARD
            .decode(endpoint.public_key())
            .unwrap();

        let bytes: [u8; 32] = endpoint_pub_key.as_slice().try_into().unwrap();
        let public_key = x25519_dalek::PublicKey::from(bytes);
        self.config.add_peer(&public_key);

        let endpoint_addr = endpoint
            .public_url()
            .as_str()
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();
        self.config.set_endpoint(&public_key, endpoint_addr);

        let network = endpoint.network();
        let allowed_ips_net = network.split('/').collect::<Vec<&str>>()[0];
        let allowed_ips_cidr = network.split('/').collect::<Vec<&str>>()[1];
        self.config.add_allowed_ip(
            &public_key,
            allowed_ips_net.parse().unwrap(),
            allowed_ips_cidr.parse().unwrap(),
        );
        self.config
            .set_persistent_keepalive_interval(&public_key, 5);

        // set up ip address
        let (connection, handle, _) = new_connection().unwrap();
        tokio::spawn(connection);

        let ip_addr = peer.address();
        let mut links = handle.link().get().match_name(self.name()).execute();
        match links.try_next().await {
            Ok(Some(link)) => {
                handle
                    .address()
                    .add(link.header.index, ip_addr.into(), allowed_ips_cidr.parse()?)
                    .execute()
                    .await?;
                handle.link().set(link.header.index).up().execute().await?;
            }
            _ => {
                error!("no link found");
            }
        }

        Ok(())
    }
}
