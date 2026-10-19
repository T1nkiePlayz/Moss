#!/bin/sh
# Run Moss with the settings in .env (public Supabase project) and .env.local (your overrides).
set -e
cd "$(dirname "$0")/.."
exec cargo run --release -- serve
