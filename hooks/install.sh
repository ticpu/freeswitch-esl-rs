#!/bin/bash
# Install this repo's git hooks as symlinks in .git/hooks.
#
# Not core.hooksPath: it replaces .git/hooks outright, which also disables any
# global hooks dispatcher installed there.

set -e

HOOKS="pre-commit pre-push"

cd "$(git rev-parse --show-toplevel)"
hooks_dir="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"

if git config --local --get core.hooksPath >/dev/null; then
    git config --local --unset core.hooksPath
    echo "Removed local core.hooksPath override"
fi

for hook in $HOOKS; do
    ln -sfn "../../hooks/$hook" "$hooks_dir/$hook"
    echo "Linked $hook"
done
