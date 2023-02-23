FROM ubuntu:22.04

RUN apt-get update -y && apt-get install -y wget curl gpg coreutils unzip git ca-certificates lsb-core libssl-dev

RUN mkdir -m 0755 -p /etc/apt/keyrings
RUN curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
RUN echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null

RUN chmod a+r /etc/apt/keyrings/docker.gpg

# install docker
RUN apt-get update -y && apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

# Get Rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y

# download public key for github.com
RUN mkdir -p -m 0600 ~/.ssh && ssh-keyscan github.com >> ~/.ssh/known_hosts

COPY . /app

RUN apt-get update -y && apt-get install -y build-essential pkg-config

RUN --mount=type=ssh  cd /app && $HOME/.cargo/bin/cargo build --release

EXPOSE 8000

ENTRYPOINT /app/target/release/kittengrid-agent
