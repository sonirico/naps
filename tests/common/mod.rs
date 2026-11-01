use std::path::PathBuf;

use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use tempfile::TempDir;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const NATS_PORT: u16 = 4222;
const NATS_CONF_PATH: &str = "/etc/nats/nats.conf";
const NATS_CERTS_DIR: &str = "/etc/nats/tls";

pub enum TestTls {
    None,
    ServerOnly,
    Mutual,
}

pub struct TestNatsOpts {
    pub tls: TestTls,
    pub token: Option<String>,
    pub jetstream: bool,
}

impl Default for TestNatsOpts {
    fn default() -> Self {
        Self {
            tls: TestTls::None,
            token: None,
            jetstream: false,
        }
    }
}

pub struct TestCerts {
    pub ca_file: PathBuf,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
}

pub struct TestNats {
    pub url: String,
    pub certs: Option<TestCerts>,
    _container: ContainerAsync<GenericImage>,
    _dir: TempDir,
}

struct PemPair {
    cert: String,
    key: String,
}

fn new_test_cert(
    names: Vec<String>,
    purpose: ExtendedKeyUsagePurpose,
    ca: &rcgen::Issuer<'_, KeyPair>,
) -> PemPair {
    let mut params = CertificateParams::new(names).expect("cert params");
    params.extended_key_usages = vec![purpose];
    let key = KeyPair::generate().expect("generate key");
    let cert = params.signed_by(&key, ca).expect("sign cert");
    PemPair {
        cert: cert.pem(),
        key: key.serialize_pem(),
    }
}

pub async fn new_test_nats(opts: TestNatsOpts) -> TestNats {
    let dir = TempDir::new().expect("create temp dir");
    let tls_on = !matches!(opts.tls, TestTls::None);
    let mutual = matches!(opts.tls, TestTls::Mutual);

    let mut conf = String::new();
    let mut image = GenericImage::new("nats", "2.15.2")
        .with_exposed_port(ContainerPort::Tcp(NATS_PORT))
        .with_wait_for(WaitFor::message_on_stderr("Server is ready"))
        .with_cmd(["-c", NATS_CONF_PATH]);
    let mut certs = None;

    if tls_on {
        let ca_key = KeyPair::generate().expect("generate ca key");
        let mut ca_params = CertificateParams::new(Vec::new()).expect("ca params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("self-sign ca");
        let ca = rcgen::Issuer::new(ca_params, ca_key);

        let server = new_test_cert(
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            ExtendedKeyUsagePurpose::ServerAuth,
            &ca,
        );
        let client = new_test_cert(
            vec!["naps-test-client".to_string()],
            ExtendedKeyUsagePurpose::ClientAuth,
            &ca,
        );

        let files = [
            ("ca.crt", ca_cert.pem()),
            ("server.crt", server.cert),
            ("server.key", server.key),
            ("client.crt", client.cert),
            ("client.key", client.key),
        ];
        for (file, content) in files {
            let path = dir.path().join(file);
            std::fs::write(&path, content).expect("write cert file");
            image = image.with_copy_to(format!("{NATS_CERTS_DIR}/{file}"), path);
        }

        conf.push_str(&format!(
            "tls {{\n  cert_file: \"{NATS_CERTS_DIR}/server.crt\"\n  key_file: \"{NATS_CERTS_DIR}/server.key\"\n  ca_file: \"{NATS_CERTS_DIR}/ca.crt\"\n  verify: {mutual}\n}}\n"
        ));

        certs = Some(TestCerts {
            ca_file: dir.path().join("ca.crt"),
            cert_file: dir.path().join("client.crt"),
            key_file: dir.path().join("client.key"),
        });
    }

    if let Some(token) = &opts.token {
        conf.push_str(&format!("authorization {{\n  token: \"{token}\"\n}}\n"));
    }

    if opts.jetstream {
        conf.push_str("jetstream {}\n");
    }

    let conf_path = dir.path().join("nats.conf");
    std::fs::write(&conf_path, conf).expect("write nats.conf");
    let container = image
        .with_copy_to(NATS_CONF_PATH, conf_path)
        .start()
        .await
        .expect("start nats container");

    let port = container
        .get_host_port_ipv4(NATS_PORT)
        .await
        .expect("nats host port");
    let scheme = if tls_on { "tls" } else { "nats" };

    TestNats {
        url: format!("{scheme}://127.0.0.1:{port}"),
        certs,
        _container: container,
        _dir: dir,
    }
}
