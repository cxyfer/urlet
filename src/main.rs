//! `uwurls` binary: process concerns only (CLI, logging, signals).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

use tracing_subscriber::EnvFilter;
use uwurls::config::{Config, LogFormat};
use uwurls::{db, server, VERSION};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");

    let outcome = match command {
        "serve" | "run" | "start" => serve().await,
        "migrate" => migrate().await,
        "healthcheck" => return healthcheck(&args),
        "version" | "--version" | "-V" => {
            println!("uwurls {VERSION}");
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_help();
            return ExitCode::from(2);
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<(), String> {
    let config = Config::from_env().map_err(|err| err.to_string())?;
    init_logging(&config);
    server::serve(config).await.map_err(|err| err.to_string())
}

async fn migrate() -> Result<(), String> {
    let config = Config::from_env().map_err(|err| err.to_string())?;
    init_logging(&config);
    let pool = db::init(&config).await.map_err(|err| err.to_string())?;
    let applied = db::migrate::applied_count(&pool)
        .await
        .map_err(|err| err.to_string())?;
    println!("database is up to date ({applied} migration(s) applied)");
    Ok(())
}

fn init_logging(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.log_filter))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    match config.log_format {
        LogFormat::Json => {
            let _ = builder.json().try_init();
        }
        LogFormat::Compact => {
            let _ = builder.try_init();
        }
    }
}

/// Container health probe. Implemented with a raw TCP request so the runtime
/// image does not need curl or wget.
fn healthcheck(args: &[String]) -> ExitCode {
    let url = args
        .iter()
        .skip(1)
        .find(|arg| arg.starts_with("http://") || arg.starts_with("https://"))
        .cloned()
        .or_else(|| std::env::var("UWURLS_HEALTHCHECK_URL").ok())
        .unwrap_or_else(default_healthcheck_url);

    match probe(&url) {
        Ok(()) => {
            println!("healthy: {url}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("unhealthy: {url}: {err}");
            ExitCode::FAILURE
        }
    }
}

fn default_healthcheck_url() -> String {
    let port = std::env::var("UWURLS_BIND")
        .ok()
        .and_then(|bind| bind.rsplit(':').next().map(str::to_string))
        .unwrap_or_else(|| "8080".to_string());
    format!("http://127.0.0.1:{port}/health")
}

fn probe(url: &str) -> Result<(), String> {
    let (host, port, path) = parse_http_url(url)?;
    let address = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&address).map_err(|err| err.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| err.to_string())?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nUser-Agent: uwurls-healthcheck\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| err.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| err.to_string())?;
    let status_line = response.lines().next().unwrap_or_default();
    if status_line.contains(" 200") {
        Ok(())
    } else {
        Err(format!("unexpected response: {status_line}"))
    }
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "missing scheme".to_string())?;
    let default_port = match scheme {
        "http" => 80,
        "https" => 443,
        other => return Err(format!("unsupported scheme {other}")),
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| "invalid port".to_string())?,
        ),
        None => (authority.to_string(), default_port),
    };
    if host.is_empty() {
        return Err("missing host".to_string());
    }
    Ok((host, port, path))
}

fn print_help() {
    println!(
        "uwurls {VERSION} — lightweight multi-user URL shortener

USAGE:
    uwurls [COMMAND]

COMMANDS:
    serve                 Start the HTTP server (default)
    migrate               Apply pending database migrations and exit
    healthcheck [URL]     Probe /health and exit 0 when healthy
    version               Print the version
    help                  Print this help

ENVIRONMENT:
    UWURLS_BASE_URL       Public base URL, e.g. https://s.example.com
    UWURLS_BIND           Listen address (default 0.0.0.0:8080)
    UWURLS_DATABASE_URL   SQLite URL (default sqlite:///data/uwurls.db?mode=rwc)
    UWURLS_DATA_DIR       Directory for the generated secret (default: db directory)
    UWURLS_SECRET         HMAC secret; generated into the data dir when unset
    UWURLS_ADMIN_EMAIL    Bootstrap administrator email
    UWURLS_ADMIN_PASSWORD Bootstrap administrator password
    UWURLS_REGISTRATION   open | invite | closed (overrides the settings table)
    UWURLS_CAPTCHA_PROVIDER
                          none | turnstile | recaptcha (overrides the settings table)
    UWURLS_CAPTCHA_SITE_KEY
                          Public site key of the chosen provider
    UWURLS_CAPTCHA_SECRET_KEY
                          Secret key of the chosen provider
    UWURLS_CAPTCHA_ON_REGISTER
                          Protect the registration form (true | false)
    UWURLS_CAPTCHA_ON_LOGIN
                          Protect the sign-in form (true | false)
    UWURLS_CAPTCHA_ON_CREATE
                          Protect link creation (browser sessions; true | false)
    UWURLS_SECURE_COOKIES Force the Secure cookie flag
    UWURLS_TRUST_PROXY    Honour X-Forwarded-For / X-Forwarded-Proto
    UWURLS_LOG            tracing filter (default info)
    UWURLS_LOG_FORMAT     compact | json

See docs/DEPLOYMENT.md for the full reference."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_urls() {
        assert_eq!(
            parse_http_url("http://127.0.0.1:8080/health").expect("valid"),
            ("127.0.0.1".to_string(), 8080, "/health".to_string())
        );
        assert_eq!(
            parse_http_url("http://localhost/health").expect("valid"),
            ("localhost".to_string(), 80, "/health".to_string())
        );
        assert_eq!(
            parse_http_url("https://s.example.com").expect("valid"),
            ("s.example.com".to_string(), 443, "/".to_string())
        );
        assert!(parse_http_url("ftp://example.com").is_err());
        assert!(parse_http_url("example.com").is_err());
    }
}
