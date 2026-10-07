#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

go build -trimpath -ldflags='-s -w' -o weather-scraper ./cmd/weather-scraper

# NixOS/Nix development shells expose GUI libraries through LD_LIBRARY_PATH.
# Keep this wrapper useful both from nix develop and on ordinary Linux hosts.
exec cargo run --release
