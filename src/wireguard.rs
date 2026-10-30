use crate::kittengrid_api::{Endpoint, Peer};
use base64::{engine::general_purpose, Engine as _};
use std::net::ToSocketAddrs;
use std::{net::SocketAddr, str::FromStr};

use defguard_wireguard_rs::{
    key::Key, net::IpAddrMask, peer::Peer as WgPeer, InterfaceConfiguration, Kernel, WGApi,
    WireguardInterfaceApi,
};
use x25519_dalek::PublicKey;

const PORT_BASE: u16 = 51820;
const MTU: u32 = 1384;

fn network_lock() -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open("/run/lock/kittengrid-agent-network.lock")
}
fn ip(arguments: &[&str]) -> std::io::Result<()> {
    let output = std::process::Command::new("ip").args(arguments).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("Cannot configure source-based development routing; check network privileges and iproute2"))
    }
}
async fn wait_for_address(
    address: std::net::Ipv4Addr,
) -> Result<filelock::OwnedFileLockGuard, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        let file = network_lock()?;
        if let Ok(Ok(guard)) = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            filelock::FileLock::from_file(file).lock_owned_async(),
        )
        .await
        {
            let output = std::process::Command::new("ip")
                .args(["-j", "address", "show"])
                .output()?;
            if !output.status.success() {
                return Err("Cannot inspect local network addresses".into());
            }
            let entries: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let occupied = entries
                .as_array()
                .ok_or("Invalid iproute2 response")?
                .iter()
                .any(|entry| {
                    entry["addr_info"].as_array().is_some_and(|addresses| {
                        addresses.iter().any(|entry| {
                            entry["local"].as_str() == Some(address.to_string().as_str())
                        })
                    })
                });
            if !occupied {
                return Ok(guard);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                "Tunnel address is already in use locally; stop the previous agent or resolve the network-address conflict".into(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

impl Drop for WireGuard {
    fn drop(&mut self) {
        if !self.development {
            return;
        }
        // Remove the address last. A replacement cannot claim it while any
        // cleanup of this registration's routes/rules is still outstanding.
        if self.rule_installed {
            if let Some(source) = self.source {
                let _ = ip(&[
                    "rule",
                    "del",
                    "priority",
                    "10000",
                    "from",
                    &format!("{source}/32"),
                    "table",
                    &self.table.to_string(),
                ]);
            }
        }
        if let Some(network) = &self.route {
            let _ = ip(&[
                "route",
                "del",
                network,
                "dev",
                &self.interface_name,
                "table",
                &self.table.to_string(),
            ]);
        }
        if let Err(error) = self.wgapi.remove_interface() {
            log::warn!(
                "Cannot remove owned interface {}: {}",
                self.interface_name,
                error
            );
        }
    }
}

pub struct WireGuard {
    wgapi: WGApi,
    interface_name: String,
    index: usize,
    development: bool,
    source: Option<std::net::Ipv4Addr>,
    table: u32,
    route: Option<String>,
    rule_installed: bool,
}

impl std::fmt::Debug for WireGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WireGuard")
            .field("interface", &self.interface_name)
            .finish()
    }
}

impl WireGuard {
    pub fn name(&self) -> String {
        self.interface_name.clone()
    }

    pub async fn new(index: usize) -> Result<WireGuard, Box<dyn std::error::Error>> {
        let interface_name = format!("wg{}", index);

        let mut wgapi = WGApi::<Kernel>::new(interface_name.clone())?;

        // create interface
        wgapi.create_interface()?;

        Ok(WireGuard {
            wgapi,
            index,
            interface_name,
            development: false,
            source: None,
            table: 0,
            route: None,
            rule_installed: false,
        })
    }

    pub async fn new_development(
        index: usize,
        registration: uuid::Uuid,
    ) -> Result<WireGuard, Box<dyn std::error::Error>> {
        if !cfg!(target_os = "linux") {
            return Err("Local development networking currently requires Linux".into());
        }
        let interface_name = format!("kg{}{}", &registration.simple().to_string()[..10], index);
        let mut wgapi = WGApi::<Kernel>::new(interface_name.clone())?;
        wgapi.create_interface()?;
        Ok(Self {
            wgapi,
            interface_name,
            index,
            development: true,
            source: None,
            table: (registration.as_u128() as u32 & 0x7fff_ffff) + 10000 + index as u32,
            route: None,
            rule_installed: false,
        })
    }

    pub async fn set_config(
        &mut self,
        peer_config: &Peer,
        endpoint: &Endpoint,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pubkey_bytes = general_purpose::STANDARD
            .decode(endpoint.public_key())
            .unwrap();
        let bytes: [u8; 32] = pubkey_bytes.as_slice().try_into().unwrap();

        // Peer configuration
        let key = PublicKey::from(bytes);

        // Peer secret key
        let peer_key: Key = key.to_bytes().as_slice().try_into().unwrap();

        let mut peer = WgPeer::new(peer_key.clone());

        // Your WireGuard server endpoint which client connects to
        let endpoint: SocketAddr = endpoint
            .public_url()
            .as_str()
            .to_socket_addrs()?
            .next()
            .ok_or("Invalid endpoint address")?;

        // Peer endpoint and interval
        peer.endpoint = Some(endpoint);
        peer.persistent_keepalive_interval = Some(5);

        peer.allowed_ips
            .push(IpAddrMask::from_str(&peer_config.network())?);

        // interface configuration
        let interface_config = InterfaceConfiguration {
            name: self.interface_name.clone(),
            prvkey: peer_config.private_key(),
            addresses: vec![peer_config.address().to_string().parse()?],
            port: if self.development {
                0
            } else {
                PORT_BASE + self.index as u16
            },
            peers: vec![peer],
            mtu: MTU.into(),
            fwmark: None,
        };

        let _guard = if self.development {
            Some(wait_for_address(peer_config.address()).await?)
        } else {
            None
        };
        self.wgapi.configure_interface(&interface_config)?;
        if self.development {
            self.source = Some(peer_config.address());
            // Source-based routing keeps two local environments on the same WG
            // subnet from replacing each other's main-table routes.
            ip(&[
                "route",
                "add",
                &peer_config.network(),
                "dev",
                &self.interface_name,
                "table",
                &self.table.to_string(),
            ])?;
            self.route = Some(peer_config.network());
            ip(&[
                "rule",
                "add",
                "priority",
                "10000",
                "from",
                &format!("{}/32", peer_config.address()),
                "table",
                &self.table.to_string(),
            ])?;
            self.rule_installed = true;
        } else {
            self.wgapi.configure_peer_routing(&interface_config.peers)?;
        }

        Ok(())
    }
}
