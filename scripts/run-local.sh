#!/bin/sh
# Run Moss against your Supabase project using .env.local (gitignored).
set -e
cd "$(dirname "$0")/.."
set -a; . ./.env.local; set +a
exec cargo run --release -- serve
