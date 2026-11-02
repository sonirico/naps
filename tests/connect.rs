mod common;

use std::path::Path;

use common::{TestNats, TestNatsOpts, TestTls, new_test_nats};
use naps::config::{Auth, Connection, Tls};
use naps::connect::connect;

const TEST_TOKEN: &str = "s3cr3t";

fn new_test_connection(nats: &TestNats, tls: Option<Tls>, auth: Option<Auth>) -> Connection {
    Connection {
        url: nats.url.clone(),
        tls,
        auth,
    }
}

fn new_test_token_file(dir: &Path, token: &str) -> std::path::PathBuf {
    let path = dir.join("token");
    std::fs::write(&path, format!("{token}\n")).expect("write token file");
    path
}

async fn assert_connected(conn: &Connection) {
    let client = connect(conn, "naps-test").await.expect("connect");
    client.flush().await.expect("flush");
}

#[tokio::test]
async fn test_connect_plain() {
    let nats = new_test_nats(TestNatsOpts::default()).await;

    assert_connected(&new_test_connection(&nats, None, None)).await;
}

#[tokio::test]
async fn test_connect_server_tls() {
    let nats = new_test_nats(TestNatsOpts {
        tls: TestTls::ServerOnly,
        ..Default::default()
    })
    .await;
    let certs = nats.certs.as_ref().expect("certs");
    let tls = Tls {
        ca_file: Some(certs.ca_file.clone()),
        cert_file: None,
        key_file: None,
    };

    assert_connected(&new_test_connection(&nats, Some(tls), None)).await;
}

#[tokio::test]
async fn test_connect_mutual_tls() {
    let nats = new_test_nats(TestNatsOpts {
        tls: TestTls::Mutual,
        ..Default::default()
    })
    .await;
    let certs = nats.certs.as_ref().expect("certs");
    let tls = Tls {
        ca_file: Some(certs.ca_file.clone()),
        cert_file: Some(certs.cert_file.clone()),
        key_file: Some(certs.key_file.clone()),
    };

    assert_connected(&new_test_connection(&nats, Some(tls), None)).await;
}

#[tokio::test]
async fn test_connect_token() {
    let dir = tempfile::tempdir().expect("temp dir");
    let nats = new_test_nats(TestNatsOpts {
        token: Some(TEST_TOKEN.to_string()),
        ..Default::default()
    })
    .await;
    let path = new_test_token_file(dir.path(), TEST_TOKEN);

    assert_connected(&new_test_connection(
        &nats,
        None,
        Some(Auth::TokenFile(path)),
    ))
    .await;
}

#[tokio::test]
async fn test_connect_wrong_token() {
    let dir = tempfile::tempdir().expect("temp dir");
    let nats = new_test_nats(TestNatsOpts {
        token: Some(TEST_TOKEN.to_string()),
        ..Default::default()
    })
    .await;
    let path = new_test_token_file(dir.path(), "wrong");
    let conn = new_test_connection(&nats, None, Some(Auth::TokenFile(path)));

    assert!(connect(&conn, "naps-test").await.is_err());
}
