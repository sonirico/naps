pub mod args;
pub mod msg;
pub mod process;
pub mod read;
pub mod stats;
pub mod timer;
pub mod write;
use std::pin::Pin;

use data_url::DataUrl;
use deno_ast::{MediaType, ParseParams, SourceTextInfo};
use deno_core::anyhow::{anyhow, bail, Error};
use deno_core::futures::FutureExt;
use deno_core::resolve_import;
use deno_core::ModuleLoader;
use deno_core::ModuleSource;
use deno_core::ModuleSourceFuture;
use deno_core::ModuleSpecifier;
use deno_core::ModuleType;

pub struct SimpleModuleLoader;

impl ModuleLoader for SimpleModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _is_main: bool,
    ) -> Result<ModuleSpecifier, Error> {
        Ok(resolve_import(specifier, referrer)?)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<ModuleSpecifier>,
        _is_dyn_import: bool,
    ) -> Pin<Box<ModuleSourceFuture>> {
        let module_specifier = module_specifier.clone();
        let string_specifier = module_specifier.to_string();
        async move {
            let bytes = match module_specifier.scheme() {
                "http" | "https" => {
                    let res = reqwest::get(module_specifier).await?;
                    // TODO: The HTML spec says to fail if the status is not
                    // 200-299, but `error_for_status()` fails if the status is
                    // 400-599.
                    let res = res.error_for_status()?;
                    res.bytes().await?
                }
                "file" => {
                    let path = match module_specifier.to_file_path() {
                        Ok(path) => path,
                        Err(_) => bail!("Invalid file URL."),
                    };
                    let bytes = tokio::fs::read(path).await?;
                    bytes.into()
                }
                "data" => {
                    let url = match DataUrl::process(module_specifier.as_str()) {
                        Ok(url) => url,
                        Err(_) => bail!("Not a valid data URL."),
                    };
                    let bytes = match url.decode_to_vec() {
                        Ok((bytes, _)) => bytes,
                        Err(_) => bail!("Not a valid data URL."),
                    };
                    bytes.into()
                }
                schema => bail!("Invalid schema {}", schema),
            };

            // Strip BOM
            let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                bytes.slice(3..)
            } else {
                bytes
            };

            let parsed = deno_ast::parse_module(ParseParams {
                specifier: string_specifier.clone(),
                source: SourceTextInfo::from_string(String::from_utf8_lossy(&bytes).into_owned()),
                media_type: MediaType::TypeScript,
                capture_tokens: false,
                scope_analysis: false,
                maybe_syntax: None,
            })?;

            Ok(ModuleSource {
                code: parsed.transpile(&Default::default())?.text,
                // TODO: JSON modules and redirects.
                module_type: ModuleType::JavaScript,
                module_url_specified: string_specifier.clone(),
                module_url_found: string_specifier.clone(),
            })
        }
        .boxed_local()
    }
}
