FROM busybox:1.36.1 as rename
WORKDIR /app
COPY target/aarch64-unknown-linux-gnu/release/alert-deleter alert-deleter-arm64
COPY target/x86_64-unknown-linux-gnu/release/alert-deleter alert-deleter-amd64

FROM gcr.io/distroless/cc-debian12:nonroot
ARG TARGETARCH
COPY --from=rename /app/alert-deleter-$TARGETARCH /app/alert-deleter
ENTRYPOINT [ "/app/alert-deleter" ]
