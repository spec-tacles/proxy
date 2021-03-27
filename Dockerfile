FROM rust:alpine AS build
WORKDIR /usr/src/proxy
RUN apk add openssl-dev musl-dev
COPY . .
RUN cargo install --path . --features redis-ratelimiter

FROM scratch
COPY --from=build /usr/local/cargo/bin/proxy /proxy
CMD ["/proxy"]
