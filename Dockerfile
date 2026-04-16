# Этап сборки: берем nightly, так как зависимости требуют свежайший rustc
FROM rustlang/rust:nightly-bookworm as builder

WORKDIR /usr/src/app

# 1. Сначала копируем только манифесты для кэширования слоев
COPY Cargo.toml Cargo.lock ./

# 2. Собираем пустые зависимости (это сэкономит уйму времени при правке кода)
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/learn_rust*

# 3. Копируем реальный код и собираем бинарник
COPY . .
RUN cargo build --release

# Этап запуска: остается максимально легким
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# Копируем бинарник из builder
COPY --from=builder /usr/src/app/target/release/learn-rust ./app

EXPOSE 3719
CMD ["./app"]