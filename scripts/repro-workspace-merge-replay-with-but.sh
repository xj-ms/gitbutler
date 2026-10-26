#!/usr/bin/env bash

set -eu -o pipefail

but_bin="${BUT_BIN:-but}"
repro_dir="${1:-workspace-merge-replay-repro}"

if test -e "$repro_dir"; then
  echo "Refusing to overwrite existing path: $repro_dir" >&2
  exit 2
fi

mkdir -p "$repro_dir"
repro_dir="$(cd "$repro_dir" && pwd)"
export NO_BG_TASKS=1

run_but() {
  "$but_bin" -C "$repro_dir" "$@"
}

write_base() {
  cat >"$repro_dir/state.txt" <<'EOF'
bool f() {
    op();
    return true;
}
EOF
}

write_reason() {
  cat >"$repro_dir/state.txt" <<'EOF'
bool f() {
    op("reason");
    return true;
}
EOF
}

write_combined() {
  cat >"$repro_dir/state.txt" <<'EOF'
void f() {
    op("reason");
}
EOF
}

# Create a non-empty target entirely through GitButler.
run_but setup --init
run_but branch new base
write_base
run_but commit --branch base --message M state.txt
run_but land base --yes

# Build the valid combined state as a stack: B adds the operation reason, and A
# removes the return value while retaining B's change.
run_but branch new B
write_reason
run_but commit --branch B --message B state.txt
run_but branch new A --above B
write_combined
run_but commit --branch A --message A state.txt

# Turning A and B into parallel branches makes A conflicted relative to the
# target. Resolving it to the same combined content asks GitButler to rebuild
# the managed workspace commit from the now-conflicting parallel tips.
run_but move A --unstack
write_combined
cp "$repro_dir/state.txt" "$repro_dir/resolved-state.txt"

set +e
output=$(
  run_but resolve apply state.txt \
    --commit A \
    --file "$repro_dir/resolved-state.txt" 2>&1
)
status=$?
set -e

printf '%s\n' "$output"

if test "$status" -eq 0; then
  echo "The workspace merge replay did not fail." >&2
  exit 1
fi

if ! printf '%s\n' "$output" |
  grep -q "Failed to merge bases while cherry picking commit"; then
  echo "The command failed, but not with the expected workspace replay error." >&2
  exit "$status"
fi

echo
echo "Reproduced the workspace merge replay failure in: $repro_dir"
