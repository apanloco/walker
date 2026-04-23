#!/bin/bash
set -e

if ! command -v docker &>/dev/null; then
  echo "Error: docker is not installed or not in PATH" >&2
  exit 1
fi

docker rm -f walker-postgres 2>/dev/null || true

if ! docker run -d --name walker-postgres \
    -e POSTGRES_PASSWORD=walker \
    -e POSTGRES_DB=walker \
    -p 5432:5432 \
    postgres:16-alpine; then
  echo "Error: failed to start walker-postgres container" >&2
  exit 1
fi

echo "Waiting for Postgres to start..."
sleep 2
echo "Done. Start server with: cargo run -- listen --dev"
