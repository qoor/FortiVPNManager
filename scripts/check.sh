#!/bin/sh
set -eu -o pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
git ls-files -z --cached --others --exclude-standard '*.swift' | xargs -0 xcrun swift-format lint --strict
