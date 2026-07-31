# Shared location of the Sailfish OS Platform SDK and the cross toolchain.
# Sourced by the other scripts; contains no machine-specific absolute paths —
# override any of these in the environment if your SDK lives elsewhere.
#
#   SAILFISH_SDK_ROOT   where the Platform SDK was unpacked
#   SFOS_RELEASE        release the target was created for
#   SFOS_ARCH           target architecture

SAILFISH_SDK_ROOT="${SAILFISH_SDK_ROOT:-$HOME/SailfishOS-Platform-SDK}"
SFOS_RELEASE="${SFOS_RELEASE:-5.0.0.62}"
SFOS_ARCH="${SFOS_ARCH:-aarch64}"

SFOS_TARGET="SailfishOS-${SFOS_RELEASE}-${SFOS_ARCH}"
SFOS_SYSROOT="${SAILFISH_SDK_ROOT}/targets/${SFOS_TARGET}"
SFOS_CROSS_BIN="${SAILFISH_SDK_ROOT}/toolings/SailfishOS-${SFOS_RELEASE}/opt/cross/bin"

# Cross prefix and Rust triple follow the target architecture.
case "$SFOS_ARCH" in
    aarch64)
        SFOS_CROSS_PREFIX="aarch64-meego-linux-gnu"
        RUST_TARGET="aarch64-unknown-linux-gnu"
        ;;
    armv7hl)
        SFOS_CROSS_PREFIX="armv7hl-meego-linux-gnueabi"
        RUST_TARGET="armv7-unknown-linux-gnueabihf"
        ;;
    i486)
        SFOS_CROSS_PREFIX="i486-meego-linux-gnu"
        RUST_TARGET="i586-unknown-linux-gnu"
        ;;
    *)
        echo "error: unsupported SFOS_ARCH: $SFOS_ARCH (aarch64, armv7hl, i486)" >&2
        return 1 2>/dev/null || exit 1
        ;;
esac

# rustup installs user-local and does not touch the login PATH.
if [ -d "$HOME/.cargo/bin" ]; then
    case ":$PATH:" in
        *":$HOME/.cargo/bin:"*) ;;
        *) PATH="$HOME/.cargo/bin:$PATH"; export PATH ;;
    esac
fi

sdk_env_check() {
    local missing=0
    [ -d "$SFOS_SYSROOT" ] || { echo "error: target sysroot not found: $SFOS_SYSROOT" >&2; missing=1; }
    [ -x "$SFOS_CROSS_BIN/${SFOS_CROSS_PREFIX}-gcc" ] || {
        echo "error: cross compiler not found: $SFOS_CROSS_BIN/${SFOS_CROSS_PREFIX}-gcc" >&2; missing=1; }
    command -v cargo >/dev/null || { echo "error: cargo not on PATH (install rustup)" >&2; missing=1; }
    [ "$missing" -eq 0 ] || {
        echo "hint: set SAILFISH_SDK_ROOT / SFOS_RELEASE / SFOS_ARCH to match your setup" >&2
        return 1
    }
}
