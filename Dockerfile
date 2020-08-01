FROM rust:alpine AS build
WORKDIR /usr/src/proxy
RUN apk add openssl-dev musl-dev \
  && rustup default nightly
COPY . .
RUN cargo install --path ./bin

FROM scratch
COPY --from=build /usr/local/cargo/bin/spectacles-proxy-bin /proxy
CMD ["/proxy"]
