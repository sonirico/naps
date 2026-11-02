mod js;
mod lua;
mod wasm;

use std::path::Path;

use anyhow::bail;

use crate::msg::Msg;

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub msg: Msg,
    pub subject_set: bool,
}

pub trait Processor {
    fn process(&mut self, msg: Msg) -> anyhow::Result<Vec<Output>>;
}

pub fn new_processor(path: &Path) -> anyhow::Result<Box<dyn Processor>> {
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match ext {
        "ts" | "js" => Ok(Box::new(js::JsProcessor::new(path)?)),
        "lua" => Ok(Box::new(lua::LuaProcessor::new(path)?)),
        "wasm" => Ok(Box::new(wasm::WasmProcessor::new(path)?)),
        _ => bail!("unsupported processor extension: {ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_processor_rejects_unsupported_extension() {
        let path = Path::new("script.py");

        let err = new_processor(path).err().expect("expected an error");

        assert!(
            err.to_string()
                .contains("unsupported processor extension: py"),
            "unexpected error: {err}"
        );
    }
}
