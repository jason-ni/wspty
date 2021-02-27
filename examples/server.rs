extern crate env_logger;
extern crate futures;
extern crate tokio;
use bytes::{BufMut, BytesMut};
use futures::SinkExt;
use futures::StreamExt;
use futures_util::stream::{SplitSink, SplitStream};
use log::{debug, error};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{accept_async, WebSocketStream};
use tungstenite::Message;
use wspty::{PtyCommand, PtyMaster};

#[tokio::main]
async fn main() {
    env_logger::init();
    ws_server()
        .await
        .map_err(|e| debug!("ws server exit with error: {:?}", e));
}

async fn handle_websocket_incoming(
    mut incoming: SplitStream<WebSocketStream<TcpStream>>,
    mut pty_shell_writer: PtyMaster,
    websocket_sender: UnboundedSender<Message>,
) -> Result<(), anyhow::Error> {
    let (stop_sender, stop_receiver) = unbounded_channel();
    while let Some(Ok(msg)) = incoming.next().await {
        match msg {
            Message::Binary(mut data) => match data[0] {
                0 => {
                    if data.len().gt(&0) {
                        println!(
                            "=== data: {}",
                            pretty_hex::pretty_hex(&data.as_slice()[1..].to_vec())
                        );
                        pty_shell_writer.write_all(&data[1..]).await?;
                    }
                }
                1 => println!("=== {}", pretty_hex::pretty_hex(&data.as_slice())),
                _ => (),
            },
            Message::Ping(data) => websocket_sender.send(Message::Pong(data))?,
            _ => (),
        };
    }
    stop_sender
        .send(())
        .map_err(|e| debug!("failed to send stop signal: {:?}", e));
    Ok(())
}

async fn handle_pty_incoming(
    mut pty_shell_reader: PtyMaster,
    websocket_sender: UnboundedSender<Message>,
) -> Result<(), anyhow::Error> {
    let fut = async move {
        let mut buffer = BytesMut::with_capacity(1024);
        loop {
            buffer.extend_from_slice(&0u8.to_be_bytes());
            buffer.resize(1024, 0u8);
            let mut tail = &mut buffer[1..];
            let n = pty_shell_reader.read_buf(&mut tail).await?;
            log::debug!("== read pty buf size: {}", n);
            if n == 0 {
                break;
            }
            match websocket_sender.send(Message::Binary(buffer[..n + 1].to_vec())) {
                Ok(_) => (),
                Err(e) => anyhow::bail!("failed to send msg to client: {:?}", e),
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    fut.await.map_err(|e| {
        log::error!("handle pty incoming error: {:?}", &e);
        e
    })
}

async fn write_to_websocket(
    mut outgoing: SplitSink<WebSocketStream<TcpStream>, Message>,
    mut receiver: UnboundedReceiver<Message>,
) -> Result<(), anyhow::Error> {
    while let Some(msg) = receiver.recv().await {
        outgoing.send(msg).await?;
    }
    Ok(())
}

async fn handle_connection(stream: TcpStream) -> Result<(), anyhow::Error> {
    let ws_stream = accept_async(stream).await?;
    let (ws_outgoing, ws_incoming) = ws_stream.split();
    let (sender, receiver) = unbounded_channel();
    let ws_sender = sender.clone();

    let mut cmd = Command::new("su");
    let mut envs: HashMap<String, String> = HashMap::new();
    envs.insert(
        "PATH".to_owned(),
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
    );
    cmd.envs(&envs).args(&["-", "jason"]);
    let mut pty_cmd = PtyCommand::from(cmd);
    let mut pty_master = pty_cmd.run().await?;

    let pty_shell_writer = pty_master.clone();
    let pty_shell_reader = pty_master.clone();

    let res = tokio::select! {
        res = handle_websocket_incoming(ws_incoming, pty_shell_writer, sender) => res,
        res = handle_pty_incoming(pty_shell_reader, ws_sender) => res,
        res = write_to_websocket(ws_outgoing, receiver) => res,
    };
    log::debug!("res = {:?}", res);
    Ok(())
}

async fn ws_server() -> Result<(), anyhow::Error> {
    let addr: SocketAddr = "127.0.0.1:7703".parse().unwrap();
    match TcpListener::bind(addr).await {
        Ok(mut listener) => {
            while let Ok((stream, peer)) = listener.accept().await {
                let fut = async move {
                    let _ = handle_connection(stream)
                        .await
                        .map_err(|e| error!("handle connection error: {:?}", e));
                };
                tokio::spawn(fut);
            }
        }
        Err(e) => return Err(anyhow::anyhow!("failed to listen: {:?}", e)),
    }
    Ok(())
}
