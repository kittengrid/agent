# syntax=docker.io/docker/dockerfile:1.3.1

FROM ubuntu:22.04 as build
RUN apt-get update -y && apt-get install -y wget curl gpg coreutils unzip \
    git ca-certificates lsb-core libssl-dev

RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
RUN mkdir /app
RUN apt-get update -y && apt-get install -y build-essential pkg-config

# For caching dependencies
RUN /root/.cargo/bin/cargo new --bin kittengrid-agent \
    && echo "fn lib() {}" > /kittengrid-agent/src/dummy-lib.rs \
    && echo "fn main() {}" > /kittengrid-agent/src/dummy-main.rs
WORKDIR /kittengrid-agent
COPY compose_rust ./compose_rust/
COPY Cargo.toml Cargo.lock ./
RUN sed -i 's#src/main.rs#src/dummy-main.rs#' Cargo.toml
RUN sed -i 's#src/lib.rs#src/dummy-lib.rs#' Cargo.toml
RUN /root/.cargo/bin/cargo build --release
RUN rm -Rf ./src/* ./target/release/deps/kittengrid-agent-* ./target/release/deps/lib-* ./target/release/kittengrid-agent
RUN sed -i 's#src/dummy-main.rs#src/main.rs#' Cargo.toml
RUN sed -i 's#src/dummy-lib.rs#src/lib.rs#' Cargo.toml

COPY ./src ./src
RUN /root/.cargo/bin/cargo build --release

FROM ubuntu:22.04 as release

RUN mkdir -m 0755 -p /etc/apt/keyrings
# install docker
RUN apt-get update -y \
    && apt-get install -y wget curl gpg coreutils unzip \
    git ca-certificates lsb-core libssl-dev

RUN curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
RUN echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
RUN chmod a+r /etc/apt/keyrings/docker.gpg

RUN apt-get update -y \
    && apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin

COPY --from=build /kittengrid-agent/target/release/kittengrid-agent /kittengrid-agent

# KUMA
ARG kuma_version=2.3.0
RUN curl -L https://download.konghq.com/kuma-binaries-release/kuma-${kuma_version}-linux-amd64.tar.gz | tar -zx -C /opt/ \
    && mv /opt/kuma-${kuma_version} /opt/kuma

COPY docker/entrypoint.sh /
RUN groupadd -g 10001 kittengrid \
    && useradd -r -u 10001 -g kittengrid kittengrid \
    && mkdir /home/kittengrid \
    && chown kittengrid:kittengrid /home/kittengrid

EXPOSE 65534
ENTRYPOINT ["/entrypoint.sh"]

