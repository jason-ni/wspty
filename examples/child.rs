use std::collections::HashMap;
use std::io::Write;
use std::os::unix::prelude::AsRawFd;
use std::sync::Arc;
use tokio::io::{split, ReadHalf, WriteHalf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use wspty::{PtyCommand, PtyMaster};

async fn run() -> Result<(), anyhow::Error> {
    let mut cmd = Command::new("su");
    let mut envs: HashMap<String, String> = HashMap::new();
    envs.insert(
        "PATH".to_owned(),
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
    );
    cmd.envs(&envs).args(&["-", "jason"]);

    let mut pty_cmd = PtyCommand::from(cmd);
    let mut pty_master = pty_cmd.run().await?;
    pty_master.resize(108, 38)?;
    let mut rh = pty_master.clone();
    let mut wh = pty_master.clone();

    let fut = async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        wh.write(&[113]).await?;
        Ok::<(), anyhow::Error>(())
    };
    tokio::spawn(fut);

    let mut buf: [u8; 512] = [0u8; 512];
    loop {
        let s = rh.read(&mut buf).await.map_err(|e| {
            log::debug!("read buf error: {:?}", e);
            e
        })?;
        if s.eq(&0) {
            break;
        }
        print!("{}", String::from_utf8_lossy(&buf[..s]));
    }
    Ok(())
}

fn main() {
    env_logger::init();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run());
}
