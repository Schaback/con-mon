use std::fmt;
use std::fs::File;
use std::io::Write;
use std::{process::Stdio, str::FromStr, time::Duration};

use log::info;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::oneshot::channel;
use tokio::{process::Command, time};

use anyhow::anyhow;
use anyhow::Error;
use anyhow::Result;

use log::{debug, warn, LevelFilter};
use simplelog::{ColorChoice, Config, TermLogger, TerminalMode};

use lazy_static::lazy_static;
use regex::Regex;

/// Maximum time to wait for ping before restarting
const PING_TIMEOUT: u64 = 10;

#[derive(Debug)]
struct Ping {
    timestamp: String,
    ms: u16,
}

impl fmt::Display for Ping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.timestamp, self.ms)
    }
}

impl FromStr for Ping {
    type Err = Error;
    fn from_str(string: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"\[(.+)\].*time=(\d+)").unwrap();
        }
        let cap = RE
            .captures(string)
            .ok_or(anyhow!("No capture groups found"))?;
        let timestamp = cap
            .get(1)
            .ok_or(anyhow!("Missing timestamp"))?
            .as_str()
            .to_string();
        let duration = cap
            .get(2)
            .ok_or(anyhow!("Missing ping time"))?
            .as_str()
            .parse()?;
        Ok(Self {
            timestamp,
            ms: duration,
        })
    }
}

async fn pinger() -> Result<()> {
    let mut handle = Command::new("ping")
        .arg("-D")
        .arg("1.1.1.1")
        .stdout(Stdio::piped())
        .spawn()?;

    let stdout = handle.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (send, recv) = channel::<()>();
    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    tokio::spawn(async move {
        tokio::select! {
            status = handle.wait() => {
                debug!("child status was: {}", status.expect("no status"));
            },
            _ = recv => handle.kill().await.expect("Kill failed"),
        }
    });

    let mut outfile = File::options().append(true).create(true).open("ping.log")?;

    let mut lines = reader.lines();
    loop {
        let line = time::timeout(Duration::from_secs(PING_TIMEOUT), lines.next_line());
        match line.await {
            Ok(Ok(Some(line))) => {
                // Timeout check passed
                debug!("Line: {}", line);
                match line.parse::<Ping>() {
                    Ok(ping) => {
                        outfile.write_all(format!("{}\n", ping).as_bytes())?;
                        outfile.flush()?;
                    }
                    Err(err) => warn!("Couldn't parse: {}", err),
                }
            }
            Ok(Ok(None)) => {
                println!("Task gave no more lines");
                break;
            }
            _ => {
                info!("Ping took longer than {} seconds.", PING_TIMEOUT);
                info!("Restarting pinger");
                break;
            }
        }
    }

    // Kill the ping process
    send.send(()).unwrap();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    loop {
        pinger().await?;
    }
}
