use bytemuck::{Pod, Zeroable};
use std::env;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub mod socketwrap;

// 1. Define the binary structure
// We use repr(C) to prevent the compiler from reordering fields
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CanFrame {
    timestamp: f64, // 8 bytes
    id: u32,        // 4 bytes
    _pad: u32,      // 4 explicit bytes to fill the gap
    data: [u8; 8],  // 8 bytes
} // Total = 24 bytes

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <can_interface> <local_listen_port>", args[0]);
        return Ok(());
    }

    let can_interface = args[1].clone();
    let local_port = format!("0.0.0.0:{}", args[2]);

    // 2. Broadcast channel for our struct
    let (tx, _) = broadcast::channel::<CanFrame>(100);

    // 3. CAN Polling Task
    let tx_can = tx.clone();
    tokio::task::spawn_blocking(move || {
        let mut cansocket = socketwrap::CanWrapper::new(&can_interface).unwrap();
        println!("Polling CAN: {}", can_interface);

        let _ = cansocket.parse();

        loop {
            let frame = CanFrame {
                timestamp: cansocket.get_timestamp(),
                id: cansocket.get_id(),
                _pad: 0, // Explicitly zero out the padding
                data: cansocket.get_data(),
            };

            if tx_can.receiver_count() > 0 {
                let _ = tx_can.send(frame);
            }
        }
    });

    // 4. TCP Server Task
    let listener = TcpListener::bind(&local_port).await?;
    println!("Binary TCP Relay listening on {}", local_port);

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("Client connected: {}", addr);

        let mut rx = tx.subscribe();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => {
                        // Cast the struct directly to bytes and send
                        let bytes = bytemuck::bytes_of(&frame);
                        if let Err(e) = socket.write_all(bytes).await {
                            eprintln!("Client {} disconnected: {}", addr, e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Client {} dropped {} frames", addr, n);
                    }
                    Err(_) => break,
                }
            }
        });
    }
}
