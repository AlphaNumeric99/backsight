//! Lists Tapo devices on the local network.
//!
//! ```text
//! cargo run -p tapo-camera --example discover [seconds] [ip]
//! ```

use std::time::Duration;

use tapo_camera::discovery;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let timeout = Duration::from_secs(seconds);

    let devices = match args.next() {
        Some(ip) => {
            let ip = ip.parse().expect("an IPv4 address");
            discovery::probe(ip, timeout).await?.into_iter().collect()
        }
        None => discovery::discover(timeout).await?,
    };

    if devices.is_empty() {
        println!("No Tapo devices answered within {seconds} s.");
    }
    for d in &devices {
        println!(
            "{:<15} {:<22} model={:<10} login={:?} encrypt_type={:?} https={:?} port={:?}",
            d.ip.to_string(),
            d.device_type,
            d.model.as_deref().unwrap_or("?"),
            d.login_scheme(),
            d.encrypt_types,
            d.supports_https,
            d.http_port,
        );
        if std::env::var_os("TAPO_DISCOVER_RAW").is_some() {
            println!("{:#}", d.raw);
        }
    }
    Ok(())
}
