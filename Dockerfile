FROM rust:latest AS builder

RUN rustup target add x86_64-unknown-linux-musl
RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

# Create appuser
ENV USER=cloudmon
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

WORKDIR /cloudmon

# copy over your manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./src ./src

RUN cargo build --target x86_64-unknown-linux-musl --release

##############
## Final image
##############
FROM scratch as cloudmon-plugin-smtp

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /cloudmon

# Copy our build
COPY --from=builder /cloudmon/target/x86_64-unknown-linux-musl/release/cloudmon-plugin-smtp ./

# Use an unprivileged user.
USER cloudmon:cloudmon

ENV PATH=/cloudmon
CMD ["/cloudmon/cloudmon-plugin-smtp"]

#################
## Init container
#################
FROM alpine as cloudmon-plugin-smtp-init

WORKDIR /cloudmon

COPY ./grafana ./grafana

RUN mkdir /cloudmon/init && tar cvfz /cloudmon/init/grafana.tar.gz grafana
RUN rm -rf /cloudmon/grafana

# Use an unprivileged user.
USER cloudmon:cloudmon
CMD ["/bin/sh"]
