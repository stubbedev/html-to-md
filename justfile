_default:
    @just --list

# Build the release binary.
build:
    cargo build --release

# Run tests (cargo-nextest).
test:
    cargo nextest run

# Re-run tests on every change.
test-watch:
    cargo nextest watch

# Auto-fix formatting, then the full clippy gate (warnings = errors).
lint: fmt
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt

# Strict read-only check — same logic CI runs.
lint-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo nextest run

nix-check:
    nix flake check --print-build-logs

# Everything CI runs, with auto-fix where possible.
check: lint test sync-flake

# Render an HTML sample through the debug build: `just try sample.html`,
# or pipe stdin with `curl … | just try`. Auto-detects the input format.
try file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{file}}" = "-" ]; then
        cargo run --quiet
    else
        cargo run --quiet < "{{file}}"
    fi

# Render the text/html part of recent messages matching a notmuch query.
# Reads the local index and prints converted markdown per message; an
# eyeball harness for output regressions. Nothing is written or committed.
try-mail query="tag:inbox" count="5":
    #!/usr/bin/env bash
    set -euo pipefail
    ids=$(notmuch search --output=messages "{{query}}" | head -n "{{count}}")
    if [ -z "$ids" ]; then
        echo "try-mail: no messages match '{{query}}'" >&2
        exit 1
    fi
    while IFS= read -r id; do
        subject=$(notmuch show --format=json "$id" \
            | jq -r '.[0][0][0].headers.Subject' 2>/dev/null || true)
        part=$(notmuch show --format=json --body=true "$id" \
            | jq -r '[.. | objects | select(."content-type"? == "text/html") | .id] | .[0]' 2>/dev/null || true)
        echo
        echo "════ ${subject:-<no subject>} ($id)"
        if [ -n "$part" ] && [ "$part" != "null" ]; then
            notmuch show --part="$part" --format=raw "$id" | cargo run --quiet
        else
            echo "(no text/html part)"
        fi
    done <<< "$ids"

# Render an iCalendar sample through the ics-to-md debug build.
try-ics file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{file}}" = "-" ]; then
        cargo run --quiet -- --calendar
    else
        cargo run --quiet -- --calendar < "{{file}}"
    fi

# Render a text sample through the text-plain debug build as if it were a
# format=flowed part (AERC_FORMAT=flowed); pass AERC_FORMAT= to test passthrough.
try-plain file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    AERC_FORMAT="${AERC_FORMAT:-flowed}" cargo run --quiet -- --plain \
        < <(if [ "{{file}}" = "-" ]; then cat; else cat "{{file}}"; fi)

# Enter the flake development shell.
dev:
    nix develop

# Keep flake.nix's version aligned with Cargo.toml. Unlike a Go module there is
# no vendor/cargo hash to chase — `cargoLock.lockFile` reads Cargo.lock — so
# this only syncs the version string. Pass a `version` to rewrite Cargo.toml,
# Cargo.lock and flake.nix (release use).
sync-flake version="":
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{version}}"
    CARGO_VERSION=$(awk -F'"' '/^version = "/ {print $2; exit}' Cargo.toml)

    if [ -n "$ARG" ]; then
        NEW="${ARG#v}"
        if [ "$NEW" != "$CARGO_VERSION" ]; then
            sed -i -E '0,/^version = "[^"]*"/s//version = "'"$NEW"'"/' Cargo.toml
            # Refresh the html-to-md entry in Cargo.lock to the new version.
            cargo update --workspace --quiet 2>/dev/null || cargo generate-lockfile
            CARGO_VERSION="$NEW"
            echo "sync-flake: Cargo.toml version -> $NEW"
        fi
    fi

    FLAKE_VERSION=$(awk -F'"' '/^[[:space:]]*version = "/ {print $2; exit}' flake.nix)
    if [ "$FLAKE_VERSION" != "$CARGO_VERSION" ]; then
        sed -i -E '0,/(version = )"[^"]*";/s//\1"'"$CARGO_VERSION"'";/' flake.nix
        echo "sync-flake: flake.nix version -> $CARGO_VERSION"
    else
        echo "sync-flake: up-to-date (version=$CARGO_VERSION)"
    fi

# ─────────────────────────── Release ───────────────────────────

release-preview:
    #!/usr/bin/env bash
    set -euo pipefail
    CURRENT_TAG=$(git tag -l 'v*.*.*' --sort=-v:refname | head -1)
    CURRENT_TAG=${CURRENT_TAG:-v0.0.0}
    CURRENT_VERSION=${CURRENT_TAG#v}
    MAJOR=$(echo "$CURRENT_VERSION" | cut -d. -f1)
    MINOR=$(echo "$CURRENT_VERSION" | cut -d. -f2)
    PATCH=$(echo "$CURRENT_VERSION" | cut -d. -f3)
    echo "Current tag: $CURRENT_TAG"
    echo "  release-major: v$((MAJOR + 1)).0.0"
    echo "  release-minor: v${MAJOR}.$((MINOR + 1)).0"
    echo "  release-patch: v${MAJOR}.${MINOR}.$((PATCH + 1))"

_release-checks:
    #!/usr/bin/env bash
    set -euo pipefail
    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    DEFAULT_BRANCH=$(git rev-parse --abbrev-ref origin/HEAD 2>/dev/null | sed 's|^origin/||' || true)
    if [ -z "${DEFAULT_BRANCH:-}" ]; then
        DEFAULT_BRANCH=$(git remote show origin 2>/dev/null | awk '/HEAD branch/ {print $NF}' || echo master)
    fi
    if [ "$BRANCH" != "$DEFAULT_BRANCH" ]; then
        echo "Error: not on default branch '$DEFAULT_BRANCH' (currently '$BRANCH')." >&2
        exit 1
    fi
    just check
    if [ -n "$(git status --porcelain)" ]; then
        echo "Formatting/lint produced changes — staging + committing."
        git add -A
        git commit -m "chore: format code for release"
    fi

_release bump:
    #!/usr/bin/env bash
    set -euo pipefail
    just _release-checks
    CURRENT_TAG=$(git tag -l 'v*.*.*' --sort=-v:refname | head -1)
    CURRENT_TAG=${CURRENT_TAG:-v0.0.0}
    CURRENT_VERSION=${CURRENT_TAG#v}
    MAJOR=$(echo "$CURRENT_VERSION" | cut -d. -f1)
    MINOR=$(echo "$CURRENT_VERSION" | cut -d. -f2)
    PATCH=$(echo "$CURRENT_VERSION" | cut -d. -f3)
    case "{{bump}}" in
        major) NEW="$((MAJOR + 1)).0.0" ;;
        minor) NEW="${MAJOR}.$((MINOR + 1)).0" ;;
        patch) NEW="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
        *) echo "unknown bump kind: {{bump}}"; exit 1 ;;
    esac
    just sync-flake "${NEW}"
    if [ -n "$(git status --porcelain Cargo.toml Cargo.lock flake.nix)" ]; then
        git add Cargo.toml Cargo.lock flake.nix
        git commit -m "chore: bump to v${NEW}"
    fi
    git tag -a "v${NEW}" -m "v${NEW}"
    git push origin HEAD
    git push origin "v${NEW}"
    echo
    echo "Tagged v${NEW}. Watch: gh run watch || open https://github.com/stubbedev/html-to-md/actions"

release-patch: (_release "patch")
release-minor: (_release "minor")
release-major: (_release "major")
