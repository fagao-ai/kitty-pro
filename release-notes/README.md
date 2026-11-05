# Release notes

One file per release, named after the tag: `v1.2.3.md`. The release
workflow (`github-release` job) feeds the file for the pushed tag to
`scripts/build-release-notes.sh`, which assembles the GitHub Release body.

## Shape

Follow magpie's format: an English half, the `<!-- lang:zh -->` marker, then
the Chinese half. Section headings are up to you (`## Features`,
`## Bug Fixes`, `## Improvements`, ...); one `-` bullet per user-visible
change, phrased for users rather than for the git log.

````markdown
## Features

- Short English description of a user-visible addition.

## Bug Fixes

- Short English description of what no longer misbehaves.

<!-- lang:zh -->

## 新功能

- 用户视角的中文描述。

## 修复

- 用户视角的中文描述。
````

Do **not** write an Install / 安装 section. The script appends a bilingual
one listing every asset with its real file names and version, because those
names are derived from the tag and must stay exact.

## If a release has no curated file

The script falls back to `CHANGELOG.md`'s `## Unreleased` section, and then
to an automatic English listing of conventional-commit subjects since the
previous tag (with a workflow warning). Releases never go out note-less, but
the commit listing is English-only and reads like a git log — write the
curated file before tagging when you can.
