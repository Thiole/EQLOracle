//! why: install_or_handoff execs and never returns on the handoff path,
//! so it can't run inside `cargo test`'s own process -- this probe is
//! spawned by hand with a fake $HOME/$APPIMAGE to watch the real flow.
//! input: env HOME, APPIMAGE set by the caller
//! output: "no-handoff" line when it returns instead of exec'ing

fn main() {
    #[cfg(target_os = "linux")]
    eqlp_app::selfinstall::install_or_handoff();
    println!("no-handoff: probe ran its own copy");
}
