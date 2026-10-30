use super::*;
use gotatun::{device::Device, udp::socket::UdpSocketFactory};
use tokio::net::TcpListener;

#[test]
fn validates_ports_hosts_and_keys() {
    assert!(Forward::new(0, "localhost").is_err());
    assert!(Forward::new(8080, "").is_err());
    assert_eq!(Forward::new(8080, "0.0.0.0").unwrap().host, "127.0.0.1");
    assert_eq!(Forward::new(8080, "::").unwrap().host, "::1");
    assert!(validate_forwards(&[
        Forward::new(8080, "localhost").unwrap(),
        Forward::new(8080, "another-host").unwrap(),
    ])
    .is_err());
    assert!(decode_key("not a key", "public key").is_err());
    assert!(decode_key(&STANDARD.encode([0; 31]), "public key").is_err());
    assert_eq!(
        decode_key(&STANDARD.encode([7; 32]), "public key").unwrap(),
        [7; 32]
    );
}

/// A real encrypted WireGuard peer, with an in-memory TCP client. No TUN,
/// host routes, fixed ports, external network, or elevated permissions.
struct Client {
    _wg: Device<(UdpSocketFactory, AppTunTx, AppTunRx)>,
    ip: SmolIpDevice,
    iface: Interface,
    sockets: SocketSet<'static>,
    to_wg: mpsc::Sender<Packet<Ip>>,
    from_wg: mpsc::Receiver<Packet<Ip>>,
    clock: std::time::Instant,
    next_port: u16,
}

impl Client {
    async fn pair(forwards: Vec<Forward>) -> (Self, KittenGuard) {
        let server_secret = StaticSecret::from([1; 32]);
        let agent_secret = StaticSecret::from([2; 32]);
        let server_public = PublicKey::from(&server_secret);
        let agent_public = PublicKey::from(&agent_secret);
        let (to_wg, packets) = mpsc::channel(CHANNEL_CAPACITY);
        let (decrypted, from_wg) = mpsc::channel(CHANNEL_CAPACITY);
        let wg = DeviceBuilder::new()
            .with_default_udp()
            .with_ip_pair(AppTunTx { tx: decrypted }, AppTunRx::new(packets))
            .with_listen_port(0)
            .with_private_key(server_secret)
            .with_peer(Peer::new(agent_public).with_allowed_ip("10.99.0.2/32".parse().unwrap()))
            .build()
            .await
            .unwrap();
        let port = wg.read(async |device| device.listen_port()).await;
        let peer: PeerConfig = serde_json::from_value(serde_json::json!({
            "address": "10.99.0.2",
            "network": "10.99.0.0/24",
            "private_key": STANDARD.encode(agent_secret.to_bytes()),
            "public_key": STANDARD.encode(agent_public.as_bytes()),
        }))
        .unwrap();
        let endpoint: Endpoint = serde_json::from_value(serde_json::json!({
            "address": "10.99.0.1",
            "network": "10.99.0.0/24",
            "public_url": format!("127.0.0.1:{port}"),
            "public_key": STANDARD.encode(server_public.as_bytes()),
        }))
        .unwrap();
        let tunnel = KittenGuard::new(&peer, &endpoint, forwards).await.unwrap();
        let mut ip = SmolIpDevice::default();
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = 42;
        let mut iface = Interface::new(config, &mut ip, Instant::from_millis(0));
        iface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::new(IpAddress::v4(10, 99, 0, 1), 24))
                .unwrap();
        });
        (
            Self {
                _wg: wg,
                ip,
                iface,
                sockets: SocketSet::new(vec![]),
                to_wg,
                from_wg,
                clock: std::time::Instant::now(),
                next_port: 40000,
            },
            tunnel,
        )
    }

    async fn pump(&mut self) {
        let now = Instant::from_millis(self.clock.elapsed().as_millis() as i64);
        self.iface.poll(now, &mut self.ip, &mut self.sockets);
        while let Some(raw) = self.ip.outgoing.pop_front() {
            self.to_wg
                .send(Packet::copy_from(raw.as_slice()).try_into_ip().unwrap())
                .await
                .unwrap();
        }
        tokio::select! {
            packet = self.from_wg.recv() => {
                self.ip.incoming.push_back(packet.unwrap().into_bytes().as_ref().to_vec());
            },
            _ = tokio::time::sleep(Duration::from_millis(2)) => {},
        }
    }

    async fn request(&mut self, port: u16, data: &[u8], half_close: bool) -> Vec<u8> {
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; BUFFER_SIZE]),
            tcp::SocketBuffer::new(vec![0; BUFFER_SIZE]),
        );
        socket
            .connect(
                self.iface.context(),
                (IpAddress::v4(10, 99, 0, 2), port),
                self.next_port,
            )
            .unwrap();
        self.next_port += 1;
        let handle = self.sockets.add(socket);
        let mut sent = 0;
        let mut response = Vec::new();
        let mut established = false;
        loop {
            self.pump().await;
            let socket = self.sockets.get_mut::<tcp::Socket>(handle);
            established |= socket.state() == tcp::State::Established;
            if sent < data.len() && socket.can_send() {
                sent += socket.send_slice(&data[sent..]).unwrap();
            }
            if half_close && sent == data.len() && socket.may_send() {
                socket.close();
            }
            if socket.can_recv() {
                socket
                    .recv(|buffer| {
                        response.extend_from_slice(buffer);
                        (buffer.len(), ())
                    })
                    .unwrap();
            }
            if established && !socket.may_recv() && !socket.can_recv() {
                socket.close();
                // Dispatch the client's FIN/ACK before starting another request.
                self.pump().await;
                return response;
            }
        }
    }
}

async fn echo_server() -> (Forward, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forward = Forward::new(listener.local_addr().unwrap().port(), "localhost").unwrap();
    let task = tokio::spawn(async move {
        let mut streams = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.unwrap();
                    streams.spawn(async move {
                        let size = stream.read_u32().await.unwrap() as usize;
                        let mut data = vec![0; size];
                        stream.read_exact(&mut data).await.unwrap();
                        stream.write_all(&data).await.unwrap();
                        // Like HTTP Connection: close, finish without waiting
                        // for the client's send half to close.
                    });
                },
                _ = streams.join_next(), if !streams.is_empty() => {},
            }
        }
    });
    (forward, task)
}

fn message(body: &[u8]) -> Vec<u8> {
    let mut data = (body.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(body);
    data
}

#[tokio::test]
async fn encrypted_tunnel_handles_repeated_multiport_large_and_dynamic_requests() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let (first, first_server) = echo_server().await;
        let (second, second_server) = echo_server().await;
        let (mut client, tunnel) = Client::pair(vec![first.clone(), second.clone()]).await;
        // Repeated requests to just one forward must not require traffic on
        // the other, and replacing listeners must preserve the target.
        for port in [second.port, first.port] {
            for _ in 0..12 {
                assert_eq!(
                    client.request(port, &message(b"hello"), false).await,
                    b"hello"
                );
            }
        }
        let body = vec![0xa5; 256 * 1024];
        assert_eq!(
            client.request(first.port, &message(&body), true).await,
            body
        );
        let (terminal, terminal_server) = echo_server().await;
        tunnel.add_forward(terminal.clone()).await.unwrap();
        // Adding the same target is safe; a different target on the same port
        // must not silently replace a working forward.
        tunnel.add_forward(terminal.clone()).await.unwrap();
        assert!(tunnel
            .add_forward(Forward::new(terminal.port, "another-host").unwrap())
            .await
            .is_err());
        assert_eq!(
            client
                .request(terminal.port, &message(b"terminal"), false)
                .await,
            b"terminal"
        );
        drop(tunnel);
        first_server.abort();
        second_server.abort();
        terminal_server.abort();
    })
    .await
    .expect("encrypted userspace requests must not stall");
}

#[tokio::test]
async fn overlapping_tunnel_addresses_are_isolated() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (forward, server) = echo_server().await;
        let (mut first, first_tunnel) = Client::pair(vec![forward.clone()]).await;
        let (mut second, second_tunnel) = Client::pair(vec![forward.clone()]).await;
        let first_data = message(b"first environment");
        let second_data = message(b"second environment");
        let (first_response, second_response) = tokio::join!(
            first.request(forward.port, &first_data, false),
            second.request(forward.port, &second_data, false),
        );
        assert_eq!(first_response, b"first environment");
        assert_eq!(second_response, b"second environment");
        drop(first_tunnel);
        assert_eq!(
            second
                .request(forward.port, &message(b"still running"), false)
                .await,
            b"still running"
        );
        drop(second_tunnel);
        server.abort();
    })
    .await
    .expect("registrations with the same tunnel IP must not interfere");
}

#[tokio::test]
async fn unavailable_target_can_start_later_without_restarting_the_tunnel() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let forward = Forward::new(address.port(), "localhost").unwrap();
        drop(listener);
        let (mut client, tunnel) = Client::pair(vec![forward]).await;
        assert!(client
            .request(address.port(), &message(b"unavailable"), false)
            .await
            .is_empty());
        let listener = TcpListener::bind(address).await.unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let size = stream.read_u32().await.unwrap() as usize;
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        assert_eq!(
            client
                .request(address.port(), &message(b"restarted"), false)
                .await,
            b"restarted"
        );
        server.await.unwrap();
        drop(tunnel);
    })
    .await
    .expect("a failed target must not break the listener or later connections");
}

#[tokio::test]
async fn dropping_a_tunnel_closes_active_local_proxies() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let forward = Forward::new(listener.local_addr().unwrap().port(), "127.0.0.1").unwrap();
        let (mut client, tunnel) = Client::pair(vec![forward.clone()]).await;
        let request =
            tokio::spawn(async move { client.request(forward.port, b"ping", false).await });
        let (mut local, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4];
        local.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ping");
        drop(tunnel);
        assert_eq!(local.read(&mut bytes).await.unwrap(), 0);
        request.abort();
        // The external service listener is not owned or stopped by the agent.
        assert!(
            tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .is_ok()
        );
    })
    .await
    .expect("dropping a tunnel must terminate its proxies");
}
