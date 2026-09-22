//! Syncs the EQ bar's color with the color of a device in OpenRGB, via
//! *polling* (periodically reading a color snapshot from that device) — NOT
//! by registering trofeo-lcd as a device controlled by OpenRGB.
//!
//! Enable it with `--openrgb-device <name-or-partial-name>` on the
//! command line (see `main.rs`). Requires OpenRGB running with the SDK
//! Server enabled (Settings > SDK Server > Enable, default port 6742, used
//! by `OpenRgbClient::connect()`'s built-in default with no manual setup
//! needed).
//!
//! Because this is polling (not a registered SDK device), if the source
//! device in OpenRGB has an ANIMATED EFFECT applied to it (rainbow/breathing/
//! etc), what gets read here is just 1 color snapshot per poll — so it will
//! change but "stutter" according to `--openrgb-poll-ms`, not as smoothly as
//! the original animation in OpenRGB. Works best when the source device is
//! set to a STATIC color.
//!
//! NOTE: this part hasn't been compile-checked yet in the development
//! sandbox (the apt toolchain available there is only Rust 1.75, while the
//! `openrgb2` crate needs edition2024 / Rust >=1.85) — unlike rusb/wasapi/etc
//! elsewhere in this project, which have passed `cargo check` in that same
//! sandbox. The code was written & manually reviewed against the official
//! API at docs.rs/openrgb2, but please report any compile errors during
//! `cargo build` on your machine (most likely just a slight difference in
//! method/type names from the version reviewed).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use openrgb2::OpenRgbClient;

/// The last color successfully read from OpenRGB (RGB 0-255), shared with
/// the main thread via a `Mutex`. `None` means no successful connect+read
/// has happened yet since the program started — callers should use a
/// fallback color (e.g. `ColorMode::Default` or the `--color` value) while
/// it's still `None`.
pub type SharedColor = Arc<Mutex<Option<(u8, u8, u8)>>>;

/// Run OpenRGB polling on a separate thread & tokio runtime (so it doesn't
/// disturb the main sync loop). Keeps auto-reconnecting if OpenRGB isn't
/// running yet / the SDK Server isn't enabled / the device hasn't been found
/// — the main program keeps running normally (using the fallback color) the
/// whole time.
pub fn spawn(device_match: String, poll_interval: Duration) -> SharedColor {
    let shared: SharedColor = Arc::new(Mutex::new(None));
    let shared_thread = Arc::clone(&shared);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for the OpenRGB client");
        rt.block_on(run(device_match, poll_interval, shared_thread));
    });

    shared
}

async fn run(device_match: String, poll_interval: Duration, shared: SharedColor) {
    let mut warned_not_found = false;
    loop {
        match OpenRgbClient::connect().await {
            Ok(client) => {
                println!(
                    "OpenRGB: connected, looking for a device whose name contains '{device_match}'..."
                );
                loop {
                    match poll_once(&client, &device_match).await {
                        Ok(Some(color)) => {
                            *shared.lock().unwrap() = Some(color);
                            warned_not_found = false;
                        }
                        Ok(None) => {
                            if !warned_not_found {
                                eprintln!(
                                    "OpenRGB: no device found with a name containing \
                                     '{device_match}'. Match it against the name shown in the \
                                     OpenRGB app (click a device in the left panel)."
                                );
                                warned_not_found = true;
                            }
                        }
                        Err(e) => {
                            eprintln!("OpenRGB: connection lost ({e}), trying to reconnect...");
                            break; // exit the inner loop -> reconnect in the outer loop
                        }
                    }
                    tokio::time::sleep(poll_interval).await;
                }
            }
            Err(e) => {
                eprintln!(
                    "OpenRGB: failed to connect ({e}) — make sure OpenRGB is running & the SDK \
                     Server is enabled (Settings > SDK Server > Enable). Trying again in 5 \
                     seconds..."
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Find the first controller whose name contains `device_match`
/// (case-insensitive, substring), returning its first LED's color.
async fn poll_once(
    client: &OpenRgbClient,
    device_match: &str,
) -> openrgb2::OpenRgbResult<Option<(u8, u8, u8)>> {
    let controllers = client.get_all_controllers().await?;
    let needle = device_match.to_ascii_lowercase();
    for c in controllers.iter() {
        if c.name().to_ascii_lowercase().contains(&needle) {
            if let Some(color) = c.colors().first() {
                return Ok(Some((color.r, color.g, color.b)));
            }
        }
    }
    Ok(None)
}
