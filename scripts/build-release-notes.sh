#!/usr/bin/env bash
# Build the GitHub Release body for one tag, in the style magpie ships:
# curated notes when the maintainer wrote them, a conventional-commit
# listing as fallback, and always a bilingual install footer listing the
# release assets by their real file names.
#
# Note sources, in order of precedence:
#   1. release-notes/<tag>.md   -- curated notes. English first, then the
#      `<!-- lang:zh -->` marker, then the Chinese translation (see
#      release-notes/README.md for the shape). Do NOT write an Install
#      section in it; this script appends the bilingual one below.
#   2. CHANGELOG.md             -- the `## Unreleased` / `## [Unreleased]`
#      section, used verbatim when the curated file is missing.
#   3. Git history              -- non-merge conventional-commit subjects
#      since the previous tag, grouped into Features / Bug Fixes /
#      Improvements / Other. English only; a workflow annotation tells the
#      Actions log so the next release can add the curated file.
#
# Usage: build-release-notes.sh [--out FILE] <tag>
set -euo pipefail

out=""
while [[ $# -gt 1 && $1 == --out ]]; do
    out=$2
    shift 2
done
if [[ $# -ne 1 || ! $1 =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "usage: $0 [--out FILE] vX.Y.Z[-suffix]" >&2
    exit 2
fi
tag=$1
version=${tag#v}

if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "tag $tag not found in this repository" >&2
    exit 1
fi

# The tag immediately before this one; the first release has none.
prev="$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)"

# Collapse leading/trailing blank lines so blocks join with exactly one.
trim() {
    printf '%s\n' "$1" | sed -e '/[^[:space:]]/,$!d' \
        | awk '{ lines[NR] = $0 } END { last = NR; while (last > 0 && lines[last] ~ /^[[:space:]]*$/) last--; for (i = 1; i <= last; i++) print lines[i] }'
}

# ---------------------------------------------------------------------------
# Notes body
# ---------------------------------------------------------------------------
notes=""
source_kind=""
if [[ -f "release-notes/$tag.md" ]]; then
    notes="$(cat "release-notes/$tag.md")"
    source_kind="curated"
elif [[ -f CHANGELOG.md ]]; then
    notes="$(awk '
        /^##[[:space:]]+(\[)?Unreleased(\])?/ { capture = 1; next }
        capture && /^##[[:space:]]/           { exit }
        capture                               { print }
    ' CHANGELOG.md)"
    notes="$(trim "$notes")"
    [[ -n $notes ]] && source_kind="changelog"
fi
if [[ -z $notes ]]; then
    if [[ -z $prev ]]; then
        echo "no release-notes/$tag.md, no CHANGELOG.md section, and no" \
            "previous tag to derive notes from" >&2
        exit 1
    fi
    source_kind="commits"
    echo "::warning title=Release notes::no curated release-notes/$tag.md;" \
        "generated an English commit listing instead" >&2
    notes="$(
        git log --no-merges --pretty=format:'%s' "${prev}..${tag}" | awk \
            -v prev="$prev" '
            function flush(title, items, count,    i) {
                if (count == 0) return
                printf "## %s\n\n", title
                for (i = 1; i <= count; i++) printf "- %s\n", items[i]
                printf "\n"
            }
            { subject = $0; sub(/^[a-z]+(\([^)]*\))?(!)?: /, "", subject) }
            /^feat/          { nf++;    features[nf]    = subject; next }
            /^fix/           { nx++;    fixes[nx]       = subject; next }
            /^(perf|refactor)/ { ni++;  improvements[ni] = subject; next }
            { no++; other[no] = subject }
            END {
                printf "_Changes since %s._\n\n", prev
                flush("Features", features, nf)
                flush("Bug Fixes", fixes, nx)
                flush("Improvements", improvements, ni)
                flush("Other", other, no)
            }
        '
    )"
fi

# ---------------------------------------------------------------------------
# Bilingual install footer. Asset names must match the files the release job
# actually uploads (see .github/workflows/release.yml).
# ---------------------------------------------------------------------------
install_en="$(cat <<EOF
### Install

- **macOS (Apple Silicon):** \`Kitty-Pro-macos-aarch64.zip\` — unzip and move
  \`Kitty Pro.app\` to Applications
- **Windows (x86_64):** \`Kitty-Pro-windows-x86_64.zip\`
- **Linux (x86_64):** \`Kitty-Pro-linux-x86_64.tar.gz\`
- **Android (arm64 + armv7):** \`Kitty-Pro-arm64-v8a-armeabi-v7a-${version}.apk\`
- **Web server (Linux x86_64):** \`Kitty-Pro-web-linux-x86_64.tar.gz\` — the
  browser page plus the process that runs the core for it; unzip, run
  \`./kitty-pro-web-server\`, open \`http://127.0.0.1:7897/\`
- **Web server (macOS Apple Silicon):** \`Kitty-Pro-web-macos-aarch64.tar.gz\`

Every archive has a matching \`.sha256\` sidecar; verify it before the first run.
Unzip and run the app from wherever you keep it: macOS builds are ad hoc signed
and not notarized, so the first launch needs a right-click → Open.
EOF
)"
install_zh="$(cat <<EOF
### 安装

- **macOS（Apple 芯片）：** \`Kitty-Pro-macos-aarch64.zip\` —— 解压后把
  \`Kitty Pro.app\` 拖进「应用程序」
- **Windows（x86_64）：** \`Kitty-Pro-windows-x86_64.zip\`
- **Linux（x86_64）：** \`Kitty-Pro-linux-x86_64.tar.gz\`
- **Android（arm64 + armv7）：** \`Kitty-Pro-arm64-v8a-armeabi-v7a-${version}.apk\`
- **Web 服务端（Linux x86_64）：** \`Kitty-Pro-web-linux-x86_64.tar.gz\` ——
  浏览器页面与替它运行核心的进程一起打包；解压后执行 \`./kitty-pro-web-server\`，
  浏览器打开 \`http://127.0.0.1:7897/\`
- **Web 服务端（macOS Apple 芯片）：** \`Kitty-Pro-web-macos-aarch64.tar.gz\`

每个压缩包都附带同名 \`.sha256\` 校验文件，首次运行前请先校验。macOS 构建仅做
ad-hoc 签名、未公证，首次打开请右键选择「打开」。
EOF
)"

# Curated notes are already bilingual: splice the English install block in
# front of the `<!-- lang:zh -->` marker and append the Chinese one at the
# end. Fallback notes are English-only, so they keep the full bilingual tail
# (the zh half still gives Chinese readers the install info).
if [[ $notes == *'<!-- lang:zh -->'* ]]; then
    en_part="$(trim "${notes%%<!-- lang:zh -->*}")"
    zh_part="$(trim "${notes#*<!-- lang:zh -->}")"
    body="${en_part}

${install_en}

<!-- lang:zh -->

${zh_part}

${install_zh}"
else
    body="$(trim "$notes")

${install_en}

<!-- lang:zh -->

${install_zh}"
fi

if [[ -n $out ]]; then
    printf '%s\n' "$body" > "$out"
else
    printf '%s\n' "$body"
fi
echo "release notes for $tag built from ${source_kind} notes" >&2
