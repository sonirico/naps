use std::path::Path;

use anyhow::Context;
use async_nats::ConnectOptions;

use crate::config::{Auth, Connection};

pub async fn connect(conn: &Connection, name: &str) -> anyhow::Result<async_nats::Client> {
    let url = &conn.url;
    connect_inner(conn, name)
        .await
        .with_context(|| format!("connect {name} to {url}"))
}

async fn connect_inner(conn: &Connection, name: &str) -> anyhow::Result<async_nats::Client> {
    let mut opts = match &conn.auth {
        Some(Auth::CredsFile(path)) => ConnectOptions::with_credentials_file(path).await?,
        _ => ConnectOptions::new(),
    }
    .name(name);

    if let Some(tls) = &conn.tls {
        opts = opts.require_tls(true);
        if let Some(ca_file) = &tls.ca_file {
            opts = opts.add_root_certificates(ca_file.clone());
        }
        if let (Some(cert_file), Some(key_file)) = (&tls.cert_file, &tls.key_file) {
            opts = opts.add_client_certificate(cert_file.clone(), key_file.clone());
        }
    }

    match &conn.auth {
        Some(Auth::NkeyFile(path)) => opts = opts.nkey(read_trimmed(path)?),
        Some(Auth::TokenFile(path)) => opts = opts.token(read_trimmed(path)?),
        Some(Auth::CredsFile(_)) | None => {}
    }

    Ok(opts.connect(&conn.url).await?)
}

fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(content.trim().to_string())
}
