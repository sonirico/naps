# NATS proxy service

Simple tool to forward specific topics from one nats.io cluster to another


# Example

```sh
cargo run --bin=naps -- --source nats://aws:4222 --destination nats://aks:4222 --topics "orders.>"
```

# TODO

- TLS conections
- JetStream 