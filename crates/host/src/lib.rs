//! `tessel-host`: the program Tessel installs in `~/.tessel-server/host/bin` on a Linux host,
//! without root. `serve` is the daemon (one per user and version, on a private Unix socket);
//! `connect --stdio` is the short-lived bridge an SSH channel runs, which starts the daemon
//! when needed and copies bytes both ways. Dropping the bridge never stops the daemon.
//! `install` is the client side of putting the program there, usable from any platform.
pub mod install;

#[cfg(target_os = "linux")]
pub mod bridge;
#[cfg(target_os = "linux")]
pub mod daemon;
#[cfg(target_os = "linux")]
mod log;
#[cfg(target_os = "linux")]
pub mod persistence;
#[cfg(target_os = "linux")]
pub mod runtime;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The architecture this program was built for, as `uname -m` names it.
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

/// What `version` prints, one line of JSON, for the client to check before attaching.
pub fn version_json() -> String {
    serde_json::json!({
        "name": "tessel-host",
        "version": VERSION,
        "arch": arch(),
        "protocol": [tessel_host_protocol::VERSION],
    })
    .to_string()
}

const USAGE: &str = "usage: tessel-host version | serve [--daemonize --method user_scope|session] | connect --stdio";

pub fn run(args: &[String]) -> i32 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["version"] | ["--version"] => {
            println!("{}", version_json());
            0
        }
        #[cfg(target_os = "linux")]
        ["serve"] => daemon::serve(false, tessel_host_protocol::PersistenceMethod::Session),
        #[cfg(target_os = "linux")]
        ["serve", "--daemonize", "--method", method] => match *method {
            "user_scope" => daemon::serve(true, tessel_host_protocol::PersistenceMethod::UserScope),
            "session" => daemon::serve(true, tessel_host_protocol::PersistenceMethod::Session),
            _ => {
                eprintln!("{USAGE}");
                2
            }
        },
        #[cfg(target_os = "linux")]
        ["connect", "--stdio"] => bridge::connect_stdio(),
        _ => {
            eprintln!("{USAGE}");
            if cfg!(target_os = "linux") { 2 } else { 3 }
        }
    }
}
