FROM rustlang/rust:nightly-bookworm AS build
RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked
WORKDIR /source
COPY . .
RUN dx build --platform web --release \
    && cargo build --release --no-default-features --features server

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 aio
WORKDIR /opt/aio
COPY --from=build /source/target/release/aio-public-shell /usr/local/bin/aio-application
COPY --from=build /source/target/dx/aio-public-shell/release/web/public /opt/aio/web
ENV AIO_WEB_PORT=8080
ENV AIO_WEB_DIST=/opt/aio/web
EXPOSE 8080
USER aio
ENTRYPOINT ["/usr/local/bin/aio-application"]
