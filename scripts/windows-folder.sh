#!/bin/bash
set -euo pipefail
git_dir=$(git rev-parse --path-format=absolute --git-dir)
if [ "$git_dir" = "$(git rev-parse --path-format=absolute --git-common-dir)" ]; then
    echo main
else
    name=$(basename "$git_dir")
    [ "$name" != main ] || { echo "error: the worktree named main would share the main checkout's folder" >&2; exit 1; }
    echo "$name"
fi
