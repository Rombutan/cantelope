use std::env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
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
    tokio::spawn(async move {
        println!("Connecting to source at {}...", remote_addr);
        match TcpStream::connect(&remote_addr).await {
            Ok(mut stream) => {
                println!("Connected to source!");
                let mut buffer = [0; 4096];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) => {
                            println!("Source closed connection.");
                            break;
                        }
                        Ok(n) => {
                            // Send the received bytes into the broadcast channel
                            let _ = tx_source.send(buffer[..n].to_vec());
                        }
                        Err(e) => {
                            eprintln!("Read error from source: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => eprintln!("Could not connect to source: {}", e),
        }
    });

    // 4. Task: Listen for Local Consumers (The Server)
    let listener = TcpListener::bind(&local_port).await?;
    println!("Relay server listening on {}", local_port);

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("New subscriber connected: {}", addr);

        // Each new client gets their own receiver for the broadcast channel
        let mut rx = tx.subscribe();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Err(e) = socket.write_all(&msg).await {
                            eprintln!("Subscriber {} disconnected: {}", addr, e);
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
