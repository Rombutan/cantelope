use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Argument Parsing
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <remote_ip:port> <local_listen_port>", args[0]);
        return Ok(());
    }

    let remote_addr = args[1].clone();
    let local_port = format!("0.0.0.0:{}", args[2]);

    // 2. Setup Broadcast Channel
    // This allows the one 'source' to send data to 'N' connected clients.
    let (tx, _) = broadcast::channel::<Vec<u8>>(32);

    // 3. Task: Connect to the Source (Remote Client)
    let tx_source = tx.clone();
    // Inside your source task
    tokio::task::spawn_blocking(move || {
        // This is now a standard blocking call
        let mut stream = TcpStream::connect(remote_addr).expect("Connect failed");
        let mut buffer = [0; 4096];
        loop {
            match stream.read(&mut buffer) {
                // Standard Read, no .await
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx_source.send(buffer[..n].to_vec());
                }
                Err(_e) => break,
            }
        }
    });

    // 4. Task: Listen for Local Consumers (The Server)
    let listener = TcpListener::bind(&local_port).unwrap();
    println!("Relay server listening on {}", local_port);

    loop {
        let (socket, addr) = listener.accept().unwrap();
        println!("New subscriber connected: {}", addr);

        // Each new client gets their own receiver for the broadcast channel
        let mut rx = tx.subscribe();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        // Move the blocking write to a background thread
                        let mut socket_clone = socket.try_clone().unwrap(); // SctpStream usually supports try_clone
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            socket_clone.write_all(&msg) // Standard std::io::Write
                        })
                        .await
                        {
                            eprintln!("Write failed: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Subscriber {} lagged by {} messages", addr, n);
                    }
                    Err(_) => break, // Channel closed
                }
            }
        });
    }
}
