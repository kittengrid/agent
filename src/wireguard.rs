use crate::kittengrid_api::{Endpoint, Peer};
use base64::{engine::general_purpose, Engine as _};
use std::net::ToSocketAddrs;
use std::{net::SocketAddr, str::FromStr};

use defguard_wireguard_rs::{
    host::Peer as WgPeer, key::Key, net::IpAddrMask, InterfaceConfiguration, Kernel, WGApi,
    WireguardInterfaceApi,
};
use x25519_dalek::PublicKey;

const PORT_BASE: u32 = 51820;
const MTU: u32 = 1384;

pub struct WireGuard {
    wgapi: WGApi,
    interface_name: String,
    index: usize,
}

impl WireGuard {
    pub fn name(&self) -> String {
        format!("wg{}", self.index).to_string()
    }

    pub async fn new(index: usize) -> Result<WireGuard, Box<dyn std::error::Error>> {
        let interface_name = format!("wg{}", index);

        let wgapi = WGApi::<Kernel>::new(interface_name.clone())?;

        // create interface
        wgapi.create_interface()?;

        Ok(WireGuard {
            wgapi,
            index,
            interface_name,
        })
    }

    pub async fn set_config(
        &self,
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
            port: PORT_BASE + self.index as u32,
            peers: vec![peer],
            mtu: MTU.into(),
        };

        self.wgapi.configure_interface(&interface_config)?;
        self.wgapi.configure_peer_routing(&interface_config.peers)?;

        Ok(())
    }
}
