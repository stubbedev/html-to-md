_default:
    @just --list

# Build the release binary.
build:
    go -C go build -trimpath -o bin/html-to-md ./cmd/html-to-md

# Run tests.
test:
    go -C go test ./...

# Re-run tests on every change (requires entr).
test-watch:
    find go -name '*.go' | entr -s 'go -C go test ./...'

# Auto-fix formatting, then golangci-lint.
lint: fmt
    cd go && golangci-lint run --fix

# Read-only golangci-lint (what CI runs).
lint-ci:
    cd go && golangci-lint run

fmt:
    cd go && golangci-lint fmt

# Keep flake.nix's version aligned with the source's version string. Pass a
# `version` to rewrite flake.nix (release use).
sync-flake version="":
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{version}}"
    SRC_VERSION=$(awk -F'"' '/^  version = "/ {print $2; exit}' flake.nix)

    if [ -n "$ARG" ]; then
        NEW="${ARG#v}"
        if [ "$NEW" != "$SRC_VERSION" ]; then
            sed -i -E "s/^  version = \"[^\"]*\";/  version = \"$NEW\";/" flake.nix
            SRC_VERSION="$NEW"
            echo "sync-flake: version -> $NEW"
        fi
    fi
    echo "sync-flake: up-to-date (version=$SRC_VERSION)"

nix-check:
    nix flake check --print-build-logs

# Everything CI runs, with auto-fix where possible.
check: lint lint-ci test

# Render an HTML sample through a fresh build: `just try sample.html`,
# or pipe stdin with `curl … | just try`. Auto-detects the input format.
try file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{file}}" = "-" ]; then
        go -C go run ./cmd/html-to-md
    else
        go -C go run ./cmd/html-to-md < "{{file}}"
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
            notmuch show --part="$part" --format=raw "$id" | go -C go run ./cmd/html-to-md
        else
            echo "(no text/html part)"
        fi
    done <<< "$ids"

# Render an iCalendar sample through the calendar pipeline.
try-ics file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{file}}" = "-" ]; then
        go -C go run ./cmd/html-to-md --calendar
    else
        go -C go run ./cmd/html-to-md --calendar < "{{file}}"
    fi

# Render a text sample as if it were a format=flowed part
# (AERC_FORMAT=flowed); pass AERC_FORMAT= to test passthrough.
try-plain file="-":
    #!/usr/bin/env bash
    set -euo pipefail
    AERC_FORMAT="${AERC_FORMAT:-flowed}" go -C go run ./cmd/html-to-md --plain \
        < <(if [ "{{file}}" = "-" ]; then cat; else cat "{{file}}"; fi)

# Enter the flake development shell.
dev:
    nix develop

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
    if [ -n "$(git status --porcelain flake.nix)" ]; then
        git add flake.nix
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
