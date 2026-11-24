FROM rust:1.100-bookworm

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    PYTHONDONTWRITEBYTECODE=1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential pkg-config ca-certificates \
        libx11-dev libxft-dev libfontconfig1-dev libfreetype6-dev libxtst-dev \
        xvfb xauth x11-utils x11-apps xkb-data \
        fonts-liberation fonts-dejavu-core fonts-droid-fallback \
        python3 procps \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Fetch crate dependencies into the image so test runs start quickly.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
 && printf 'fn main() {}\n' > src/main.rs \
 && cargo fetch --locked \
 && rm -rf src

# The repository is bind-mounted at /src by compose.yaml; this is the fallback
# entry point when the image is run directly.
CMD ["/src/tests/docker/run.sh"]
