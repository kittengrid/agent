//! TCP forwarding for externally managed services bound to localhost (or a
//! different namespace). Listening only on the tunnel IP avoids taking over the
//! developer's local port. Targets are reconnected for every incoming stream.
use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddrV4},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};

pub(crate) async fn listen(
    address: Ipv4Addr,
    port: u16,
    host: &str,
) -> std::io::Result<Option<impl Future<Output = ()> + Send + 'static>> {
    let listener = match TcpListener::bind(SocketAddrV4::new(address, port)).await {
        Ok(listener) => listener,
        // A service listening on every host address is already tunnel-reachable.
        Err(error)
            if error.kind() == std::io::ErrorKind::AddrInUse
                && matches!(host, "localhost" | "127.0.0.1" | "::1") =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let host = host.to_owned();
    Ok(Some(async move {
        let mut streams = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((mut incoming, _)) = accepted else { break; };
                    let host = host.clone();
                    streams.spawn(async move {
                        if let Ok(Ok(mut target)) = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((host.as_str(), port))).await {
                            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut target).await;
                        }
                    });
                },
                _ = streams.join_next(), if !streams.is_empty() => {},
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn forwards_to_an_existing_local_process_without_owning_it() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = target.local_addr().unwrap().port();
        let Some(proxy) = listen(Ipv4Addr::new(127, 0, 0, 2), port, "127.0.0.1")
            .await
            .unwrap()
        else {
            panic!("proxy expected");
        };
        let forwarding = tokio::spawn(proxy);
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut buffer = [0; 4];
            stream.read_exact(&mut buffer).await.unwrap();
            assert_eq!(&buffer, b"ping");
            stream.write_all(b"pong").await.unwrap();
            target
        });
        let mut stream = TcpStream::connect((Ipv4Addr::new(127, 0, 0, 2), port))
            .await
            .unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut buffer = [0; 4];
        stream.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"pong");
        let listener = target_task.await.unwrap();
        forwarding.abort();
        let _ = forwarding.await;
        assert!(TcpStream::connect(listener.local_addr().unwrap())
            .await
            .is_ok());
    }
}
