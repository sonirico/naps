use crate::msg::Msg;
use crate::SimpleModuleLoader;
use crossbeam::channel::{select, Receiver, Sender};
use deno_core::anyhow::Error;
use deno_core::error::AnyError;
use deno_core::futures::{FutureExt, TryFutureExt};
use deno_core::RuntimeOptions;
use deno_core::{v8, FsModuleLoader};
use deno_core::{JsRuntime, ModuleSpecifier, NoopModuleLoader};
use deno_runtime::deno_broadcast_channel::InMemoryBroadcastChannel;
use deno_runtime::deno_web::BlobStore;
use deno_runtime::permissions::Permissions;
use deno_runtime::worker::MainWorker;
use deno_runtime::worker::WorkerOptions;
use deno_runtime::BootstrapOptions;
use serde_v8::Serializable;
use std::borrow::Borrow;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use std::{thread, time};

fn get_error_class_name(e: &AnyError) -> &'static str {
    deno_runtime::errors::get_error_class_name(e).unwrap_or("Error")
}

pub fn process_loop(
    script: String,
    process_rc: Receiver<Msg>,
    write_sc: Sender<Msg>,
    shutdown_arc: Arc<AtomicBool>,
) -> Result<(), AnyError> {
    let code = format!(
        r#"
            import {{ Buffer }} from 'http://deno.land/x/node_buffer/index.ts';

            {}; // User code

            if (recv && typeof recv === 'function') {{
                globalThis.recv = recv;
            }} else {{
                globalThis.recv = function recv(topic, uint8array) {{
                    return {{topic, msg: Buffer.from(uint8array).toString() }};
                }};
            }}
        "#,
        script
    );

    // TODO: Windows support
    // FIXME: A temporary named file is created to store the final JS code to be executed. This
    // code cannot be injected by memory to deno main module for some reason that still needs
    // to be investigated. What works it to send an existing file to `deno_core::resolve_path`.
    let micro_seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_micros();
    let path = format!("/tmp/{}.deno", micro_seconds.to_string());
    let path_str = path.as_str();
    // TODO: Check file does not exist
    match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path_str)
    {
        Err(e) => eprintln!("errors occurred when opening file: {}", e),
        Ok(mut fd) => {
            fd.write(code.as_bytes());
            fd.flush()?;
        }
    };

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let future = async move {
        let module_loader = Rc::new(SimpleModuleLoader);
        let create_web_worker_cb = Arc::new(|_| {
            todo!("Web workers are not supported ");
        });

        let options = WorkerOptions {
            bootstrap: BootstrapOptions {
                apply_source_maps: false,
                args: vec![],
                cpu_count: 1,
                debug_flag: false,
                enable_testing_features: false,
                location: None,
                no_color: false,
                runtime_version: "x".to_string(),
                ts_version: "x".to_string(),
                unstable: false,
            },
            extensions: vec![],
            unsafely_ignore_certificate_errors: None,
            root_cert_store: None,
            user_agent: "hello_runtime".to_string(),
            seed: None,
            js_error_create_fn: None,
            //web_worker_preload_module_cb,
            maybe_inspector_server: None,
            should_break_on_first_statement: false,
            module_loader,
            get_error_class_fn: Some(&get_error_class_name),
            origin_storage_dir: None,
            blob_store: BlobStore::default(),
            broadcast_channel: InMemoryBroadcastChannel::default(),
            shared_array_buffer_store: None,
            compiled_wasm_module_store: None,
            create_web_worker_cb,
        };

        let main_module = deno_core::resolve_path(path_str.as_ref())?;
        let permissions = Permissions::allow_all();

        let mut worker =
            MainWorker::bootstrap_from_options(main_module.clone(), permissions, options);

        worker.execute_main_module(&main_module).await?;
        //println!("worker module main executed");

        worker.run_event_loop(false).await?;
        //println!("worker event loop loaded");

        let runtime = &mut worker.js_runtime;

        let scope = &mut runtime.handle_scope();
        let context = scope.get_current_context();
        let global = context.global(scope);
        let scope = &mut v8::ContextScope::new(scope, context);

        let global_this_key = v8::String::new(scope, "globalThis").unwrap();
        let global_this = global.get(scope, v8::Local::from(global_this_key)).unwrap();
        let global_this_obj = v8::Local::<v8::Object>::try_from(global_this).unwrap();
        let global_this_recv_key = v8::String::new(scope, "recv").unwrap();
        let recv_func = global_this_obj
            .get(scope, v8::Local::from(global_this_recv_key))
            .unwrap();
        let recv_func_obj = v8::Local::<v8::Function>::try_from(recv_func).unwrap();
        let recv_func_scope = &mut v8::TryCatch::new(scope);
        let recv_ctx_this = v8::undefined(recv_func_scope).into();

        while !shutdown_arc.load(Ordering::Relaxed) {
            let msg = process_rc.recv();
            if msg.is_err() {
                eprintln!("{}", msg.unwrap_err());
                continue;
            }

            let msg = msg.unwrap();

            let recv_args_topic = msg.topic.to_v8(recv_func_scope).unwrap();
            let recv_args_data = msg.data.to_v8(recv_func_scope).unwrap();

            let value = recv_func_obj.call(
                recv_func_scope,
                recv_ctx_this,
                &[recv_args_topic, recv_args_data],
            );

            if let Some(exception) = recv_func_scope.exception() {
                eprint!(
                    "deno exception: { }",
                    exception.to_rust_string_lossy(recv_func_scope)
                );
                continue;
            }

            if value.is_none() {
                continue;
            }

            let value = value.unwrap();

            if value.is_boolean() && value.is_true() {
                // Just forward the message
                write_sc.send(msg);
            } else if value.is_object() {
                // Extract topic and msg
                let res = value.to_object(recv_func_scope).unwrap();
                let topic_key = v8::String::new(recv_func_scope, "topic").unwrap().into();
                let msg_key = v8::String::new(recv_func_scope, "msg").unwrap().into();
                let topic_val = res.get(recv_func_scope, topic_key).unwrap();
                let data_val = res.get(recv_func_scope, msg_key).unwrap();

                // TODO: data_val should support [u8]
                if topic_val.is_string() && data_val.is_string() {
                    let topic = topic_val
                        .to_string(recv_func_scope)
                        .unwrap()
                        .to_rust_string_lossy(recv_func_scope);
                    let data = data_val
                        .to_string(recv_func_scope)
                        .unwrap()
                        .to_rust_string_lossy(recv_func_scope);

                    let msg = Msg::from_str(data, topic);

                    // Send message to channel
                    write_sc.send(msg);
                }
            }
        }

        Ok(())
    };

    let res = tokio_runtime.block_on(future);

    std::fs::remove_file(path_str)?;

    eprintln!("process loop exited");

    res
}
