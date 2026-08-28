#!/usr/bin/env bash
# Symlink this repo's Claude Code command and agent definitions into ~/.claude, so
# the vendored copies stay the source of truth rather than drifting from your live
# setup. Any existing real file is backed up to <name>.bak first.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"

cd "$repo/.claude"
find commands agents -name '*.md' | while read -r rel; do
  target="$dest/$rel"
  mkdir -p "$(dirname "$target")"
  if [ -e "$target" ] && [ ! -L "$target" ]; then
    mv "$target" "$target.bak"
    echo "backed up  $rel -> $rel.bak"
  fi
  ln -sfn "$repo/.claude/$rel" "$target"
  echo "linked     $rel"
done

echo
echo "Done. Definitions now live in $repo/.claude and are symlinked into $dest."
