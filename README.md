# naps

naps relays messages between [NATS](https://nats.io) servers. It subscribes to subjects (or
pulls from a JetStream consumer) on a source server, optionally runs each message through a
processor written in TypeScript, JavaScript, Lua or WebAssembly, and publishes the results to a
destination server. A processor can rewrite a message, drop it, or fan it out into several
messages. naps is configured by one YAML file, exposes health and Prometheus endpoints, and
ships as a distroless container image with a Helm chart.

## Configuration

naps reads `/etc/naps/naps.yaml` by default; use `--config <path>` to change it. Unknown keys
are an error and every referenced file must exist at startup.

```yaml
http:
  listen: "0.0.0.0:8080"          # optional, this is the default
source:
  url: "nats://nats:4222"         # required, also accepts tls://
  tls:                            # optional; ca_file alone = server verification only
    ca_file: /etc/naps/tls/ca.crt
    cert_file: /etc/naps/tls/tls.crt   # cert_file and key_file go together or not at all
    key_file: /etc/naps/tls/tls.key
  auth:                           # optional, exactly one key
    creds_file: /etc/naps/creds/user.creds
    # nkey_file: /path            (file holds the nkey seed)
    # token_file: /path
destination:                      # same shape as source
  url: "nats://nats:4222"
routes:                           # at least one
  - name: orders                  # required, unique, [a-z0-9-]+, used as metric label
    from:                         # exactly one of subject | jetstream
      subject: "orders.>"
      queue_group: naps-orders    # optional, defaults to "naps-<name>"
      # jetstream: { stream: ORDERS, consumer: naps-orders }   # existing durable pull consumer
    processor: orders.ts          # optional; .ts .js .lua .wasm; relative to the config file dir
    to:
      mode: core                  # core | jetstream ; default core
      subject: "myapp.orders"     # optional
```

Notes:

- `from` takes exactly one of `subject` or `jetstream`. `queue_group` together with
  `jetstream` is a config error.
- Output subject precedence: the subject set by the processor, then `to.subject`, then the
  input subject.
- A route without a `processor` forwards each message unchanged.

See `examples/naps.yaml` for a complete file with three routes.

## Processors

A processor exposes one function, `recv`, that receives a message and returns nothing (drop),
one output, or a list of outputs (fan-out). The message has a `subject`, `headers` (a map of
header name to list of values) and `data` (bytes). An output has an optional `subject`, optional
`headers` (empty when missing) and `data`. Throwing or raising an error is a processing error.

### TypeScript and JavaScript

The module exports `function recv(msg)`. `msg.data` is a `Uint8Array`; an output `data` can be
a `Uint8Array` or a string (UTF-8). Types are stripped from `.ts` files; `import` is not
supported. The runtime provides the UTF-8 only globals `TextEncoder` and `TextDecoder`.

`examples/orders.ts` keeps only the confirmed orders and moves them to
`myapp.orders.confirmed`:

```typescript
interface Msg {
  subject: string;
  headers: Record<string, string[]>;
  data: Uint8Array;
}

export function recv(msg: Msg) {
  const data = new TextDecoder().decode(msg.data);
  const order = JSON.parse(data);
  if (order.status !== "confirmed") {
    return null;
  }
  return { subject: "myapp.orders.confirmed", data };
}
```

### Lua

Scripts run in a sandboxed Luau runtime. The global function `recv(msg)` has the same shape,
with `data` as a Lua string. It returns `nil`, an output table, or an array of output tables.

`examples/orders.lua` does the same without a JSON library:

```lua
function recv(msg)
  if not string.find(msg.data, '"status":"confirmed"', 1, true) then
    return nil
  end
  return { subject = "myapp.orders.confirmed", data = msg.data }
end
```

### WebAssembly

A WebAssembly processor is an [Extism](https://extism.org) plugin that exports `recv`. The
input is the JSON `{"subject", "headers", "data"}` with `data` in base64 (standard alphabet,
padded). The output is JSON `null`, an output object or an array of them, with `data` in
base64. The plugin has no network, no file system and 64 MiB of memory at most.

A plugin built with `extism-pdk` (target `wasm32-unknown-unknown`, crate type `cdylib`) that
renames the subject from a header:

```rust
use std::collections::BTreeMap;

use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Msg {
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    headers: BTreeMap<String, Vec<String>>,
    data: String,
}

#[plugin_fn]
pub fn recv(input: Vec<u8>) -> FnResult<Vec<u8>> {
    let mut msg: Msg = serde_json::from_slice(&input)?;
    msg.subject = msg.headers.get("x-rename").and_then(|v| v.first()).cloned();
    Ok(serde_json::to_vec(&msg)?)
}
```

## Delivery semantics

| Source | Event | Behavior |
|---|---|---|
| core subject | success | at-most-once; outputs are published |
| core subject | processing or publish error | message dropped, logged at `warn` with route name and error |
| JetStream | every output published (and acked by the server when `to.mode: jetstream`) | input is acked |
| JetStream | processing or publish error | input is nacked for redelivery |
| JetStream | zero outputs | input is acked (intentional drop) |

## Metrics and health

Port 8080 serves `/healthz`, `/readyz` and `/metrics`. All metrics are labelled `route`.

- `naps_messages_in_total`: every received input.
- `naps_messages_out_total`: every published output.
- `naps_messages_dropped_total`: inputs for which the processor returned zero outputs.
- `naps_errors_total{stage="process"|"publish"}`: errors by stage.
- `naps_process_duration_seconds`: processor duration histogram, only for routes with a
  processor.

## Install with Helm

The chart lives in `charts/naps`. The `config` value is rendered as `naps.yaml`; `scripts`
holds processor files by name and `binaryScripts` holds `.wasm` files as base64. Secrets are
mounted with `secretMounts` at the `mountPath` of each entry.

```sh
helm install naps charts/naps -f values.yaml
```

## Local development

```sh
just check         # fmt, clippy, tests, helm lint and template
just wasm-fixture  # rebuild tests/fixtures/scripts/recv.wasm
just image         # docker build -t naps:dev .
just e2e           # build the image and run the kind end-to-end test
```

The tests need a running docker daemon.

# Thanks

- Thanks to the rust community for such a good documentation and wide range of libraries which have made this journey
  far easier.
- Thanks to the [denoland](https://github.com/denoland/deno) community for pointing me into the right direction. Specially [Andreu Botella](https://github.com/andreubotella), denoland 
  contributor who patiently answered all my questions and guided me to a decent solution. Many thanks, man!
