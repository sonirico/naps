use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use extism::{Manifest, Plugin, Wasm};
use serde_json::{Map, Value, json};

use super::{Output, Processor};
use crate::msg::Msg;

const MEMORY_MAX_PAGES: u32 = 1024;

pub struct WasmProcessor {
    plugin: Plugin,
}

impl WasmProcessor {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let manifest = Manifest::new([Wasm::file(path)]).with_memory_max(MEMORY_MAX_PAGES);
        let plugin = Plugin::new(&manifest, [], true)
            .map_err(|err| anyhow!("wasm: load {}: {err}", path.display()))?;
        if !plugin.function_exists("recv") {
            bail!("recv export missing");
        }

        Ok(Self { plugin })
    }
}

impl Processor for WasmProcessor {
    fn process(&mut self, msg: Msg) -> anyhow::Result<Vec<Output>> {
        let input = serde_json::to_vec(&msg_to_json(&msg)).context("wasm: encode input")?;
        let ret = self
            .plugin
            .call::<&[u8], &[u8]>("recv", &input)
            .map_err(|err| anyhow!("wasm: {err}"))?;
        let value: Value = serde_json::from_slice(ret).context("wasm: decode output")?;
        outputs_from_json(&value, &msg.subject)
    }
}

fn msg_to_json(msg: &Msg) -> Value {
    json!({
        "subject": msg.subject,
        "headers": msg.headers,
        "data": STANDARD.encode(&msg.data),
    })
}

fn outputs_from_json(ret: &Value, input_subject: &str) -> anyhow::Result<Vec<Output>> {
    match ret {
        Value::Null => Ok(vec![]),
        Value::Object(object) => Ok(vec![output_from_json(object, input_subject)?]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Object(object) => output_from_json(object, input_subject),
                _ => bail!("recv must return null, an object or an array of objects"),
            })
            .collect(),
        _ => bail!("recv must return null, an object or an array of objects"),
    }
}

fn output_from_json(object: &Map<String, Value>, input_subject: &str) -> anyhow::Result<Output> {
    let subject = object.get("subject").and_then(Value::as_str);
    let subject_set = subject.is_some();

    let headers = match object.get("headers") {
        Some(headers) => serde_json::from_value::<BTreeMap<String, Vec<String>>>(headers.clone())
            .context("wasm: headers must map names to string arrays")?,
        None => BTreeMap::new(),
    };

    let Some(Value::String(text)) = object.get("data") else {
        bail!("data must be a base64 string");
    };
    let data = STANDARD
        .decode(text)
        .context("wasm: data must be a base64 string")?;

    Ok(Output {
        msg: Msg {
            subject: subject.unwrap_or(input_subject).to_owned(),
            headers,
            data: Bytes::from(data),
        },
        subject_set,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT_SUBJECT: &str = "orders.new";
    const INPUT_DATA: &[u8] = b"payload";

    struct TestCase {
        name: &'static str,
        headers: BTreeMap<String, Vec<String>>,
        want: Option<Vec<Output>>,
    }

    fn fixture_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripts/recv.wasm")
    }

    fn new_test_input(headers: BTreeMap<String, Vec<String>>) -> Msg {
        Msg {
            subject: INPUT_SUBJECT.to_owned(),
            headers,
            data: Bytes::from_static(INPUT_DATA),
        }
    }

    fn new_test_output(
        subject: &str,
        headers: &BTreeMap<String, Vec<String>>,
        subject_set: bool,
    ) -> Output {
        Output {
            msg: Msg {
                subject: subject.to_owned(),
                headers: headers.clone(),
                data: Bytes::from_static(INPUT_DATA),
            },
            subject_set,
        }
    }

    fn new_test_headers(key: &str, value: &str) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(key.to_owned(), vec![value.to_owned()])])
    }

    #[test]
    fn process_maps_plugin_return_to_outputs() {
        let plain = new_test_headers("k", "v");
        let rename = new_test_headers("x-rename", "new.subject");
        let fanout = new_test_headers("x-fanout", "1");
        let fail = new_test_headers("x-fail", "1");
        let cases = vec![
            TestCase {
                name: "passes the message through without a subject",
                headers: plain.clone(),
                want: Some(vec![new_test_output(INPUT_SUBJECT, &plain, false)]),
            },
            TestCase {
                name: "one output with a new subject",
                headers: rename.clone(),
                want: Some(vec![new_test_output("new.subject", &rename, true)]),
            },
            TestCase {
                name: "array of two outputs",
                headers: fanout.clone(),
                want: Some(vec![
                    new_test_output(INPUT_SUBJECT, &fanout, false),
                    new_test_output(INPUT_SUBJECT, &fanout, false),
                ]),
            },
            TestCase {
                name: "plugin error",
                headers: fail,
                want: None,
            },
        ];

        for tc in cases {
            let mut processor = WasmProcessor::new(&fixture_path()).expect(tc.name);

            let got = processor.process(new_test_input(tc.headers));

            match tc.want {
                Some(want) => assert_eq!(got.expect(tc.name), want, "{}", tc.name),
                None => assert!(got.is_err(), "{} should fail", tc.name),
            }
        }
    }

    #[test]
    fn new_fails_without_recv_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_recv.wasm");
        let wasm = wat::parse_str("(module (memory (export \"memory\") 1))").unwrap();
        std::fs::write(&path, wasm).unwrap();

        let err = WasmProcessor::new(&path).err().expect("expected an error");

        assert!(
            err.to_string().contains("recv export missing"),
            "unexpected error: {err}"
        );
    }
}
