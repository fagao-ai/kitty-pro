#!/usr/bin/env bash
# Fetch the Kotlin compiler jars `scripts/build-android.sh` compiles the VPN bridge with.
#
# The bridge is Kotlin, NSH's Android packaging has no Kotlin step to hook into, and Gradle
# is not in this build at all — so a Kotlin compiler is the one thing the APK needs that no
# other tool in this repository provides. A CI runner starts without one. This script exists
# so the workflow installs it by running something the repository already tested, rather
# than by carrying its own copy of the artifact list.
#
# The jars come from Maven Central rather than the GitHub release archive: the compiler ships
# there as ordinary artifacts, and only the modules `K2JVMCompiler` actually loads are
# fetched. `MAVEN_CENTRAL` overrides the repository for a mirror — measured from this
# machine, repo1.maven.org runs at ~0.3 MB/s and maven.aliyun.com at ~3.4 MB/s, which is the
# difference between a job that finishes and one that hits its timeout.
#
# Files land WITHOUT a version suffix, which is not cosmetic: `build-android.sh` names
# `$kotlin_lib/kotlin-stdlib.jar` outright and passes it to d8, and puts `$kotlin_lib/*` on
# the compiler's classpath. A directory of version-suffixed jars would satisfy the second and
# fail the first.
#
# Usage: install-kotlinc.sh [--version X.Y.Z] [--into DIR]
#
# Prints the directory to point KOTLIN_HOME at, and nothing else on stdout, so it can be used
# as `export KOTLIN_HOME=$(scripts/install-kotlinc.sh)`.
set -euo pipefail

# The compiler the APK that ran on a device was built with. Pinned to what has been measured
# rather than to what is current: 2.0.21 compiled this bridge with `-jvm-target 1.8`, and a
# silent bump here would change the bytecode that carries the traffic.
version=2.0.21
into="${KOTLIN_HOME:-$HOME/.local/kotlinc}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) version="$2"; shift 2 ;;
        --into) into="$2"; shift 2 ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

repo="${MAVEN_CENTRAL:-https://repo1.maven.org/maven2}"
lib="$into/lib"

# `local-name:maven-path`. The path is spelled out in full rather than assembled from the
# artifact name, because Maven repeats the name three times in a path and a script that
# re-derived it got the order wrong on the first try. `$version` is interpolated by the
# array below, so the compiler's five modules move together and the three that do not are
# visibly pinned on their own line.
artifacts=(
    "kotlin-compiler:org/jetbrains/kotlin/kotlin-compiler/$version/kotlin-compiler-$version.jar"
    "kotlin-stdlib:org/jetbrains/kotlin/kotlin-stdlib/$version/kotlin-stdlib-$version.jar"
    "kotlin-reflect:org/jetbrains/kotlin/kotlin-reflect/$version/kotlin-reflect-$version.jar"
    "kotlin-script-runtime:org/jetbrains/kotlin/kotlin-script-runtime/$version/kotlin-script-runtime-$version.jar"
    "kotlin-daemon-embeddable:org/jetbrains/kotlin/kotlin-daemon-embeddable/$version/kotlin-daemon-embeddable-$version.jar"
    # The three the CLI asks for by name and does not ship: `annotations` and `trove4j`,
    # without which K2JVMCompiler fails inside IntelliJ's data structures rather than at
    # something a reader would recognise, and `kotlinx-coroutines-core-jvm`, which it needs
    # to start at all.
    "annotations:org/jetbrains/annotations/13.0/annotations-13.0.jar"
    "trove4j:org/jetbrains/intellij/deps/trove4j/1.0.20200330/trove4j-1.0.20200330.jar"
    "kotlinx-coroutines-core-jvm:org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.6.4/kotlinx-coroutines-core-jvm-1.6.4.jar"
)

command -v curl >/dev/null || {
    echo "curl is required to fetch the Kotlin compiler" >&2
    exit 1
}

# A directory holding two versions of one module is refused rather than fixed, because
# `$kotlin_lib/*` would put both on the compiler's classpath and which one wins is decided by
# readdir order. That is not hypothetical: it is what a hand-assembled toolchain directory
# looks like after a version change, and the failure is a compiler that behaves oddly rather
# than one that is missing.
for entry in "${artifacts[@]}"; do
    name="${entry%%:*}"
    for stray in "$lib/$name"-*.jar; do
        [[ -e $stray ]] || continue
        stray_version="$(basename "$stray" .jar)"
        stray_version="${stray_version#"$name-"}"
        if [[ "$stray_version" != "$version" ]]; then
            echo "$lib holds $(basename "$stray") and is being asked for $name-$version.jar." >&2
            echo "Point --into at a clean directory, or delete the jars of the other version." >&2
            exit 1
        fi
    done
done

mkdir -p "$lib"
for entry in "${artifacts[@]}"; do
    name="${entry%%:*}"
    path="${entry#*:}"
    jar="$lib/$name.jar"
    # Skipped when present, so a runner that restores a cache does not re-download 100 MB to
    # rediscover the files it has. A truncated jar is caught by the compiler check below,
    # which is the reason it exists, rather than by a size test here.
    [[ -s $jar ]] && continue
    url="$repo/$path"
    echo "fetching $url" >&2
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$jar.part" "$url"; then
        echo "could not fetch $url" >&2
        exit 1
    fi
    mv "$jar.part" "$jar"
done

# Prove the compiler loads, and loads as the version asked for, rather than only that some
# jars arrived: a moved artifact or a partial fetch would otherwise surface as a Kotlin stack
# trace from inside the APK build, after the cross-compiled Rust that takes the long part of
# the job.
java_bin="${JAVA_HOME:+$JAVA_HOME/bin/java}"
java_bin="${java_bin:-$(command -v java || true)}"
[[ -n "$java_bin" ]] || {
    echo "no java to verify the compiler with; install JDK 17 or newer" >&2
    exit 1
}
reported="$("$java_bin" -cp "$lib/*" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -version 2>&1 || true)"
if [[ "$reported" != *"kotlinc-jvm $version"* ]]; then
    echo "the compiler in $lib reports '$(printf %s "$reported" | head -1)', not $version" >&2
    exit 1
fi

printf '%s\n' "$into"
