use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

const PROCESSOR_EXTENSIONS: [&str; 4] = ["ts", "js", "lua", "wasm"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub http: Http,
    pub source: Connection,
    pub destination: Connection,
    pub routes: Vec<Route>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Http {
    pub listen: SocketAddr,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 8080)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub url: String,
    pub tls: Option<Tls>,
    pub auth: Option<Auth>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub ca_file: Option<PathBuf>,
    pub cert_file: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum Auth {
    CredsFile(PathBuf),
    NkeyFile(PathBuf),
    TokenFile(PathBuf),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub name: String,
    pub from: From,
    pub processor: Option<PathBuf>,
    #[serde(default)]
    pub to: To,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawFrom")]
pub enum From {
    Subject {
        subject: String,
        queue_group: String,
    },
    Jetstream {
        stream: String,
        consumer: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJetstream {
    stream: String,
    consumer: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFrom {
    subject: Option<String>,
    #[serde(default)]
    queue_group: String,
    jetstream: Option<RawJetstream>,
}

impl TryFrom<RawFrom> for From {
    type Error = String;

    fn try_from(raw: RawFrom) -> Result<Self, Self::Error> {
        match (raw.subject, raw.jetstream) {
            (Some(subject), None) => Ok(From::Subject {
                subject,
                queue_group: raw.queue_group,
            }),
            (None, Some(js)) if raw.queue_group.is_empty() => Ok(From::Jetstream {
                stream: js.stream,
                consumer: js.consumer,
            }),
            (None, Some(_)) => Err("queue_group is only valid with subject".to_string()),
            _ => Err("from needs exactly one of subject or jetstream".to_string()),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct To {
    pub mode: Mode,
    pub subject: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Core,
    Jetstream,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let mut config: Config = serde_norway::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;

        let config_dir = path.parent().unwrap_or(Path::new(""));
        for route in &mut config.routes {
            if let Some(processor) = &route.processor {
                route.processor = Some(config_dir.join(processor));
            }
            if let From::Subject { queue_group, .. } = &mut route.from
                && queue_group.is_empty()
            {
                *queue_group = format!("naps-{}", route.name);
            }
        }

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.routes.is_empty() {
            bail!("routes: at least one route is required");
        }

        let mut seen: Vec<&str> = Vec::new();
        for route in &self.routes {
            if seen.contains(&route.name.as_str()) {
                bail!("route {:?}: duplicate route name", route.name);
            }
            seen.push(&route.name);
        }

        for route in &self.routes {
            let valid = !route.name.is_empty()
                && route
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid {
                bail!("route {:?}: name must match [a-z0-9-]+", route.name);
            }
        }

        for route in &self.routes {
            let Some(processor) = &route.processor else {
                continue;
            };
            let extension = processor.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !PROCESSOR_EXTENSIONS.contains(&extension) {
                bail!(
                    "route {:?}: unsupported processor extension: {extension:?}",
                    route.name
                );
            }
            if !processor.is_file() {
                bail!(
                    "route {:?}: processor file {} does not exist",
                    route.name,
                    processor.display()
                );
            }
        }

        self.source.validate("source")?;
        self.destination.validate("destination")?;
        Ok(())
    }
}

impl Connection {
    fn validate(&self, field: &str) -> anyhow::Result<()> {
        if let Some(tls) = &self.tls {
            require_file(field, "tls.ca_file", &tls.ca_file)?;
            require_file(field, "tls.cert_file", &tls.cert_file)?;
            require_file(field, "tls.key_file", &tls.key_file)?;
            if tls.cert_file.is_some() != tls.key_file.is_some() {
                bail!("{field}.tls: cert_file and key_file must be set together");
            }
        }
        match &self.auth {
            Some(Auth::CredsFile(path)) => {
                require_file(field, "auth.creds_file", &Some(path.clone()))?
            }
            Some(Auth::NkeyFile(path)) => {
                require_file(field, "auth.nkey_file", &Some(path.clone()))?
            }
            Some(Auth::TokenFile(path)) => {
                require_file(field, "auth.token_file", &Some(path.clone()))?
            }
            None => {}
        }
        Ok(())
    }
}

fn require_file(field: &str, key: &str, path: &Option<PathBuf>) -> anyhow::Result<()> {
    if let Some(path) = path
        && !path.is_file()
    {
        bail!("{field}.{key}: file {} does not exist", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = "tests/fixtures/config";

    struct TestCase {
        name: &'static str,
        path: &'static str,
        want_err: Option<&'static str>,
        check: fn(&Config),
    }

    fn check_none(_: &Config) {}

    fn check_default_queue_group(config: &Config) {
        let From::Subject { queue_group, .. } = &config.routes[0].from else {
            panic!("expected subject source");
        };
        assert_eq!(queue_group, "naps-orders");
    }

    fn check_default_listen(config: &Config) {
        assert_eq!(config.http.listen.to_string(), "0.0.0.0:8080");
    }

    fn check_relative_processor(config: &Config) {
        assert_eq!(
            config.routes[0].processor,
            Some(Path::new(FIXTURES).join("noop.ts"))
        );
    }

    fn check_default_to(config: &Config) {
        assert_eq!(config.routes[0].to.mode, Mode::Core);
        assert_eq!(config.routes[0].to.subject, None);
    }

    #[test]
    fn load() {
        let cases = [
            TestCase {
                name: "valid example",
                path: "examples/naps.yaml",
                want_err: None,
                check: check_none,
            },
            TestCase {
                name: "unknown key",
                path: "unknown_key.yaml",
                want_err: Some("bogus"),
                check: check_none,
            },
            TestCase {
                name: "duplicate route name",
                path: "duplicate_route.yaml",
                want_err: Some("duplicate route name"),
                check: check_none,
            },
            TestCase {
                name: "bad route name",
                path: "bad_name.yaml",
                want_err: Some("[a-z0-9-]+"),
                check: check_none,
            },
            TestCase {
                name: "unknown processor extension",
                path: "bad_extension.yaml",
                want_err: Some("unsupported processor extension"),
                check: check_none,
            },
            TestCase {
                name: "missing processor file",
                path: "missing_processor.yaml",
                want_err: Some("does not exist"),
                check: check_none,
            },
            TestCase {
                name: "cert without key",
                path: "cert_without_key.yaml",
                want_err: Some("set together"),
                check: check_none,
            },
            TestCase {
                name: "zero routes",
                path: "no_routes.yaml",
                want_err: Some("at least one route"),
                check: check_none,
            },
            TestCase {
                name: "default queue group",
                path: "examples/naps.yaml",
                want_err: None,
                check: check_default_queue_group,
            },
            TestCase {
                name: "default http listen",
                path: "examples/naps.yaml",
                want_err: None,
                check: check_default_listen,
            },
            TestCase {
                name: "relative processor",
                path: "relative_processor.yaml",
                want_err: None,
                check: check_relative_processor,
            },
            TestCase {
                name: "omitted to",
                path: "relative_processor.yaml",
                want_err: None,
                check: check_default_to,
            },
        ];

        for tc in cases {
            let path = if tc.path.starts_with("examples/") {
                PathBuf::from(tc.path)
            } else {
                Path::new(FIXTURES).join(tc.path)
            };

            let result = Config::load(&path);

            match (result, tc.want_err) {
                (Ok(config), None) => (tc.check)(&config),
                (Err(err), Some(want)) => {
                    let msg = format!("{err:#}");
                    assert!(msg.contains(want), "{}: {msg:?} lacks {want:?}", tc.name);
                }
                (Ok(_), Some(want)) => panic!("{}: expected error containing {want:?}", tc.name),
                (Err(err), None) => panic!("{}: unexpected error {err:#}", tc.name),
            }
        }
    }
}
