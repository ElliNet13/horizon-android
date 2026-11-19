#!/usr/bin/env bash
# Rebase the Android overlay branch onto the latest upstream main.
#
#   tools/sync-upstream.sh            # fetch, rebase, report
#   UPSTREAM=upstream BRANCH=android tools/sync-upstream.sh
#
# The branch is a stack: upstream's still-open Android PR commits at the bottom, then this fork's own
# small commits. When upstream merges one of those PRs, git drops the now-identical commit during the
# rebase on its own; the script lists what was dropped. On a conflict it stops, as git does: fix, `git
# rebase --continue`, and run it again.
set -euo pipefail
# The remote that points at horizon-rust/horizon: found by URL, or added as "upstream".
UPSTREAM=${UPSTREAM:-$(git remote -v | awk '/horizon-rust\/horizon(\.git)? \(fetch\)/ {print $1; exit}')}
UPSTREAM=${UPSTREAM:-upstream}
BRANCH=${BRANCH:-android}
URL=https://github.com/horizon-rust/horizon.git

git remote get-url "$UPSTREAM" >/dev/null 2>&1 || git remote add "$UPSTREAM" "$URL"
git fetch "$UPSTREAM" main
git checkout "$BRANCH"
before=$(git rev-list --count "$UPSTREAM/main..$BRANCH")
git log --format='%h %s' "$UPSTREAM/main..$BRANCH" | sort -k2 > /tmp/horizon-sync-before.txt
git rebase "$UPSTREAM/main"
after=$(git rev-list --count "$UPSTREAM/main..$BRANCH")
echo "overlay: $before commits before, $after after ($((before - after)) now in upstream)"
echo "still on top of upstream:"
git log --format='  %h %s' "$UPSTREAM/main..$BRANCH"
echo "push with: git push --force-with-lease <your-fork-remote> $BRANCH"
echo "next: cargo check -p horizon-client --no-default-features && cargo check -p horizon-android"
