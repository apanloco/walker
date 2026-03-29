#!/bin/bash
docker rm -f walker-postgres 2>/dev/null
docker run -d --name walker-postgres -e POSTGRES_PASSWORD=walker -e POSTGRES_DB=walker -p 5432:5432 postgres:16-alpine
echo "Waiting for Postgres to start..."
sleep 2
echo "Done. Start server with: DATABASE_URL=postgres://postgres:walker@localhost/walker cargo run -- listen --dev"
