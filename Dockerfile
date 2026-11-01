FROM rust:1-trixie AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/naps /naps
EXPOSE 8080
ENTRYPOINT ["/naps", "--config", "/etc/naps/naps.yaml"]
