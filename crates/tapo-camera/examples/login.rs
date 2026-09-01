//! Checks a camera password and saves it in the OS credential store, so the other
//! examples can reach the camera without asking again.
//!
//! ```text
//! cargo run -p tapo-camera --example login -- <camera-ip>
//! ```
//!
//! Prompts (with hidden input) for the camera owner's TP-Link account password. A wrong
//! password is never retried: repeated failures lock the camera for about 30 minutes.

use keyring::v1::Entry;
use tapo_camera::{Camera, CameraConfig, Error};

const SERVICE: &str = "backsight-dev";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args().nth(1).ok_or("usage: login <camera-ip>")?;
    let password = rpassword::prompt_password(format!(
        "TP-Link account password of the owner of {host} (hidden): "
    ))?;

    let camera = match Camera::connect(CameraConfig::new(&host, password.clone())).await {
        Ok(camera) => camera,
        Err(Error::BadCredentials) => {
            eprintln!(
                "The camera rejected the password. Check that it is the TP-Link account \
                 password of the camera's owner, and that Third-Party Compatibility is on \
                 in the Tapo app (Me > Third-Party Services). Nothing was saved."
            );
            std::process::exit(1);
        }
        Err(err) => return Err(err.into()),
    };

    let info = camera.device_info().await?;
    let fingerprint = camera.certificate().await.ok_or("no certificate seen")?;
    Entry::new(SERVICE, &host)?.set_password(&password)?;
    Entry::new(SERVICE, &format!("{host}#cert"))?.set_password(&fingerprint.to_hex())?;

    let field = |key: &str| info.get(key).and_then(|v| v.as_str()).unwrap_or("?");
    println!(
        "Logged in to \"{}\" ({} hw {}, firmware {}), password hash {:?}.",
        field("device_alias"),
        field("device_model"),
        field("hw_version"),
        field("sw_version"),
        camera.password_hash().await,
    );
    println!("Certificate pinned: {fingerprint}");
    println!(
        "Password saved in the OS credential store (service \"{SERVICE}\", account \"{host}\")."
    );
    Ok(())
}
