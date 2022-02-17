use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use threadpool::ThreadPool;

use serde::Deserialize;
use serde_yaml::{self};

use statsd::Client;

#[macro_use]
extern crate log;

use env_logger::Env;

// Use Jemalloc only for musl-64 bits platforms
#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_connect_timeout")]
    timeout: u8,
    #[serde(default = "default_check_interval")]
    interval: u8,
    smtp_servers: Vec<SmtpServer>,
    statsd: Statsd,
}

#[derive(Debug, Deserialize)]
struct SmtpServer {
    addr: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Statsd {
    server: String,
    prefix: String,
}

fn default_connect_timeout() -> u8 {
    2
}
fn default_check_interval() -> u8 {
    5
}

fn main() {
    let env = Env::default()
        .filter_or("CLOUDMON_LOG_LEVEL", "info")
        .write_style_or("CLOUDMON_LOG_STYLE", "always");

    env_logger::init_from_env(env);
    info!("Starting cloudmon-plugin-smtp");

    let terminate = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&terminate)).unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminate)).unwrap();

    let f = std::fs::File::open("config.yaml").expect("Could not open file.");
    let config: Config = serde_yaml::from_reader(f).expect("Could not read values.");
    let pool = ThreadPool::new(4);
    for server in config.smtp_servers.iter() {
        // Copy vars to pass them into the thread
        let addr = server.addr.clone();
        let name = server.name.clone();
        let statsd_server = config.statsd.server.clone();
        let statsd_prefix = config.statsd.prefix.clone();
        let timeout = Duration::from_secs(config.timeout as u64);
        let interval = Duration::from_secs(config.interval as u64);
        let terminate = terminate.clone();
        // Submit thread for each server to be monitored
        pool.execute(move || {
            let addr: SocketAddr = addr.parse().unwrap();
            let statsd_client: Client = Client::new(statsd_server, &statsd_prefix).unwrap();
            while !terminate.load(Ordering::Relaxed) {
                debug!(
                    "Checking availability of the SMTP server {} [{}]",
                    name, addr
                );
                let mut pipe = statsd_client.pipeline();
                /* Measure latency time */
                statsd_client.time(format!("smtp.{}", name).as_str(), || {
                    let mut success: bool = false;
                    match TcpStream::connect_timeout(&addr, timeout) {
                        Ok(mut stream) => {
                            /* Set socket read timeout and ignore errors */
                            stream.set_read_timeout(Some(timeout)).ok();
                            /* using 3 byte buffer - no need to look at more */
                            let mut data = [0 as u8; 3];
                            if let Ok(_) = stream.read_exact(&mut data) {
                                if let Ok(code) = from_utf8(&data) {
                                    /* Increase passed counter */
                                    pipe.incr(format!("smtp.{}.{}", name, code).as_str());
                                    trace!("Received {} from {}", code, name);
                                    success = true;
                                }
                            };
                        }
                        Err(e) => {
                            error!("Failed to connect: {}", e);
                        }
                    };
                    if !success {
                        pipe.incr(format!("smtp.{}.failed", name).as_str());
                    };
                });
                // Send data to statsd
                pipe.send(&statsd_client);
                // Sleep interval
                sleep(interval);
            }
        });
    }
    pool.join();
    info!("Stopped cloudmon-plugin-smtp");
}
