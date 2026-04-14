#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# verify-contracts.sh — Audit every frontend invoke() call against backend
# command registrations in Swift (PokeclawPlugin.swift) and Rust (lib.rs /
# commands.rs).
#
# Usage:  bash tauri-plugin-pokeclaw/scripts/verify-contracts.sh
#
# Exit 0 if all commands resolve; exit 1 if any FAIL.
# ---------------------------------------------------------------------------

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
PASS_COUNT=0
FAIL_COUNT=0

# -- colours (disabled when stdout is not a terminal) -----------------------
if [ -t 1 ]; then
  GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
else
  GREEN=''; RED=''; NC=''
fi

# -- extract all invoke() command names from frontend composables -----------
# We use a temp file to avoid pipe+subshell issues with set -e.

COMPOSABLES="
src/composables/useAccessibility.ts
src/composables/useChat.ts
src/composables/useModel.ts
src/composables/useTask.ts
src/composables/useAgent.ts
src/composables/usePersistence.ts
"

TMP_CMDS=$(mktemp)
trap 'rm -f "$TMP_CMDS"' EXIT

for f in $COMPOSABLES; do
  filepath="${ROOT_DIR}/${f}"
  if [ ! -f "$filepath" ]; then
    echo "WARN: composable not found: $filepath"
    continue
  fi
  # Match invoke('cmd') or invoke<Type>('cmd') — both single and double quotes
  # Use grep on lines containing invoke (not just import)
  grep -n "invoke\b" "$filepath" | grep -v "import" | grep -o "'[a-z_][a-z_0-9]*'" | sed "s/'//g" >> "$TMP_CMDS"
  grep -n "invoke\b" "$filepath" | grep -v "import" | grep -o '"[a-z_][a-z_0-9]*"' | sed 's/"//g' >> "$TMP_CMDS"
done

# Deduplicate while preserving order
COMMANDS=$(awk '!seen[$0]++' "$TMP_CMDS")
CMD_COUNT=$(echo "$COMMANDS" | grep -c "." || true)

echo ""
echo "=== Frontend -> Backend Contract Audit ==="
echo "Frontend composable invoke() calls found: $CMD_COUNT"
echo ""

# -- pre-extract registered command names from each backend -----------------

SWIFT_PLUGIN="${ROOT_DIR}/tauri-plugin-pokeclaw/ios/Sources/PokeclawPlugin/PokeclawPlugin.swift"
RUST_LIB="${ROOT_DIR}/tauri-plugin-pokeclaw/src/lib.rs"
RUST_COMMANDS="${ROOT_DIR}/src-tauri/src/commands.rs"

# Swift: @objc public func <name>(_ invoke: Invoke)
SWIFT_FUNCS=""
if [ -f "$SWIFT_PLUGIN" ]; then
  SWIFT_FUNCS=$(grep "@objc public func" "$SWIFT_PLUGIN" | sed 's/.*func \([a-z_]*\).*/\1/' | sort -u)
fi

# Rust desktop_commands in lib.rs: pub fn <name>(...)
RUST_DESKTOP_FUNCS=""
if [ -f "$RUST_LIB" ]; then
  RUST_DESKTOP_FUNCS=$(grep "pub fn\|pub async fn" "$RUST_LIB" | sed 's/.*fn \([a-z_]*\).*/\1/' | sort -u)
fi

# Rust main app commands.rs: pub fn <name>(...)
RUST_MAIN_FUNCS=""
if [ -f "$RUST_COMMANDS" ]; then
  RUST_MAIN_FUNCS=$(grep "pub fn\|pub async fn" "$RUST_COMMANDS" | sed 's/.*fn \([a-z_]*\).*/\1/' | sort -u)
fi

# -- check each frontend command against backends --------------------------

echo "$COMMANDS" | while IFS= read -r cmd; do
  [ -z "$cmd" ] && continue

  swift_match=""
  rust_match=""

  # Check Swift plugin
  if echo "$SWIFT_FUNCS" | grep -qx "$cmd"; then
    swift_match="Swift PokeclawPlugin"
  fi

  # Check Rust desktop commands (lib.rs)
  if echo "$RUST_DESKTOP_FUNCS" | grep -qx "$cmd"; then
    rust_match="Rust lib.rs desktop_commands"
  fi

  # Check Rust main commands (commands.rs — app-level Tauri commands)
  if echo "$RUST_MAIN_FUNCS" | grep -qx "$cmd"; then
    if [ -n "$rust_match" ]; then
      rust_match="$rust_match + commands.rs"
    else
      rust_match="Rust commands.rs (app-level)"
    fi
  fi

  # Determine result
  if [ -n "$swift_match" ] && [ -n "$rust_match" ]; then
    printf "${GREEN}PASS${NC}  %-40s %s\n" "$cmd" "→ $swift_match | $rust_match"
  elif [ -n "$swift_match" ]; then
    printf "${GREEN}PASS${NC}  %-40s %s\n" "$cmd" "→ $swift_match (iOS; unavailableResult stubs)"
  elif [ -n "$rust_match" ]; then
    printf "${GREEN}PASS${NC}  %-40s %s\n" "$cmd" "→ $rust_match"
  else
    printf "${RED}FAIL${NC}  %-40s %s\n" "$cmd" "→ NOT FOUND in Swift or Rust backend!"
    # Write FAIL marker to temp file for counting
    echo "FAIL:$cmd" >> "$TMP_CMDS.fail"
  fi
done

# Count failures
FAIL_COUNT=0
if [ -f "$TMP_CMDS.fail" ]; then
  FAIL_COUNT=$(wc -l < "$TMP_CMDS.fail")
  rm -f "$TMP_CMDS.fail"
fi

# -- summary ----------------------------------------------------------------
PASS_COUNT=$((CMD_COUNT - FAIL_COUNT))

echo ""
echo "=== Summary ==="
printf "Total: %d  PASS: %d  FAIL: %d\n" "$CMD_COUNT" "$PASS_COUNT" "$FAIL_COUNT"
echo ""

if [ "$FAIL_COUNT" -gt 0 ]; then
  echo "❌ Contract audit FAILED — $FAIL_COUNT command(s) missing from backend."
  exit 1
else
  echo "✅ All frontend invoke() calls resolve to backend commands."
  exit 0
fi
