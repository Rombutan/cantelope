use std::env;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use udp_stream::UdpListener;
use udp_stream::UdpStream;

pub mod tcpwrapper;
pub mod udpwrapper;

enum NetworkServer {
    Tcp(TcpListener),
    Udp(UdpListener),
}

enum NetworkClient {
    Tcp(tcpwrapper::TcpWrapper),
    Udp(udpwrapper::UdpWrapper),
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

    let input = match r_t_u.as_str() {
        "-t" => NetworkClient::Tcp(tcpwrapper::TcpWrapper::new(remote_addr.as_str())),
        "-u" => NetworkClient::Udp(udpwrapper::UdpWrapper::new(remote_addr.as_str()).await),
        _ => panic!("Unknown server type: use -t (TCP) or -u (UDP)"),
    };

    let (tx, _) = broadcast::channel::<Vec<u8>>(32);

    match input {
        NetworkClient::Tcp(mut tcp_wrapper) => {
            let tx_tcp = tx.clone();
            tokio::task::spawn_blocking(move || {
                loop {
                    if let Ok(content) = tcp_wrapper.parse() {
                        let _ = tx_tcp.send(content);
                    }
                }
            });
        }
        NetworkClient::Udp(mut udp_wrapper) => {
            let tx_udp = tx.clone();
            tokio::spawn(async move {
                loop {
                    if let Ok(content) = udp_wrapper.parse().await {
                        let _ = tx_udp.send(content);
                    }
                }
            });
        }
    }

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
