#!/usr/bin/env bash
# Stage, validate, and transactionally publish Omatainer's Omarchy integration.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/install-transaction.py" "$@"
