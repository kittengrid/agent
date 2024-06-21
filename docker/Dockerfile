# syntax=docker.io/docker/dockerfile:1.3.1
FROM ubuntu:22.04 as base
RUN apt-get update -y && apt-get install -y wget curl gpg coreutils unzip \
    git ca-certificates lsb-core libssl-dev

RUN curl https://sh.rustup.rs -sSf | RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo bash -s -- -y --no-modify-path \
    && /usr/local/cargo/bin/rustup default stable
RUN mkdir /app
RUN apt-get update -y && apt-get install -y build-essential pkg-config

FROM base as dev
ARG APP_USER=kittengrid
ARG APP_ROOT=/app
RUN apt-get update -qq \
    && DEBIAN_FRONTEND=noninteractive apt-get -y dist-upgrade \
    && apt-get install -qy gnupg apt-transport-https ca-certificates curl \
    && echo 'deb [trusted=yes] https://kittengrid-repository-public.s3.eu-west-3.amazonaws.com/package-repository/ jammy main' > /etc/apt/sources.list.d/internal_repository.list \
    && apt-get update -qq \
    && DEBIAN_FRONTEND=noninteractive apt-get install -o Dpkg::Options::=--force-confdef -yq \
    --no-install-recommends jq curl dnsutils lsof ngrep wget ca-certificates iproute2 inetutils-ping vim iptables tcpdump
RUN useradd -m -o -u 999 ${APP_USER} && chown ${APP_USER} ${APP_ROOT} \
    && apt-get install -y sudo \
    && adduser ${APP_USER} sudo \
    && echo "${APP_USER} ALL=NOPASSWD: ALL" > /etc/sudoers.d/${APP_USER} \
    && chmod ugo+s /usr/sbin/usermod /usr/sbin/groupmod /usr/bin/cp \
    && rm /home/kittengrid/.bashrc /home/kittengrid/.profile \
    && chown -R kittengrid /home/kittengrid
COPY docker/rootfs/ /
USER kittengrid
WORKDIR ${APP_ROOT}

FROM base as build
ENV PATH="${PATH}:/usr/local/cargo/bin"
# For caching dependencies
RUN cargo new --bin kittengrid-agent \
    && echo "fn lib() {}" > /kittengrid-agent/src/dummy-lib.rs \
    && echo "fn main() {}" > /kittengrid-agent/src/dummy-main.rs \
    && mkdir /kittengrid-agent/src/log_generator && echo "fn main() {}" > /kittengrid-agent/src/log_generator/dummy-main.rs
WORKDIR /kittengrid-agent
COPY Cargo.toml Cargo.lock ./
RUN sed -i 's#src/main.rs#src/dummy-main.rs#' Cargo.toml
RUN sed -i 's#src/log_generator/main.rs#src/log_generator/dummy-main.rs#' Cargo.toml
RUN sed -i 's#src/lib.rs#src/dummy-lib.rs#' Cargo.toml
RUN /usr/local/cargo/bin/cargo build --release
RUN rm -Rf ./src/* ./target/release/deps/kittengrid-agent-* ./target/release/deps/lib-* ./target/release/kittengrid-agent
RUN sed -i 's#src/dummy-main.rs#src/main.rs#' Cargo.toml
RUN sed -i 's#src/dummy-lib.rs#src/lib.rs#' Cargo.toml
RUN sed -i 's#src/log_generator/dummy-main.rs#src/log_generator/main.rs#' Cargo.toml

COPY ./src ./src
RUN /usr/local/cargo/bin/cargo build --release

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
RUN groupadd -g 10001 kittengrid && \
    useradd -r -u 10001 -g kittengrid kittengrid
USER kittengrid
EXPOSE 8000
ENTRYPOINT ["/kittengrid-agent"]
CMD ["--bind-address", "0.0.0.0", "--bind-port", "8000"]
