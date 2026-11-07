#!/bin/zsh
# Build a local, unsigned macOS application. No installation or existing data removal.
set -eu
ROOT="${0:A:h:h}"
cd "$ROOT"
cargo build --release --locked
APP="$ROOT/dist/FastDICOM.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ROOT/target/AppIcon.iconset"
# Synchronize only this generated resource directory; preserve all source studies.
SAMPLES="$ROOT/samples"
if [[ ! -d "$SAMPLES" || -L "$SAMPLES" ]]; then
  print -u2 "Missing samples directory: $SAMPLES"
  exit 1
fi
rsync -a --delete --exclude='.DS_Store' "$SAMPLES/" "$APP/Contents/Resources/samples/"
cp "$ROOT/target/aarch64-apple-darwin/release/fastdicom" "$APP/Contents/MacOS/FastDICOM.new"
mv -f "$APP/Contents/MacOS/FastDICOM.new" "$APP/Contents/MacOS/FastDICOM"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ROOT/assets/app-icon.png" --out "$ROOT/target/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$ROOT/assets/app-icon.png" --out "$ROOT/target/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ROOT/target/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/assets/Info.plist" "$APP/Contents/Info.plist"
# Finder/File Provider can attach metadata to Desktop bundles that codesign rejects.
# This touches only the generated app, never the source assets or study files.
xattr -cr "$APP"
codesign --force --deep --sign - "$APP"
# Refresh the bundle timestamp so Finder notices regenerated resources.
touch "$APP"
print "Built $APP"
