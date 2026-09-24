#!/usr/bin/env bash
# Applies the repository settings that live outside git (issue #16): the rulesets in
# .github/rulesets/, secret scanning, Dependabot, CodeQL and the workflow policy for forks.
# Run by the owner with an admin `gh` login; safe to re-run.
set -euo pipefail
repo=${1:-Eolan/forge-engine}
root=$(cd "$(dirname "$0")/.." && pwd)

# Rulesets on main, created or updated by name.
for file in "$root"/.github/rulesets/*.json; do
  name=$(sed -n 's/^ *"name": "\(.*\)",$/\1/p' "$file" | head -n 1)
  id=$(gh api "repos/$repo/rulesets" -q ".[] | select(.name == \"$name\") | .id")
  if [ -n "$id" ]; then
    gh api -X PUT "repos/$repo/rulesets/$id" --input "$file" > /dev/null
  else
    gh api -X POST "repos/$repo/rulesets" --input "$file" > /dev/null
  fi
  echo "ruleset: $name"
done

# Secret scanning with push protection: a pushed credential is refused, not only reported.
gh api -X PATCH "repos/$repo" --input - > /dev/null <<'JSON'
{"security_and_analysis": {
  "secret_scanning": {"status": "enabled"},
  "secret_scanning_push_protection": {"status": "enabled"}}}
JSON
echo "secret scanning and push protection: on"

# Dependabot alerts and security-fix pull requests; private vulnerability reports (SECURITY.md).
gh api -X PUT "repos/$repo/vulnerability-alerts"
gh api -X PUT "repos/$repo/automated-security-fixes"
gh api -X PUT "repos/$repo/private-vulnerability-reporting"
echo "dependabot alerts, security fixes, private vulnerability reporting: on"

# CodeQL default setup (Rust and the workflows): pushes, pull requests and a weekly scan.
gh api -X PATCH "repos/$repo/code-scanning/default-setup" \
  -f state=configured -f query_suite=default > /dev/null
echo "codeql default setup: configured"

# Workflows on pull requests from forks wait for approval unless the author is a collaborator.
gh api -X PUT "repos/$repo/actions/permissions/fork-pr-contributor-approval" \
  -f approval_policy=all_external_contributors
# Pull requests merge by squash or rebase (linear history); their branches are deleted after.
gh api -X PATCH "repos/$repo" -F allow_merge_commit=false -F allow_squash_merge=true \
  -F allow_rebase_merge=true -F delete_branch_on_merge=true > /dev/null
echo "fork workflow approval and merge methods: set"
