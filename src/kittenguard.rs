//! WireGuard and TCP forwarding entirely in userspace.
//!
//! Each peer owns an isolated IP stack and an ephemeral UDP socket. Tunnel
//! addresses are never assigned to the host, so registrations can overlap safely.
use crate::{
    kittengrid_api::{Endpoint, Peer as PeerConfig},
    smol_device::{SmolIpDevice, MTU},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use gotatun::{
    device::{DeviceBuilder, Peer},
    packet::{Ip, Packet, PacketBufPool},
    tun::{IpRecv, IpSend, MtuWatcher},
};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::tcp,
    time::Instant,
    wire::{HardwareAddress, IpAddress, IpCidr},
};
use std::{collections::HashSet, io, net::Ipv4Addr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use x25519_dalek::{PublicKey, StaticSecret};

#[cfg(test)]
mod tests;

const CHANNEL_CAPACITY: usize = 100;
const STREAM_CAPACITY: usize = 32;
const BUFFER_SIZE: usize = 65535;

/// A tunnel TCP port and the host/port to connect to for each incoming stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forward {
    port: u16,
    host: String,
}

impl Forward {
    pub fn new(port: u16, host: &str) -> io::Result<Self> {
        if port == 0 || host.trim().is_empty() {
            return Err(invalid("TCP forwards require a host and a nonzero port"));
        }
        let host = host.trim();
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        // Wildcard bind addresses are not connection destinations.
        let host = match host {
            "0.0.0.0" => "127.0.0.1",
            "::" => "::1",
            host => host,
        };
        Ok(Self {
            port,
            host: host.to_owned(),
        })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn decode_key(value: &str, label: &str) -> io::Result<[u8; 32]> {
    STANDARD
        .decode(value)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            invalid(&format!(
                "Invalid {label}: expected a base64-encoded 32-byte key"
            ))
        })
}

fn validate_forwards(forwards: &[Forward]) -> io::Result<()> {
    let mut ports = HashSet::new();
    for forward in forwards {
        if !ports.insert(forward.port) {
            return Err(invalid(&format!(
                "Duplicate tunnel TCP port {}",
                forward.port
            )));
        }
    }
    Ok(())
}

struct AddForward {
    forward: Forward,
    reply: oneshot::Sender<io::Result<()>>,
}

pub struct KittenGuard {
    commands: mpsc::Sender<AddForward>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for KittenGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KittenGuard")
            .field("finished", &self.task.is_finished())
            .finish()
    }
}

impl Drop for KittenGuard {
    fn drop(&mut self) {
        // Dropping the stack also aborts every local proxy. GotaTun's Device
        // shuts down its UDP/timer tasks on drop.
        self.task.abort();
    }
}

impl KittenGuard {
    pub async fn new(
        peer: &PeerConfig,
        endpoint: &Endpoint,
        forwards: Vec<Forward>,
    ) -> io::Result<Self> {
        validate_forwards(&forwards)?;
        let public_key = decode_key(&endpoint.public_key(), "endpoint public key")?;
        let private_key = decode_key(&peer.private_key(), "peer private key")?;
        let endpoint_address = tokio::net::lookup_host(endpoint.public_url())
            .await?
            .next()
            .ok_or_else(|| invalid("WireGuard endpoint resolved to no addresses"))?;
        let mut wg_peer = Peer::new(PublicKey::from(public_key))
            .with_endpoint(endpoint_address)
            .with_allowed_ip(
                peer.network()
                    .parse()
                    .map_err(|_| invalid("Invalid peer network"))?,
            );
        if !wg_peer.allowed_ips[0].is_ipv4()
            || !wg_peer.allowed_ips[0].contains(std::net::IpAddr::V4(peer.address()))
        {
            return Err(invalid("Peer address must belong to its IPv4 network"));
        }
        let prefix = wg_peer.allowed_ips[0].prefix();
        // Establish NAT mappings even before any tunneled request arrives.
        wg_peer.keepalive = Some(5);

        let (to_wg_tx, to_wg_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (from_wg_tx, from_wg_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let mut device = DeviceBuilder::new()
            .with_default_udp()
            .with_ip_pair(AppTunTx { tx: from_wg_tx }, AppTunRx::new(to_wg_rx))
            .with_listen_port(0)
            .with_private_key(StaticSecret::from(private_key))
            .with_peer(wg_peer)
            .build()
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (commands, command_rx) = mpsc::channel(16);
        let address = peer.address();
        let task = tokio::spawn(async move {
            // Either failure tears down both sides, including all local streams.
            tokio::select! {
                _ = device.wait() => log::error!("Userspace WireGuard device at {address} stopped"),
                _ = run_stack(address, prefix, forwards, from_wg_rx, to_wg_tx, command_rx) => {
                    log::error!("Userspace TCP stack at {address} stopped");
                },
            }
        });
        Ok(Self { commands, task })
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Add dynamically allocated ports (for example ttyd) without restarting
    /// the tunnel or disrupting existing connections.
    pub async fn add_forward(&self, forward: Forward) -> io::Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(AddForward { forward, reply })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Tunnel stack stopped"))?;
        response
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Tunnel stack stopped"))?
    }
}

struct AppTunTx {
    tx: mpsc::Sender<Packet<Ip>>,
}

impl IpSend for AppTunTx {
    async fn send(&mut self, packet: Packet<Ip>) -> io::Result<()> {
        self.tx
            .send(packet)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Tunnel stack stopped"))
    }
}

struct AppTunRx {
    rx: mpsc::Receiver<Packet<Ip>>,
    mtu: MtuWatcher,
}

impl AppTunRx {
    fn new(rx: mpsc::Receiver<Packet<Ip>>) -> Self {
        Self {
            rx,
            mtu: MtuWatcher::new(MTU),
        }
    }
}

impl IpRecv for AppTunRx {
    async fn recv<'a>(
        &'a mut self,
        _pool: &mut PacketBufPool,
    ) -> io::Result<impl Iterator<Item = Packet<Ip>> + Send + 'a> {
        let packet =
            self.rx.recv().await.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "Tunnel stack stopped")
            })?;
        Ok(std::iter::once(packet))
    }

    fn mtu(&self) -> MtuWatcher {
        self.mtu.clone()
    }
}

struct TcpConnection {
    handle: SocketHandle,
    target: Forward,
    to_local: Option<mpsc::Sender<Vec<u8>>>,
    from_local: Option<mpsc::Receiver<Vec<u8>>>,
    proxy: Option<JoinHandle<io::Result<()>>>,
    started: bool,
    pending: Vec<u8>,
    offset: usize,
    local_eof: bool,
}

impl TcpConnection {
    fn new(handle: SocketHandle, target: Forward) -> Self {
        Self {
            handle,
            target,
            to_local: None,
            from_local: None,
            proxy: None,
            started: false,
            pending: Vec::new(),
            offset: 0,
            local_eof: false,
        }
    }

    async fn poll(&mut self, socket: &mut tcp::Socket<'_>) {
        if !self.started
            && matches!(
                socket.state(),
                tcp::State::Established | tcp::State::CloseWait
            )
        {
            let (to_local, from_smol) = mpsc::channel(STREAM_CAPACITY);
            let (to_smol, from_local) = mpsc::channel(STREAM_CAPACITY);
            self.to_local = Some(to_local);
            self.from_local = Some(from_local);
            self.started = true;
            self.proxy = Some(tokio::spawn(proxy_local_tcp(
                self.target.clone(),
                from_smol,
                to_smol,
            )));
        }
        if !self.started {
            return;
        }

        if self.proxy.as_ref().is_some_and(|task| task.is_finished()) {
            match self.proxy.take().unwrap().await {
                Ok(Ok(())) => {}
                result => {
                    log::debug!(
                        "TCP proxy to {}:{} failed: {result:?}",
                        self.target.host,
                        self.target.port
                    );
                    socket.abort();
                    return;
                }
            }
        }

        // Do not consume bytes until the local writer has accepted them.
        if socket.can_recv() {
            if let Some(tx) = &self.to_local {
                let mut closed = false;
                let _ = socket.recv(|buffer| {
                    let n = buffer.len().min(8192);
                    match tx.try_send(buffer[..n].to_vec()) {
                        Ok(()) => (n, ()),
                        Err(mpsc::error::TrySendError::Full(_)) => (0, ()),
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            closed = true;
                            (0, ())
                        }
                    }
                });
                if closed {
                    socket.abort();
                }
            }
        }
        if !socket.may_recv() && !socket.can_recv() {
            self.to_local.take();
        }

        if self.offset == self.pending.len() {
            self.pending.clear();
            self.offset = 0;
            if let Some(rx) = &mut self.from_local {
                match rx.try_recv() {
                    Ok(data) => self.pending = data,
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => self.local_eof = true,
                }
            }
        }
        if self.offset < self.pending.len() && socket.can_send() {
            match socket.send_slice(&self.pending[self.offset..]) {
                Ok(n) => self.offset += n,
                Err(_) => socket.abort(),
            }
        }
        // Preserve the transmit half until all server data is queued, even
        // when the client has already half-closed its side of the stream.
        if self.local_eof && self.offset == self.pending.len() && socket.may_send() {
            socket.close();
        }
    }
}

impl Drop for TcpConnection {
    fn drop(&mut self) {
        if let Some(task) = &self.proxy {
            task.abort();
        }
    }
}

async fn proxy_local_tcp(
    target: Forward,
    mut from_smol: mpsc::Receiver<Vec<u8>>,
    to_smol: mpsc::Sender<Vec<u8>>,
) -> io::Result<()> {
    // Resolve on every connection so externally managed services may restart
    // or move between container addresses without restarting the agent.
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await??;
    let (mut reader, mut writer) = stream.into_split();
    let read = async move {
        let mut buffer = [0; 8192];
        loop {
            let n = reader.read(&mut buffer).await?;
            if n == 0 || to_smol.send(buffer[..n].to_vec()).await.is_err() {
                break;
            }
        }
        Ok::<_, io::Error>(())
    };
    let write = async move {
        while let Some(data) = from_smol.recv().await {
            writer.write_all(&data).await?;
        }
        writer.shutdown().await
    };
    tokio::try_join!(read, write)?;
    Ok(())
}

fn create_tcp_listener(sockets: &mut SocketSet<'_>, port: u16) -> SocketHandle {
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; BUFFER_SIZE]),
        tcp::SocketBuffer::new(vec![0; BUFFER_SIZE]),
    );
    socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(30)));
    socket.set_timeout(Some(smoltcp::time::Duration::from_secs(120)));
    socket.listen(port).expect("validated nonzero TCP port");
    sockets.add(socket)
}

async fn run_stack(
    address: Ipv4Addr,
    prefix: u8,
    forwards: Vec<Forward>,
    mut from_wg: mpsc::Receiver<Packet<Ip>>,
    to_wg: mpsc::Sender<Packet<Ip>>,
    mut commands: mpsc::Receiver<AddForward>,
) {
    let clock = std::time::Instant::now();
    let now = || Instant::from_millis(clock.elapsed().as_millis() as i64);
    let mut device = SmolIpDevice::default();
    let mut config = Config::new(HardwareAddress::Ip);
    config.random_seed = rand::random();
    let mut iface = Interface::new(config, &mut device, now());
    iface.update_ip_addrs(|addresses| {
        addresses
            .push(IpCidr::new(IpAddress::Ipv4(address), prefix))
            .unwrap();
    });
    let mut sockets = SocketSet::new(vec![]);
    let mut listeners: Vec<_> = forwards
        .into_iter()
        .map(|target| (create_tcp_listener(&mut sockets, target.port), target))
        .collect();
    let mut connections = Vec::<TcpConnection>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            received = from_wg.recv(), if device.outgoing.len() < CHANNEL_CAPACITY => {
                let Some(packet) = received else { break; };
                device.incoming.push_back(packet.into_bytes().as_ref().to_vec());
            },
            permit = to_wg.reserve(), if !device.outgoing.is_empty() => {
                let Ok(permit) = permit else { break; };
                let raw = device.outgoing.pop_front().unwrap();
                match Packet::copy_from(raw.as_slice()).try_into_ip() {
                    Ok(packet) => { permit.send(packet); },
                    Err(error) => log::warn!("Invalid outgoing tunnel packet: {error}"),
                }
            },
            command = commands.recv() => {
                let Some(AddForward { forward, reply }) = command else { break; };
                let result = if let Some((_, existing)) = listeners.iter().find(|(_, target)| target.port == forward.port) {
                    if existing == &forward { Ok(()) }
                    else { Err(invalid(&format!("Tunnel port {} is already forwarded", forward.port))) }
                } else {
                    listeners.push((create_tcp_listener(&mut sockets, forward.port), forward));
                    Ok(())
                };
                let _ = reply.send(result);
            },
            _ = tick.tick() => {},
        }

        // Never await a full outbound channel while inbound traffic is waiting:
        // that can deadlock WireGuard's receive and transmit tasks. Backpressure
        // instead pauses IP polling until the channel can drain.
        if device.outgoing.len() >= CHANNEL_CAPACITY {
            continue;
        }
        iface.poll(now(), &mut device, &mut sockets);
        for (handle, target) in &mut listeners {
            let state = sockets.get::<tcp::Socket>(*handle).state();
            if state != tcp::State::Listen {
                if matches!(
                    state,
                    tcp::State::SynReceived | tcp::State::Established | tcp::State::CloseWait
                ) {
                    connections.push(TcpConnection::new(*handle, target.clone()));
                } else {
                    sockets.remove(*handle);
                }
                *handle = create_tcp_listener(&mut sockets, target.port);
            }
        }
        for connection in &mut connections {
            connection
                .poll(sockets.get_mut::<tcp::Socket>(connection.handle))
                .await;
        }
        iface.poll_egress(now(), &mut device, &mut sockets);
        connections.retain(|connection| {
            // An RST during the handshake can put an accepted socket back
            // into LISTEN. Its replacement already exists, so discard it too.
            if matches!(
                sockets.get::<tcp::Socket>(connection.handle).state(),
                tcp::State::Closed | tcp::State::Listen
            ) {
                sockets.remove(connection.handle);
                false
            } else {
                true
            }
        });
    }
}
