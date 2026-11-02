use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use bytes::Bytes;
use mlua::{Function, Lua, Table, Value};

use super::{Output, Processor};
use crate::msg::Msg;

pub struct LuaProcessor {
    recv: Function,
    _lua: Lua,
}

impl LuaProcessor {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let source =
            std::fs::read(path).with_context(|| format!("read processor {}", path.display()))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let lua = Lua::new();
        lua.sandbox(true).map_err(lua_error)?;
        lua.load(source).set_name(name).exec().map_err(lua_error)?;
        let Ok(recv) = lua.globals().get::<Function>("recv") else {
            bail!("recv function missing");
        };

        Ok(Self { recv, _lua: lua })
    }
}

impl Processor for LuaProcessor {
    fn process(&mut self, msg: Msg) -> anyhow::Result<Vec<Output>> {
        let input = msg_to_lua(&self._lua, &msg).map_err(lua_error)?;
        let ret: Value = self.recv.call(input).map_err(lua_error)?;
        outputs_from_lua(&ret, &msg.subject)
    }
}

fn msg_to_lua(lua: &Lua, msg: &Msg) -> mlua::Result<Table> {
    let headers = lua.create_table()?;
    for (key, values) in &msg.headers {
        headers.set(key.as_str(), values.clone())?;
    }
    let input = lua.create_table()?;
    input.set("subject", msg.subject.as_str())?;
    input.set("headers", headers)?;
    input.set("data", lua.create_string(&msg.data)?)?;
    Ok(input)
}

fn outputs_from_lua(ret: &Value, input_subject: &str) -> anyhow::Result<Vec<Output>> {
    let table = match ret {
        Value::Nil => return Ok(vec![]),
        Value::Table(table) => table,
        _ => bail!("recv must return nil, a table or an array of tables"),
    };
    if table.contains_key("data").map_err(lua_error)? {
        return Ok(vec![output_from_lua(table, input_subject)?]);
    }
    table
        .sequence_values::<Value>()
        .map(|item| match item.map_err(lua_error)? {
            Value::Table(item) => output_from_lua(&item, input_subject),
            _ => bail!("recv must return nil, a table or an array of tables"),
        })
        .collect()
}

fn output_from_lua(table: &Table, input_subject: &str) -> anyhow::Result<Output> {
    let subject_value: Value = table.get("subject").map_err(lua_error)?;
    let subject = match subject_value {
        Value::String(subject) => Some(subject.to_str().map_err(lua_error)?.to_owned()),
        _ => None,
    };
    let subject_set = subject.is_some();

    let headers_value: Value = table.get("headers").map_err(lua_error)?;
    let mut headers = BTreeMap::new();
    if let Value::Table(headers_table) = headers_value {
        for entry in headers_table.pairs::<String, Vec<String>>() {
            let (key, values) = entry.map_err(lua_error)?;
            headers.insert(key, values);
        }
    }

    let data_value: Value = table.get("data").map_err(lua_error)?;
    let Value::String(text) = data_value else {
        bail!("data must be a string");
    };
    let data = Bytes::copy_from_slice(&text.as_bytes());

    Ok(Output {
        msg: Msg {
            subject: subject.unwrap_or_else(|| input_subject.to_owned()),
            headers,
            data,
        },
        subject_set,
    })
}

fn lua_error(err: mlua::Error) -> anyhow::Error {
    anyhow!("lua: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT_SUBJECT: &str = "orders.new";
    const INPUT_DATA: &[u8] = b"payload";

    struct TestCase {
        name: &'static str,
        fixture: &'static str,
        want: Vec<Output>,
    }

    fn fixture_path(fixture: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/scripts")
            .join(fixture)
    }

    fn new_test_input() -> Msg {
        Msg {
            subject: INPUT_SUBJECT.to_owned(),
            headers: BTreeMap::from([("k".to_owned(), vec!["v1".to_owned(), "v2".to_owned()])]),
            data: Bytes::from_static(INPUT_DATA),
        }
    }

    fn new_test_output(subject: &str, data: &'static [u8], subject_set: bool) -> Output {
        Output {
            msg: Msg {
                subject: subject.to_owned(),
                headers: BTreeMap::new(),
                data: Bytes::from_static(data),
            },
            subject_set,
        }
    }

    #[test]
    fn process_maps_script_return_to_outputs() {
        let input = new_test_input();
        let mut with_extra_headers = input.headers.clone();
        with_extra_headers.insert("x-extra".to_owned(), vec!["1".to_owned(), "2".to_owned()]);
        let cases = vec![
            TestCase {
                name: "passes the message through unchanged",
                fixture: "passthrough.lua",
                want: vec![Output {
                    msg: input.clone(),
                    subject_set: true,
                }],
            },
            TestCase {
                name: "nil drops the message",
                fixture: "drop.lua",
                want: vec![],
            },
            TestCase {
                name: "one output with a new subject",
                fixture: "new_subject.lua",
                want: vec![new_test_output("orders.routed", INPUT_DATA, true)],
            },
            TestCase {
                name: "array of two outputs",
                fixture: "two_outputs.lua",
                want: vec![
                    new_test_output("a", b"first", true),
                    new_test_output(INPUT_SUBJECT, INPUT_DATA, false),
                ],
            },
            TestCase {
                name: "binary data with a zero byte round-trips",
                fixture: "binary_data.lua",
                want: vec![new_test_output(INPUT_SUBJECT, b"a\0b", false)],
            },
            TestCase {
                name: "headers round-trip",
                fixture: "headers.lua",
                want: vec![Output {
                    msg: Msg {
                        subject: INPUT_SUBJECT.to_owned(),
                        headers: with_extra_headers,
                        data: Bytes::from_static(INPUT_DATA),
                    },
                    subject_set: false,
                }],
            },
        ];

        for tc in cases {
            let mut processor = LuaProcessor::new(&fixture_path(tc.fixture)).expect(tc.name);

            let got = processor.process(input.clone()).expect(tc.name);

            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn process_returns_script_error_message() {
        let mut processor = LuaProcessor::new(&fixture_path("throws.lua")).unwrap();

        let err = processor.process(new_test_input()).unwrap_err();

        assert!(err.to_string().contains("boom"), "unexpected error: {err}");
    }

    #[test]
    fn new_fails_without_recv_function() {
        let err = LuaProcessor::new(&fixture_path("no_recv.lua"))
            .err()
            .expect("expected an error");

        assert!(
            err.to_string().contains("recv function missing"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn process_fails_when_script_uses_forbidden_library() {
        for fixture in ["uses_io.lua", "uses_os_execute.lua"] {
            let mut processor = LuaProcessor::new(&fixture_path(fixture)).expect(fixture);

            let result = processor.process(new_test_input());

            assert!(result.is_err(), "{fixture} should fail");
        }
    }

    #[test]
    fn process_fails_when_script_mutates_shared_library() {
        let mut processor = LuaProcessor::new(&fixture_path("mutates_string_lib.lua")).unwrap();

        let result = processor.process(new_test_input());

        assert!(result.is_err());
    }
}
