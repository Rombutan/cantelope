use bytemuck::{Pod, Zeroable};
use std::env;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use udp_stream::UdpListener;
use udp_stream::UdpStream;

mod can_interfaces;
use can_interfaces::tcpwrapper::TcpWrapper;
use can_interfaces::udpwrapper::UdpWrapper;
use can_interfaces::{CanFrameData, CanInterface, Frame};

// SocketCAN's extended-frame flag, as used on the wire
const EFF_FLAG: u32 = 0x8000_0000;

// 24-byte wire format shared with the cantelope TCP/UDP wrappers
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct WireFrame {
    timestamp: f64,
    id: u32,
    _pad: u32,
    data: [u8; 8],
}

fn encode(frame: &Frame) -> Vec<u8> {
    let mut data = [0u8; 8];
    let len = frame.data.len().min(8);
    data[..len].copy_from_slice(&frame.data[..len]);

    let wire = WireFrame {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0),
        id: if frame.extended {
            frame.id | EFF_FLAG
        } else {
            frame.id
        },
        _pad: 0,
        data,
    };
    bytemuck::bytes_of(&wire).to_vec()
}

enum NetworkServer {
    Tcp(TcpListener),
    Udp(UdpListener),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "Usage: {} -t|-u <remote_ip:port> -t|-u <local_listen_port>",
            args[0]
        );
        return Ok(());
    }

    let r_t_u = args[1].clone();
    let remote_addr = args[2].clone();
    let netmode = args[3].clone();
    let local_port = format!("0.0.0.0:{}", args[4]);

    let input: Box<dyn CanInterface + Send> = match r_t_u.as_str() {
        "-t" => Box::new(TcpWrapper::new(remote_addr.as_str()).expect("Failed to connect (TCP)")),
        "-u" => Box::new(UdpWrapper::new(remote_addr.as_str()).expect("Failed to connect (UDP)")),
        _ => panic!("Unknown server type: use -t (TCP) or -u (UDP)"),
    };

    let (tx, _) = broadcast::channel::<Vec<u8>>(32);

    // Both wrappers block in parse(), so read them on a blocking thread.
    let tx_in = tx.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            match input.parse() {
                Ok(CanFrameData { frame, .. }) => {
                    let _ = tx_in.send(encode(&frame));
                }
                Err(e) => {
                    eprintln!("Relay input error: {}", e);
                    std::thread::sleep(Duration::from_millis(1)); // don't spin on a dead link
                }
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
        _ => panic!("Unknown server type: use -t (TCP) or -u (UDP)"),
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
                    if socket.write_all(&frame).is_err() {
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
                    match socket.write(&frame).await {
                        Ok(_v) => {}
                        Err(v) => {
                            println!("{}", v);
                            return;
                        }
                    }
                }
                println!("UDP client {} disconnected", addr);
            });
        },
    }
}
