use std::net::{TcpStream,SocketAddr};
use std::io::{Read};
use std::str::from_utf8;
use std::time::Duration;
use std::thread::sleep;
use threadpool::ThreadPool;

use serde::{Deserialize};
use serde_yaml::{self};

use statsd::Client;

#[macro_use]
extern crate log;

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
    name: String
}

#[derive(Debug, Deserialize)]
struct Statsd {
    server: String,
    prefix: String
}

fn default_connect_timeout() -> u8 {2}
fn default_check_interval() -> u8 {5}

fn check_server(addr: &str, name: &str, timeout: Duration, interval: u8, statsd_server: &str, statsd_prefix: &str) {
    let addr: SocketAddr = addr.parse().unwrap();
    let statsd_client: Client = Client::new(statsd_server, &statsd_prefix).unwrap();
    loop {
        debug!("Checking availability of the SMTP server {} [{}]", name, addr);
        let mut pipe = statsd_client.pipeline();
        // Increase attempted counter
        pipe.incr(format!("smtp.{}.attempted", name).as_str());
        /* Measure latency time */
        statsd_client.time(format!("smtp.{}", name).as_str(), || {
            let mut success: bool = false;
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(mut stream) => {
                    let mut data = [0 as u8; 3]; // using 3 byte buffer - no need to look at more
                    match stream.read_exact(&mut data) {
                        Ok(_) => {
                            match from_utf8(&data) {
                                Ok(text) => {
                                    /* Increase passed counter */
                                    pipe.incr(format!("smtp.{}.{}", name, text).as_str());
                                    success = true;
                                },
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                },
                Err(e) => {error!("Failed to connect: {}", e);}
            };
            if !success {
                pipe.incr(format!("smtp.{}.failed", name).as_str());
            };
        });
        // Send data to statsd
        pipe.send(&statsd_client);
        // Sleep interval
        sleep(Duration::from_secs(interval as u64));
    }
}

fn main() {
    env_logger::init();
    let f = std::fs::File::open("config.yaml").expect("Could not open file.");
    let config: Config = serde_yaml::from_reader(f).expect("Could not read values.");
    let pool = ThreadPool::new(4);
    for server in config.smtp_servers.iter() {
        // Copy vars to pass them into the thread
        let sa = server.addr.clone();
        let sn = server.name.clone();
        let statsd_server = config.statsd.server.clone();
        let statsd_prefix = config.statsd.prefix.clone();
        // Submit thread for each server to be monitored
        pool.execute(move || {
            check_server(
                sa.as_str(),
                sn.as_str(),
                Duration::from_secs(config.timeout as u64), 
                config.interval,
                statsd_server.as_str(), 
                statsd_prefix.as_str()
            );
        });
    }
    pool.join();
}
