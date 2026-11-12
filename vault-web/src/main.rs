//! `vault-web` — read-only web server for a Notes vault (M5).
//!
//! Run: `vault-web [VAULT_PATH] [--port N] [--token T] [--no-auth]`.
//! The vault defaults to the last-used vault from the GUI's settings.

use std::sync::Arc;

use vault_core::Vault;
use vault_web::{config, server::ServerState};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match config::parse_args(&args, config::last_used_vault()) {
        Ok(cfg) => cfg,
        Err(error) => {
            if error.0.is_empty() {
                // --help
                println!("{}", config::USAGE);
                std::process::exit(0);
            }
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let vault = match Vault::open(&cfg.vault) {
        Ok(vault) => vault,
        Err(error) => {
            eprintln!("cannot open {}: {error}", cfg.vault.display());
            std::process::exit(1);
        }
    };

    let state = Arc::new(ServerState::new(
        vault,
        cfg.auth.clone(),
        cfg.view_folder.clone(),
    ));
    let app = vault_web::server::router(state);
    let addr = std::net::SocketAddr::new(cfg.bind, cfg.port);

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            match &cfg.auth {
                Some(token) => println!(
                    "Serving {} read-only at http://{addr}/?token={token}",
                    cfg.vault.display()
                ),
                None => println!(
                    "Serving {} read-only at http://{addr}/ (auth disabled)",
                    cfg.vault.display()
                ),
            }
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("server error: {error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("cannot bind {addr}: {error}");
            std::process::exit(1);
        }
    }
}
