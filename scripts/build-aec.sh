#!/bin/bash
# Cross-build the call echo canceller - webrtc-audio-processing (AEC3) and the
# abseil it needs - into static libraries for the Sailfish target, and write the
# qmake fragment that links them.
#
# Built on the host like the Rust core, with the same SDK wrappers: the targets
# carry no abseil and no webrtc-audio-processing, and their gcc 10 does C++17.
# Sources are fetched once, checked against pinned hashes, and kept under
# core/target/aec/src. The result lands in core/target/aec/<rust triple>.
#
# Usage: scripts/build-aec.sh   (after scripts/build-core.sh for the same target)

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$HERE")"
# shellcheck source=sdk-env.sh
. "$HERE/sdk-env.sh"
sdk_env_check

SDK_BIN="$ROOT/core/target/sdk-bin"
if [ ! -x "$SDK_BIN/cross-g++" ]; then
    echo "error: $SDK_BIN/cross-g++ is missing - run scripts/build-core.sh first" >&2
    exit 1
fi
# The wrappers belong to whichever target build-core.sh ran for last.
if ! grep -qF -- "\"${SFOS_CROSS_BIN}/${SFOS_CROSS_PREFIX}-g++\" --sysroot=\"${SFOS_SYSROOT}\" " "$SDK_BIN/cross-g++"; then
    echo "error: $SDK_BIN/cross-g++ is not for ${SFOS_CROSS_PREFIX} - run scripts/build-core.sh for this target first" >&2
    exit 1
fi

# Linking (the build systems' compiler checks do) needs the prefixed binutils:
# collect2 looks for the target's `ld` by its full name on PATH.
export PATH="${SFOS_CROSS_BIN}:$PATH"

ABSL_VERSION=20240722.0
ABSL_SHA256=f50e5ac311a81382da7fa75b97310e4b9006474f9560ac46f54a9967f07d4ae3
ABSL_URL="https://github.com/abseil/abseil-cpp/archive/refs/tags/${ABSL_VERSION}.tar.gz"
WAP_VERSION=2.1
WAP_SHA256=2358883a8551711f3b6169f6799c003dc1a6efd37bdb24a135a015cc9e3d7713
WAP_URL="https://gitlab.freedesktop.org/pulseaudio/webrtc-audio-processing/-/archive/v${WAP_VERSION}/webrtc-audio-processing-v${WAP_VERSION}.tar.gz"

AEC="$ROOT/core/target/aec"
SRC="$AEC/src"
PREFIX="$AEC/$RUST_TARGET"
WORK="$AEC/build-$RUST_TARGET"
mkdir -p "$SRC"

# --- sources ----------------------------------------------------------------
fetch() {
    local url="$1" sha="$2" file="$3"
    if [ ! -f "$SRC/$file" ]; then
        curl -sSLf -o "$SRC/$file.part" "$url"
        mv "$SRC/$file.part" "$SRC/$file"
    fi
    echo "$sha  $SRC/$file" | sha256sum -c --quiet - || {
        echo "error: $file does not match its pinned hash" >&2
        exit 1
    }
}
fetch "$ABSL_URL" "$ABSL_SHA256" "abseil-cpp-${ABSL_VERSION}.tar.gz"
fetch "$WAP_URL" "$WAP_SHA256" "webrtc-audio-processing-${WAP_VERSION}.tar.gz"

rm -rf "$WORK" "$PREFIX"
mkdir -p "$WORK"
tar xzf "$SRC/abseil-cpp-${ABSL_VERSION}.tar.gz" -C "$WORK"
tar xzf "$SRC/webrtc-audio-processing-${WAP_VERSION}.tar.gz" -C "$WORK"

# No build machine path in the binary (__FILE__ in assertions), and the app's own hardening.
# Each path also as resolved: a symlinked checkout reaches the compiler that way.
ROOT_REAL="$(realpath "$ROOT")"
SDK_REAL="$(realpath "$SAILFISH_SDK_ROOT")"
FLAGS=("-ffile-prefix-map=$ROOT=/build" "-ffile-prefix-map=$SAILFISH_SDK_ROOT=/sdk")
[ "$ROOT_REAL" = "$ROOT" ] || FLAGS+=("-ffile-prefix-map=$ROOT_REAL=/build")
[ "$SDK_REAL" = "$SAILFISH_SDK_ROOT" ] || FLAGS+=("-ffile-prefix-map=$SDK_REAL=/sdk")
FLAGS+=(-D_FORTIFY_SOURCE=2 -fstack-protector-strong)

case "$SFOS_ARCH" in
    aarch64) CPU_FAMILY=aarch64; CPU=aarch64 ;;
    armv7hl) CPU_FAMILY=arm; CPU=armv7hl ;;
    i486) CPU_FAMILY=x86; CPU=i686 ;;
    *) echo "error: unsupported architecture $SFOS_ARCH" >&2; exit 1 ;;
esac

# --- abseil -------------------------------------------------------------------
cat > "$WORK/toolchain.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR ${CPU})
set(CMAKE_C_COMPILER ${SDK_BIN}/cross-gcc)
set(CMAKE_CXX_COMPILER ${SDK_BIN}/cross-g++)
set(CMAKE_AR ${SDK_BIN}/ar)
set(CMAKE_RANLIB ${SDK_BIN}/ranlib)
set(CMAKE_FIND_ROOT_PATH ${SFOS_SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
EOF

echo "→ abseil ${ABSL_VERSION} for ${RUST_TARGET}"
cmake -S "$WORK/abseil-cpp-${ABSL_VERSION}" -B "$WORK/absl" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$WORK/toolchain.cmake" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_FLAGS="${FLAGS[*]}" \
    -DCMAKE_CXX_FLAGS="${FLAGS[*]}" \
    -DCMAKE_CXX_STANDARD=17 \
    -DABSL_PROPAGATE_CXX_STD=ON \
    -DABSL_ENABLE_INSTALL=ON \
    -DBUILD_TESTING=OFF \
    -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_INSTALL_LIBDIR=lib > "$WORK/absl-configure.log"
ninja -C "$WORK/absl" install > "$WORK/absl-build.log"

# --- webrtc-audio-processing ---------------------------------------------------
cat > "$WORK/cross.ini" <<EOF
[binaries]
c = '${SDK_BIN}/cross-gcc'
cpp = '${SDK_BIN}/cross-g++'
ar = '${SDK_BIN}/ar'
strip = '${SDK_BIN}/strip'
pkg-config = 'pkg-config'

[properties]
pkg_config_libdir = ['${PREFIX}/lib/pkgconfig']

[built-in options]
c_args = [$(printf "'%s', " "${FLAGS[@]}")]
cpp_args = [$(printf "'%s', " "${FLAGS[@]}")]

[host_machine]
system = 'linux'
cpu_family = '${CPU_FAMILY}'
cpu = '${CPU}'
endian = 'little'
EOF

echo "→ webrtc-audio-processing ${WAP_VERSION} for ${RUST_TARGET}"
meson setup "$WORK/wap" "$WORK/webrtc-audio-processing-v${WAP_VERSION}" \
    --cross-file "$WORK/cross.ini" \
    --prefix "$PREFIX" --libdir lib \
    --buildtype release \
    --default-library static \
    --wrap-mode nodownload \
    -Db_staticpic=true \
    -Dcpp_std=c++17 > "$WORK/wap-configure.log"
ninja -C "$WORK/wap" install > "$WORK/wap-build.log"

# --- path gate -----------------------------------------------------------------
# A map that missed leaves the build machine in the archives; no fragment then.
HOME_REAL="$(realpath "$HOME")"
LEAK=0
for lib in "$PREFIX"/lib/*.a; do
    for needle in "$HOME" "$HOME_REAL" "$ROOT" "$ROOT_REAL" "$SAILFISH_SDK_ROOT" "$SDK_REAL"; do
        # On the file: grep -q on a pipe ends it early, and pipefail reads that as no match.
        if grep -aqF -- "$needle" "$lib"; then
            echo "error: $(basename "$lib") contains a build machine path" >&2
            LEAK=1
            break
        fi
    done
done
[ "$LEAK" -eq 0 ] || exit 1

# --- qmake fragment ------------------------------------------------------------
# The link line in dependency order, as pkg-config resolves it for a static link.
LIBS_LINE="$(PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" pkg-config --static --libs webrtc-audio-processing-2)"
INCLUDE_DIRS="$(PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" pkg-config --cflags-only-I webrtc-audio-processing-2 | sed 's/-I//g')"
cat > "$PREFIX/aec.pri" <<EOF
# Generated by scripts/build-aec.sh — do not edit.
INCLUDEPATH += ${INCLUDE_DIRS}
LIBS += -Wl,--start-group ${LIBS_LINE} -Wl,--end-group
EOF

echo "→ done: $(du -sh "$PREFIX/lib" | cut -f1)  $PREFIX"
