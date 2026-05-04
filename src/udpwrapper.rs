use bytemuck::{Pod, Zeroable};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

use tokio::time::{Duration, timeout};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CanFrame {
    timestamp: f64,
    id: u32,
    _pad: u32,
    data: [u8; 8],
}

pub struct UdpWrapper {
    socket: UdpSocket,
    timestamp: f64,
    id: u32,
    data: [u8; 8],
}

impl UdpWrapper {
    pub async fn new(addr: &str) -> Self {
        println!("Trying to connect to UDP");
        //let remote = SocketAddr::from_str(addr).unwrap();
        // let socket = UdpSocket::bind(SocketAddr::new(
        //     std::net::IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        //     remote.port(),
        // ))
        // .await
        // .unwrap();

        let socket_proto = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        #[cfg(not(windows))]
        socket_proto.set_reuse_address(true).unwrap();
        socket_proto.set_reuse_port(true).unwrap();
        let local_addr: SocketAddr = "0.0.0.0:1534".parse().unwrap();
        socket_proto.bind(&local_addr.into()).unwrap();

        let std_sock: std::net::UdpSocket = socket_proto.into();
        let socket: UdpSocket = tokio::net::UdpSocket::from_std(std_sock).unwrap();

        socket.connect(addr).await.unwrap();
        socket.send("Please connecto bro".as_bytes()).await.unwrap();
        println!("Connected to UDP");

        Self {
            socket: socket,
            timestamp: 0.0,
            id: 0,
            data: [0; 8],
        }
    }

    pub async fn parse(&mut self) -> Result<Vec<u8>, std::io::Error> {
        let mut buffer = vec![0u8; std::mem::size_of::<CanFrame>()];

        let mut n;
        loop {
            match timeout(Duration::from_secs(1), self.socket.recv(&mut buffer)).await {
                Ok(read_result) => {
                    // The read completed before the timeout
                    n = read_result.expect("Failed to read from udp");
                    if n == std::mem::size_of::<CanFrame>() {
                        break;
                    }
                }
                Err(_) => {
                    println!("Timeout reached. Attempting to reconnect...");

                    if let Err(e) = self.socket.send(b"timeout-ping").await {
                        eprintln!("Failed to write to stream: {}", e);
                    }
                }
            }
        }

        if n != std::mem::size_of::<CanFrame>() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Expected 24 bytes, got {}", buffer.len()),
            ));
        }

        let frame = *bytemuck::from_bytes::<CanFrame>(&buffer);
        self.timestamp = frame.timestamp;
        self.id = frame.id;
        self.data = frame.data;
        Ok(buffer)
    }

    pub fn get_timestamp(&self) -> f64 {
        self.timestamp
    }
    pub fn get_id(&self) -> u32 {
        self.id
    }
    pub fn get_data(&self) -> [u8; 8] {
        self.data
    }
}
