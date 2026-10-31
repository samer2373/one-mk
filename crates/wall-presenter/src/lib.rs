// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(all(feature = "m1-lab", not(debug_assertions)))]
compile_error!("presentation laboratory features require debug assertions");

#[cfg(all(target_os = "linux", feature = "gnome-bridge"))]
pub mod gnome_bridge;

#[cfg(all(target_os = "linux", feature = "m1-lab"))]
pub mod presentation_session;
#[cfg(all(target_os = "linux", feature = "m1-lab"))]
pub mod crop;
#[cfg(all(target_os = "linux", feature = "m1-lab"))]
pub mod playback;
