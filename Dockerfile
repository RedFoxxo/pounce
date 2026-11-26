# pounce in a container: a static Rust build on Alpine. Directories such as
# Glama build this image, start it and read tools/list; pounce reaches
# Targetprocess only on the first tool call, so the placeholder TP_BASE_URL and
# TP_TOKEN below are enough for that. To use it, set your own:
#
#   docker run -i --rm -e TP_BASE_URL=https://yourcompany.tpondemand.com \
#     -e TP_TOKEN pounce
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM alpine:3
# rustls checks the instance's certificate against the system's CA store.
RUN apk add --no-cache ca-certificates
COPY --from=build /src/target/release/pounce /usr/local/bin/pounce
ENV TP_BASE_URL=https://example.invalid TP_TOKEN=placeholder
USER nobody
ENTRYPOINT ["pounce"]
