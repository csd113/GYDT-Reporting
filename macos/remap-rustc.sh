#!/bin/sh
set -eu

# Let Cargo select the user's rustflags, then append the packaging remaps.
set -- "$@" \
    "--remap-path-prefix=${GYDT_BUNDLE_HOME%/}/=/build-home/" \
    "--remap-path-prefix=${GYDT_BUNDLE_CARGO_HOME%/}/registry/=/cargo/registry/" \
    --remap-path-scope=object
if [ -n "$GYDT_BUNDLE_RUSTC_WRAPPER" ]; then
    set -- "$GYDT_BUNDLE_RUSTC_WRAPPER" "$@"
fi
# The pinned toolchain's macOS rust-objcopy needs LLVM from the sysroot.
DYLD_FALLBACK_LIBRARY_PATH="${DYLD_FALLBACK_LIBRARY_PATH:+$DYLD_FALLBACK_LIBRARY_PATH:}$GYDT_BUNDLE_TOOLCHAIN_LIB" \
    exec "$@"
