use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use bytes::Bytes;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};
use rquickjs::{
    CatchResultExt, CaughtError, Context, Ctx, Exception, Function, Module, Object, Persistent,
    Runtime, TypedArray, Value, function::Opt,
};

use super::{Output, Processor};
use crate::msg::Msg;

const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

pub struct JsProcessor {
    recv: Persistent<Function<'static>>,
    context: Context,
    _runtime: Runtime,
}

impl JsProcessor {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let mut source = std::fs::read_to_string(path)
            .with_context(|| format!("read processor {}", path.display()))?;
        if path.extension().and_then(|ext| ext.to_str()) == Some("ts") {
            source = strip_types(path, &source)?;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let runtime = Runtime::new().context("create js runtime")?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        let context = Context::full(&runtime).context("create js context")?;
        let recv = context.with(|ctx| {
            register_text_codecs(&ctx)?;
            load_recv(&ctx, name, source)
        })?;

        Ok(Self {
            recv,
            context,
            _runtime: runtime,
        })
    }
}

impl Processor for JsProcessor {
    fn process(&mut self, msg: Msg) -> anyhow::Result<Vec<Output>> {
        let subject = msg.subject.clone();
        self.context.with(|ctx| {
            let recv = self.recv.clone().restore(&ctx).map_err(js_error)?;
            let input = msg_to_js(&ctx, &msg).catch(&ctx).map_err(caught_error)?;
            let ret: Value = recv.call((input,)).catch(&ctx).map_err(caught_error)?;
            outputs_from_js(&ret, &subject)
        })
    }
}

fn strip_types(path: &Path, source: &str) -> anyhow::Result<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(err) = parsed.diagnostics.first() {
        bail!("parse {}: {err}", path.display());
    }
    let mut program = parsed.program;

    let semantic = SemanticBuilder::new().build(&program);
    if let Some(err) = semantic.diagnostics.first() {
        bail!("analyze {}: {err}", path.display());
    }
    let scoping = semantic.semantic.into_scoping();

    let transformed = Transformer::new(&allocator, path, &TransformOptions::default())
        .build_with_scoping(scoping, &mut program);
    if let Some(err) = transformed.diagnostics.first() {
        bail!("transform {}: {err}", path.display());
    }

    Ok(Codegen::new().build(&program).code)
}

fn register_text_codecs(ctx: &Ctx<'_>) -> anyhow::Result<()> {
    let encoder = Function::new(ctx.clone(), new_text_encoder)
        .map_err(js_error)?
        .with_constructor(true);
    let decoder = Function::new(ctx.clone(), new_text_decoder)
        .map_err(js_error)?
        .with_constructor(true);
    let globals = ctx.globals();
    globals.set("TextEncoder", encoder).map_err(js_error)?;
    globals.set("TextDecoder", decoder).map_err(js_error)?;
    Ok(())
}

fn new_text_encoder<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let encoder = Object::new(ctx.clone())?;
    encoder.set("encoding", "utf-8")?;
    encoder.set(
        "encode",
        Function::new(ctx, |ctx: Ctx<'js>, text: String| {
            TypedArray::<u8>::new_copy(ctx, text.as_bytes())
        })?,
    )?;
    Ok(encoder)
}

fn new_text_decoder<'js>(ctx: Ctx<'js>, label: Opt<String>) -> rquickjs::Result<Object<'js>> {
    if let Some(label) = label.0
        && !label.eq_ignore_ascii_case("utf-8")
        && !label.eq_ignore_ascii_case("utf8")
    {
        return Err(Exception::throw_range(
            &ctx,
            &format!("unsupported encoding label: {label}"),
        ));
    }
    let decoder = Object::new(ctx.clone())?;
    decoder.set("encoding", "utf-8")?;
    decoder.set(
        "decode",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, data: TypedArray<'js, u8>| {
            // SAFETY: the slice is decoded into an owned string and no JS runs while it is alive.
            let bytes = unsafe { data.as_bytes() }
                .ok_or_else(|| Exception::throw_type(&ctx, "data buffer is detached"))?;
            Ok::<_, rquickjs::Error>(String::from_utf8_lossy(bytes).into_owned())
        })?,
    )?;
    Ok(decoder)
}

fn load_recv(
    ctx: &Ctx<'_>,
    name: String,
    source: String,
) -> anyhow::Result<Persistent<Function<'static>>> {
    let declared = Module::declare(ctx.clone(), name, source)
        .catch(ctx)
        .map_err(caught_error)?;
    let (module, promise) = declared.eval().catch(ctx).map_err(caught_error)?;
    promise.finish::<()>().catch(ctx).map_err(caught_error)?;

    let value: Value = module.get("recv").catch(ctx).map_err(caught_error)?;
    let Some(recv) = value.into_function() else {
        bail!("recv export missing");
    };
    Ok(Persistent::save(ctx, recv))
}

fn msg_to_js<'js>(ctx: &Ctx<'js>, msg: &Msg) -> rquickjs::Result<Object<'js>> {
    let headers = Object::new(ctx.clone())?;
    for (key, values) in &msg.headers {
        headers.set(key.as_str(), values.clone())?;
    }
    let input = Object::new(ctx.clone())?;
    input.set("subject", msg.subject.as_str())?;
    input.set("headers", headers)?;
    input.set("data", TypedArray::<u8>::new_copy(ctx.clone(), &msg.data)?)?;
    Ok(input)
}

fn outputs_from_js(ret: &Value<'_>, input_subject: &str) -> anyhow::Result<Vec<Output>> {
    if ret.is_null() || ret.is_undefined() {
        return Ok(vec![]);
    }
    if let Some(array) = ret.as_array() {
        return array
            .iter::<Value>()
            .map(|item| output_from_js(&item.map_err(js_error)?, input_subject))
            .collect();
    }
    Ok(vec![output_from_js(ret, input_subject)?])
}

fn output_from_js(value: &Value<'_>, input_subject: &str) -> anyhow::Result<Output> {
    let Some(object) = value.as_object() else {
        bail!("recv must return null, an object or an array of objects");
    };

    let subject_value: Value = object.get("subject").map_err(js_error)?;
    let subject = subject_value
        .as_string()
        .map(|subject| subject.to_string())
        .transpose()
        .map_err(js_error)?;
    let subject_set = subject.is_some();

    let headers_value: Value = object.get("headers").map_err(js_error)?;
    let mut headers = BTreeMap::new();
    if let Some(headers_object) = headers_value.as_object() {
        for entry in headers_object.props::<String, Vec<String>>() {
            let (key, values) = entry.map_err(js_error)?;
            headers.insert(key, values);
        }
    }

    let data_value: Value = object.get("data").map_err(js_error)?;
    let data = if let Some(text) = data_value.as_string() {
        Bytes::from(text.to_string().map_err(js_error)?.into_bytes())
    } else {
        let array = TypedArray::<u8>::from_value(data_value).map_err(js_error)?;
        // SAFETY: the slice is copied immediately and no JS runs while it is alive.
        let bytes =
            unsafe { array.as_bytes() }.ok_or_else(|| anyhow!("data buffer is detached"))?;
        Bytes::copy_from_slice(bytes)
    };

    Ok(Output {
        msg: Msg {
            subject: subject.unwrap_or_else(|| input_subject.to_owned()),
            headers,
            data,
        },
        subject_set,
    })
}

fn js_error(err: rquickjs::Error) -> anyhow::Error {
    anyhow!("js: {err}")
}

fn caught_error(err: CaughtError<'_>) -> anyhow::Error {
    match err {
        CaughtError::Exception(ex) => {
            let message = ex.message().unwrap_or_default();
            let described = match ex.get::<_, Option<String>>("name") {
                Ok(Some(name)) => format!("{name}: {message}"),
                _ => message,
            };
            anyhow!(
                "js exception: {described}\n{}",
                ex.stack().unwrap_or_default()
            )
        }
        CaughtError::Value(value) => anyhow!("js exception: {value:?}"),
        CaughtError::Error(err) => js_error(err),
    }
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
                name: "typescript passes the message through unchanged",
                fixture: "passthrough.ts",
                want: vec![Output {
                    msg: input.clone(),
                    subject_set: true,
                }],
            },
            TestCase {
                name: "null drops the message",
                fixture: "drop.js",
                want: vec![],
            },
            TestCase {
                name: "one output with a new subject",
                fixture: "new_subject.js",
                want: vec![new_test_output("orders.routed", INPUT_DATA, true)],
            },
            TestCase {
                name: "array of two outputs",
                fixture: "two_outputs.js",
                want: vec![
                    new_test_output("a", b"first", true),
                    new_test_output(INPUT_SUBJECT, INPUT_DATA, false),
                ],
            },
            TestCase {
                name: "string data is utf-8",
                fixture: "string_data.js",
                want: vec![new_test_output(
                    INPUT_SUBJECT,
                    "caf\u{e9}".as_bytes(),
                    false,
                )],
            },
            TestCase {
                name: "uint8array data",
                fixture: "bytes_data.js",
                want: vec![new_test_output(INPUT_SUBJECT, &[1, 2, 3], false)],
            },
            TestCase {
                name: "headers round-trip",
                fixture: "headers.js",
                want: vec![Output {
                    msg: Msg {
                        subject: INPUT_SUBJECT.to_owned(),
                        headers: with_extra_headers,
                        data: Bytes::from_static(INPUT_DATA),
                    },
                    subject_set: false,
                }],
            },
            TestCase {
                name: "text encoder yields utf-8 bytes",
                fixture: "text_encode.js",
                want: vec![new_test_output(INPUT_SUBJECT, &[0xC3, 0xB1], false)],
            },
        ];

        for tc in cases {
            let mut processor = JsProcessor::new(&fixture_path(tc.fixture)).expect(tc.name);

            let got = processor.process(input.clone()).expect(tc.name);

            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn process_text_decoder_decodes_input_data() {
        struct DecodeCase {
            name: &'static str,
            data: &'static [u8],
            want: &'static [u8],
        }
        let cases = [
            DecodeCase {
                name: "valid utf-8",
                data: b"hello",
                want: b"HELLO",
            },
            DecodeCase {
                name: "invalid sequence is replaced",
                data: &[0xFF],
                want: &[0xEF, 0xBF, 0xBD],
            },
        ];

        for tc in cases {
            let mut processor = JsProcessor::new(&fixture_path("text_decode.js")).expect(tc.name);
            let mut input = new_test_input();
            input.data = Bytes::from_static(tc.data);

            let got = processor.process(input).expect(tc.name);

            assert_eq!(
                got,
                vec![new_test_output(INPUT_SUBJECT, tc.want, false)],
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn process_text_decoder_rejects_unsupported_label() {
        let mut processor = JsProcessor::new(&fixture_path("text_decoder_latin1.js")).unwrap();

        let err = processor.process(new_test_input()).unwrap_err();

        assert!(
            err.to_string().contains("RangeError"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn process_returns_thrown_message_as_error() {
        let mut processor = JsProcessor::new(&fixture_path("throws.js")).unwrap();

        let err = processor.process(new_test_input()).unwrap_err();

        assert!(
            err.to_string().contains("Error: boom from script"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn new_fails_without_recv_export() {
        let err = JsProcessor::new(&fixture_path("no_recv.js"))
            .err()
            .expect("expected an error");

        assert!(
            err.to_string().contains("recv export missing"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn new_fails_on_import() {
        let result = JsProcessor::new(&fixture_path("with_import.js"));

        assert!(result.is_err());
    }
}
