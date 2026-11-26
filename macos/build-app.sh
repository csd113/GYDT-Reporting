#!/bin/sh
set -eu

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

if [ "$#" -ne 1 ]; then
    fail "Usage: $0 path/to/app.zip"
fi
[ "$(uname -s)" = Darwin ] || fail 'Run this script on macOS.'
case "$1" in
    /*) output=$1 ;;
    *) output="$PWD/$1" ;;
esac
case "$output" in
    *.zip) ;;
    *) fail 'Choose an output filename ending in .zip.' ;;
esac
if [ -e "$output" ] || [ -L "$output" ]; then
    fail 'The output path already exists. Choose a new filename.'
fi
output_dir=$(cd -P "$(dirname "$output")" && pwd)
archive_name=$(basename "$output")

cd "$(dirname "$0")/.."
./scripts/check-synthetic-inputs.sh
target=aarch64-apple-darwin
minimum_macos=11.0
executable=gydt-results-compiler
app_name='GYDT Sample Reporting'
bundle="$app_name.app"
contents="$bundle/Contents"
plist="$contents/Info.plist"
binary="$contents/MacOS/$executable"
icon="$contents/Resources/AppIcon.icns"

metadata=$(cargo metadata --format-version 1 --no-deps --locked)
cargo_version=$(printf '%s\n' "$metadata" | plutil -extract packages.0.version raw -expect string -o - -)
target_dir=$(printf '%s\n' "$metadata" | plutil -extract target_directory raw -expect string -o - -)
# macOS version keys take only the numeric part of a Cargo SemVer.
bundle_version=${cargo_version%%[-+]*}

repo_dir=$(pwd -P)
remap_wrapper="$repo_dir/macos/remap-rustc.sh"
cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
case "$cargo_home" in
    /*) ;;
    *) cargo_home="$repo_dir/$cargo_home" ;;
esac
if [ -d "$cargo_home" ]; then
    cargo_home=$(cd -P "$cargo_home" && pwd)
fi
toolchain_lib="$(rustc --print sysroot)/lib"
# Cargo does not fingerprint flags added by a compiler wrapper.
remap_key=$({ printf '%s\000' "$HOME" "$cargo_home" "$toolchain_lib"; cat "$remap_wrapper"; } | shasum -a 256)
target_dir="$target_dir/gydt-macos/${remap_key%% *}"
GYDT_BUNDLE_HOME="$HOME" \
GYDT_BUNDLE_CARGO_HOME="$cargo_home" \
GYDT_BUNDLE_TOOLCHAIN_LIB="$toolchain_lib" \
GYDT_BUNDLE_RUSTC_WRAPPER="${RUSTC_WRAPPER-${CARGO_BUILD_RUSTC_WRAPPER-}}" \
RUSTC_WRAPPER="$remap_wrapper" \
CARGO_BUILD_BUILD_DIR="$target_dir" \
MACOSX_DEPLOYMENT_TARGET="$minimum_macos" \
    cargo build --release --locked --no-default-features --target "$target" --bin "$executable" --target-dir "$target_dir"
built_binary="$target_dir/$target/release/$executable"
if LC_ALL=C grep -aqF -e "$HOME" -e "$cargo_home/registry/" -- "$built_binary"; then
    fail 'The executable still contains a builder home or Cargo registry path.'
elif [ "$?" -ne 1 ]; then
    fail 'Could not check the executable for builder paths.'
fi
[ "$(lipo -archs "$built_binary")" = arm64 ] || fail 'Expected an arm64 executable.'
build_info=$(xcrun vtool -show-build "$built_binary")
built_minimum=$(printf '%s\n' "$build_info" | awk '$1 == "minos" { print $2 }')
[ "$built_minimum" = "$minimum_macos" ] || fail "Expected a macOS $minimum_macos deployment target."

stage=$(mktemp -d "$output_dir/.gydt-macos.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
iconset="$stage/AppIcon.iconset"
master="$stage/AppIcon.png"
sips -s format png -z 1024 1024 assets/app-icon.svg --out "$master" >/dev/null
mkdir "$iconset"
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -z "$size" "$size" "$master" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$double" "$double" "$master" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

cd "$stage"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 755 "$built_binary" "$binary"
iconutil -c icns -o "$icon" "$iconset"
cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>$app_name</string>
    <key>CFBundleExecutable</key><string>$executable</string>
    <key>CFBundleIconFile</key><string>AppIcon.icns</string>
    <key>CFBundleIdentifier</key><string>com.getyourdrugstested.sample-reporting.profile</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>GYDT Reporting</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$bundle_version</string>
    <key>CFBundleVersion</key><string>$bundle_version</string>
    <key>LSMinimumSystemVersion</key><string>$built_minimum</string>
</dict>
</plist>
EOF
plutil -lint "$plist" >/dev/null
chmod 644 "$plist" "$icon"

# Keep the ZIP independent of staging times and include only the bundle's files.
TZ=UTC touch -t 200001010000 "$plist" "$binary" "$icon"
TZ=UTC ZIPOPT= zip -q -X "$stage/$archive_name" "$plist" "$binary" "$icon"
UNZIP= UNZIPOPT= unzip -tq "$stage/$archive_name" >/dev/null
# Publish the verified ZIP atomically without replacing a competing output.
/bin/link "$stage/$archive_name" "$output_dir/$archive_name"
printf '%s\n' "$output_dir/$archive_name"
