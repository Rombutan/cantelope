use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::net::SocketAddr;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use udp_stream::UdpListener;
use udp_stream::UdpStream;

pub mod socketwrap;

// 1. Define the binary structure
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CanFrame {
    timestamp: f64, // 8 bytes
    id: u32,        // 4 bytes
    _pad: u32,      // 4 bytes padding
    data: [u8; 8],  // 8 bytes
} // Total = 24 bytes

enum NetworkServer {
    Tcp(TcpListener),
    Udp(UdpListener), // just hold the bind address; we'll build our own UdpSocket
}

/// Per-observer state: their address and the last observe sequence number sent.
struct Observer {
    addr: SocketAddr,
    seq: u32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!(
            "Usage: {} <can_interface> -t|-u <local_listen_port>",
            args[0]
        );
        return Ok(());
    }

    let can_interface = args[1].clone();
    let netmode = args[2].clone();
    let local_port = format!("0.0.0.0:{}", args[3]);

    // 2. Broadcast channel for CanFrame
    let (tx, _) = broadcast::channel::<CanFrame>(100);

    // 3. CAN polling task
    let tx_can = tx.clone();
    tokio::task::spawn_blocking(move || {
        let mut cansocket = socketwrap::CanWrapper::new(&can_interface).unwrap();
        println!("Polling CAN: {}", can_interface);
        loop {
            if let Err(e) = cansocket.parse() {
                eprintln!("CAN parse error: {}", e);
                continue;
            }
            let frame = CanFrame {
                timestamp: cansocket.get_timestamp(),
                id: cansocket.get_id(),
                _pad: 0,
                data: cansocket.get_data(),
            };
            if tx_can.receiver_count() > 0 {
                let _ = tx_can.send(frame);
            }
        }
    });

    // 4. Server task
    let server_enum = match netmode.as_str() {
        "-t" => {
            let tlistener = TcpListener::bind(&local_port)?;
            NetworkServer::Tcp(tlistener)
        }
        "-u" => NetworkServer::Udp(
            UdpListener::bind(local_port.parse().expect("Invalid address"))
                .await
                .unwrap(),
        ),
        _ => panic!("Unknown server type: use -t (TCP) or -u (UDP/CoAP)"),
    };

    println!("Listening on {}", local_port);

    match server_enum {
        NetworkServer::Tcp(listener) => loop {
            let (mut socket, addr): (TcpStream, _) = listener.accept()?;
            let mut rx = tx.subscribe();
            println!("New TCP client at {}", addr);
            tokio::spawn(async move {
                socket.set_nodelay(true).unwrap();
                while let Ok(frame) = rx.recv().await {
                    let bytes = bytemuck::bytes_of(&frame);
                    if socket.write_all(bytes).is_err() {
                        break;
                    }
                }
                println!("TCP client {} disconnected", addr);
            });
        },

        NetworkServer::Udp(listener) => loop {
            let (mut socket, addr): (UdpStream, _) = listener.accept().await.unwrap();
            let mut rx = tx.subscribe();
            println!("New UDP client at {}", addr);
            tokio::spawn(async move {
                while let Ok(frame) = rx.recv().await {
                    let bytes = bytemuck::bytes_of(&frame);
                    socket.write(&bytes).await;
                }
                println!("UDP client {} disconnected", addr);
            });
        },
    }
}
