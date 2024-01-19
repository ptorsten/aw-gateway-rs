FROM --platform=$TARGETPLATFORM tonistiigi/xx AS xx

FROM --platform=$TARGETPLATFORM rust:alpine AS builder
COPY --from=xx / /

WORKDIR /app
COPY ./ .

RUN xx-apk add clang lld openssl-dev cmake

ARG TARGETPLATFORM
RUN xx-cargo build --target-dir ./build --release && \
    xx-verify --static ./build/$(xx-cargo --print-target-triple)/release/aw-gateway-rs

ARG TARGETPLATFORM
RUN mkdir -p /app/release

ARG TARGETPLATFORM
RUN cp ./build/$(xx-cargo --print-target-triple)/release/aw-gateway-rs /app/release/ 
    
# Create image
FROM rust:alpine
WORKDIR /app

ARG TARGETPLATFORM
COPY --from=builder /app/release/aw-gateway-rs ./

USER root

CMD ["./aw-gateway-rs"]
