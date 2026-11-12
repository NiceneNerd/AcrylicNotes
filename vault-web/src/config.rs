//! Server configuration: CLI arguments, vault selection, auth token.
//!
//! Pure parsing and decisions, unit-testable without a socket (M5).

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Everything the server needs before it opens the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Vault to serve.
    pub vault: PathBuf,
    /// Bind address (127.0.0.1 by default, per spec).
    pub bind: IpAddr,
    /// Port (default 8177).
    pub port: u16,
    /// Required token; `None` disables auth (explicit `--no-auth` only).
    pub auth: Option<String>,
    /// Limit the web view to this vault-relative folder (and its subfolders).
    pub view_folder: Option<String>,
}

/// One CLI mistake, with the usage line attached.
#[derive(Debug, PartialEq, Eq)]
pub struct ArgsError(pub String);

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\n{}", self.0, USAGE)
    }
}

pub const USAGE: &str =
    "usage: vault-web [VAULT_PATH] [--port N] [--bind ADDR] [--token T] [--no-auth]
                 [--view-folder PATH]

Serves a vault read-only over HTTP. Vault defaults to the last-used vault
from the GUI's settings. Auth defaults to a generated token (printed at
startup); pass it as `?token=…` or `Authorization: Bearer …`.

--view-folder limits the view to one folder of the vault (and its
subfolders); notes outside it are neither listed nor reachable.";

/// Parse CLI args into a [`Config`]. `vault` may still be empty — the caller
/// falls back to the last-used vault (and reports it in the error).
pub fn parse_args(args: &[String], default_vault: Option<PathBuf>) -> Result<Config, ArgsError> {
    let mut vault: Option<PathBuf> = None;
    let mut bind: Option<IpAddr> = None;
    let mut port: Option<u16> = None;
    let mut token: Option<String> = None;
    let mut no_auth = false;
    let mut view_folder: Option<String> = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => {
                let Some(value) = it.next() else {
                    return Err(ArgsError("--port needs a number".into()));
                };
                port = Some(
                    value
                        .parse()
                        .map_err(|_| ArgsError(format!("--port: not a port number: {value}")))?,
                );
            }
            "--bind" => {
                let Some(value) = it.next() else {
                    return Err(ArgsError("--bind needs an IP address".into()));
                };
                bind = Some(
                    value
                        .parse()
                        .map_err(|_| ArgsError(format!("--bind: not an IP address: {value}")))?,
                );
            }
            "--token" => {
                let Some(value) = it.next() else {
                    return Err(ArgsError("--token needs a value".into()));
                };
                token = Some(value.clone());
            }
            "--no-auth" => no_auth = true,
            "--view-folder" => {
                let Some(value) = it.next() else {
                    return Err(ArgsError("--view-folder needs a value".into()));
                };
                view_folder = Some(value.to_string());
            }
            "--help" | "-h" => return Err(ArgsError(String::new())),
            other => {
                if other.starts_with('-') {
                    return Err(ArgsError(format!("unknown flag: {other}")));
                }
                if vault.is_some() {
                    return Err(ArgsError(format!("unexpected extra argument: {other}")));
                }
                vault = Some(PathBuf::from(other));
            }
        }
    }

    let vault = vault.or(default_vault).ok_or_else(|| {
        ArgsError("no vault given and no last-used vault in the GUI settings".into())
    })?;

    // Safety rule: anything but the loopback default needs an explicit auth
    // decision — either --token or --no-auth. A generated token on a LAN
    // address would be fine, but silently exposing a vault is not.
    let bind = bind.unwrap_or(std::net::IpAddr::from([127u8, 0, 0, 1]));
    let auth = if no_auth {
        if !bind.is_loopback() {
            return Err(ArgsError(
                "--no-auth on a non-loopback bind would expose the vault to the \
                 network; refuse. Use --token instead."
                    .into(),
            ));
        }
        None
    } else {
        Some(token.unwrap_or_else(generate_token))
    };

    Ok(Config {
        vault,
        bind,
        port: port.unwrap_or(8177),
        auth,
        view_folder,
    })
}

/// 16 random bytes, hex-encoded. Reads `/dev/urandom`; if that is
/// unavailable (non-Unix), falls back to a time+pid hash — weaker, but only
/// reached off the platforms the milestone targets.
pub fn generate_token() -> String {
    urandom_token().unwrap_or_else(fallback_token)
}

fn urandom_token() -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    let mut bytes = [0u8; 16];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn fallback_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(nanos);
    hasher.write_u128(std::process::id() as u128);
    format!("{:032x}", hasher.finish() as u128)
}

/// The GUI's last-used vault, from the same settings file the app writes
/// (`$XDG_CONFIG_HOME/vault-handoff/settings.json`).
pub fn last_used_vault() -> Option<PathBuf> {
    #[derive(serde::Deserialize)]
    struct LastVault {
        #[serde(default)]
        last_vault: Option<PathBuf>,
    }
    let path = dirs::config_dir()?
        .join("vault-handoff")
        .join("settings.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<LastVault>(&text).ok()?.last_vault
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_loopback_8177_generated_token() {
        let config = parse_args(&args(&["/vault"]), None).expect("positional vault only");
        assert_eq!(config.vault, PathBuf::from("/vault"));
        assert_eq!(config.bind, IpAddr::from([127, 0, 0, 1]));
        assert_eq!(config.port, 8177);
        // A token was generated (32 hex chars).
        let token = config.auth.expect("default is auth on");
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn flags_override_defaults() {
        let config = parse_args(
            &args(&[
                "/v",
                "--port",
                "9000",
                "--bind",
                "127.0.0.2",
                "--token",
                "abc",
            ]),
            None,
        )
        .unwrap();
        assert_eq!(config.port, 9000);
        assert_eq!(config.bind, IpAddr::from([127, 0, 0, 2]));
        assert_eq!(config.auth.as_deref(), Some("abc"));
    }

    #[test]
    fn default_vault_used_when_positional_missing() {
        let config = parse_args(&args(&["--no-auth"]), Some(PathBuf::from("/last")))
            .expect("falls back to last-used");
        assert_eq!(config.vault, PathBuf::from("/last"));
        assert_eq!(config.auth, None);
    }

    #[test]
    fn no_vault_anywhere_is_an_error() {
        let err = parse_args(&args(&[]), None).unwrap_err();
        assert!(err.0.contains("no vault"), "{err}");
    }

    #[test]
    fn no_auth_off_loopback_is_refused() {
        let err = parse_args(&args(&["/v", "--bind", "0.0.0.0", "--no-auth"]), None).unwrap_err();
        assert!(err.0.contains("non-loopback"), "{err}");
        // With a token, a non-loopback bind is allowed.
        let config = parse_args(&args(&["/v", "--bind", "0.0.0.0", "--token", "t"]), None).unwrap();
        assert_eq!(config.bind, IpAddr::from([0, 0, 0, 0]));
    }

    #[test]
    fn bad_values_and_unknown_flags_are_errors() {
        assert!(parse_args(&args(&["/v", "--port", "http"]), None).is_err());
        assert!(parse_args(&args(&["/v", "--bind", "hostname"]), None).is_err());
        assert!(parse_args(&args(&["/v", "--wat"]), None).is_err());
        assert!(parse_args(&args(&["/v", "--port"]), None).is_err());
        assert!(parse_args(&args(&["/a", "/b"]), None).is_err());
    }

    #[test]
    fn generated_tokens_differ() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn view_folder_is_parsed() {
        let config = parse_args(&args(&["/v", "--view-folder", "Notes/Sub"]), None).unwrap();
        assert_eq!(config.view_folder.as_deref(), Some("Notes/Sub"));
    }

    #[test]
    fn view_folder_defaults_to_none() {
        let config = parse_args(&args(&["/v"]), None).unwrap();
        assert_eq!(config.view_folder, None);
    }
}
