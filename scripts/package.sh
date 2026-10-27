#!/usr/bin/env bash
# Package a release build for one target into dist/.
# Usage: scripts/package.sh <rust-target> <version>
# Expects `cargo build --release --target <rust-target>` to have run.
set -euo pipefail

target="$1"
version="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
name="vais-commander"
bin_dir="$root/target/$target/release"
dist="$root/dist"
mkdir -p "$dist"
arch="${target%%-*}"

case "$target" in
    *-linux-*)
        stage="$dist/$name-$version-$arch-linux"
        rm -rf "$stage"
        mkdir -p "$stage"
        cp "$bin_dir/$name" "$stage/"
        cp "$root/assets/$name.desktop" "$root/assets/$name.svg" "$root/assets/vais.portal" "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$stage/"
        cp "$root/scripts/install.sh" "$stage/install.sh"
        cp "$root/scripts/install-portal.sh" "$stage/install-portal.sh"
        chmod +x "$stage/install.sh" "$stage/install-portal.sh" "$stage/$name"
        tar -C "$dist" -czf "$stage.tar.gz" "$(basename "$stage")"
        rm -rf "$stage"
        echo "$stage.tar.gz"
        ;;
    *-apple-darwin)
        app="$dist/VAIS Commander.app"
        rm -rf "$app"
        mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
        cp "$bin_dir/$name" "$app/Contents/MacOS/$name"
        chmod +x "$app/Contents/MacOS/$name"
        cp "$root/CHANGELOG.md" "$root/LICENSE" "$app/Contents/Resources/"
        # Icon: iconset from the 1024px PNG, then icns.
        iconset="$dist/AppIcon.iconset"
        rm -rf "$iconset"
        mkdir -p "$iconset"
        for size in 16 32 64 128 256 512; do
            sips -z $size $size "$root/assets/$name-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
            double=$((size * 2))
            sips -z $double $double "$root/assets/$name-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
        done
        iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
        rm -rf "$iconset"
        cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>VAIS Commander</string>
    <key>CFBundleDisplayName</key><string>VAIS Commander</string>
    <key>CFBundleIdentifier</key><string>cz.vais.commander</string>
    <key>CFBundleExecutable</key><string>$name</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
        zip="$dist/$name-$version-$arch-macos.zip"
        rm -f "$zip"
        ditto -c -k --keepParent "$app" "$zip"
        rm -rf "$app"
        echo "$zip"
        ;;
    *-windows-*)
        stage="$dist/$name-$version-$arch-windows"
        rm -rf "$stage"
        mkdir -p "$stage"
        cp "$bin_dir/$name.exe" "$stage/"
        cp "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$stage/"
        (cd "$dist" && 7z a -tzip "$(basename "$stage").zip" "$(basename "$stage")" >/dev/null)
        rm -rf "$stage"
        echo "$stage.zip"
        ;;
    *)
        echo "unknown target $target" >&2
        exit 1
        ;;
esac
