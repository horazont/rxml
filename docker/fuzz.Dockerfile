FROM debian:testing

RUN set -eu; \
        apt-get update; \
        DEBIAN_FRONTEND=noninteractive apt-get install -y rustc cargo; \
        useradd -m -u 2000 fuzzer;

RUN set -eu; \
        DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential

RUN set -eu; \
        su -c 'cargo install afl' - fuzzer; \
        mkdir /app

RUN set -eu; \
        DEBIAN_FRONTEND=noninteractive apt-get install -y util-linux

ADD Cargo.toml /app/
ADD fuzz /app/fuzz
ADD src /app/src

RUN chown -R fuzzer:fuzzer /app

RUN set -eu; \
    su -c 'export PATH="/home/fuzzer/.cargo/bin:$PATH" && cd /app && cargo build' - fuzzer; \
    su -c 'export PATH="/home/fuzzer/.cargo/bin:$PATH" && cd /app/fuzz && cargo afl build' - fuzzer

ADD docker/fuzz-entrypoint.sh /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
