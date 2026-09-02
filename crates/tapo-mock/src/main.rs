//! Runs one or more fake cameras on loopback addresses, with the real ports (443 and
//! 8800) so the app can add them like real cameras.
//!
//! ```text
//! cargo run -p tapo-mock -- --count 2 --password mock
//! ```
//! Then add `127.0.0.2`, `127.0.0.3`, … in Backsight with the password `mock`.

use std::net::{IpAddr, Ipv4Addr};

use clap::Parser;
use tapo_mock::{MockCamera, MockOptions};

#[derive(Parser)]
#[command(about = "Fake Tapo cameras for Backsight development")]
struct Args {
    /// Number of cameras, on 127.0.0.2, 127.0.0.3, …
    #[arg(long, default_value_t = 2)]
    count: u8,
    /// Password the cameras accept.
    #[arg(long, default_value = "mock")]
    password: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let names = [
        "Front Door",
        "Garden",
        "Garage",
        "Living Room",
        "Driveway",
        "Nursery",
    ];
    let models = ["C200", "C320WS", "C210", "C520WS", "C325WB", "C220"];
    let mut cameras = Vec::new();
    for i in 0..args.count {
        let mut options = MockOptions::new(&args.password);
        options.ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2 + i));
        options.control_port = 443;
        options.media_port = 8800;
        options.name = names[usize::from(i) % names.len()].into();
        options.model = models[usize::from(i) % models.len()].into();
        let camera = MockCamera::start(options).await?;
        println!(
            "mock camera {} listening on {} (media {})",
            i + 1,
            camera.control_addr,
            camera.media_addr
        );
        cameras.push(camera);
    }
    println!("password: {}   (Ctrl+C to stop)", args.password);
    tokio::signal::ctrl_c().await?;
    Ok(())
}
