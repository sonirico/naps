# NATS proxy service

Simple tool to forward specific topics from one [nats.io](https://github.com/nats-io/nats-server) cluster to the same server or 
another. Provides support to process messages with [deno](https://github.com/denoland/deno) Javascript or
TypeScript code.

### Example

Imagine that we use nats.io to relay events for every confirmed or
canceled order in our shopping platform:

```sh
./naps --source nats://aws:4222 --destination nats://aks:4222 --topics "orders.>"
```

### Processing Example

If the `--script` flag is present, `naps` will spawn a `Deno` runtime with all v8
capabilities plus promises and all event loop goodies, allowing you to, for example, only
keep the _confirmed_ ones and  relay them to the `myapp.orders.confirmed`. You only have to code a `recv` function
with the following signature:

```typescript
interface RecvResult {
    topic: string,
    msg: string
};

function recv(topic: string, data: Uint8Array): boolean | RecvResult {
    //... your code here...
}
```

- If the function returns `true`, the message will be simply forwarded to the same topic. **Do note** that the message
  will end up twice in the topic
- If the function returns `false`, this message will be discarded
- Finally, when `RecvResult` is returned, that data will be sent over the nats wire.

Example command:

```sh
./naps --source nats://aws:4222 --destination nats://aks:4222 --topics "myapp.v1.orders" --script "
    import { Buffer } from 'http://deno.land/x/node_buffer/index.ts';
    
    interface Order {
        status: 'confirmed' | 'canceled',
        user: string,
        amount: number,
        item: any
    };
    
    function processOrder(data: Buffer): RecvResult {
        const orderRaw = data.toString();
        const order = JSON.parse(orderRaw) as Order;
        
        // Skip orders that are not confirmed
        if (order.status !== 'confirmed') {
            return false;
        }
        return {
            topic: 'myapp.v1.orders.confirmed',
            msg: orderRaw
        };
    }

    function recv(topic, uint8array) {
        switch (topic) {
            case "myapp.v1.orders":
                return processOrder(Buffer.from(uint8array))
            default:
                // nothing to do...
        }
    }
"
```

# Thanks

- Thanks to the rust community for such a good documentation and wide range of libraries which have made this journey
  far easier.
- Thanks to the [denoland](https://github.com/denoland/deno) community for pointing me into the right direction. Specially [Andreu Botella](https://github.com/andreubotella), denoland 
  contributor who patiently answered all my questions and guided me to a decent solution. Many thanks, man!

# TODOs

- Support for NATS TLS connections
- JetStream 
- Feature Sagas by allowing to return multiple `RecvResult` when employing _deno_ runtime
