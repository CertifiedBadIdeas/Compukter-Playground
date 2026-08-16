#!/usr/bin/env sh
set -eu

REPOSITORY=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$REPOSITORY/firmware/nuttx/revisions.env"

for tool in git make clang ld.lld llvm-ar llvm-nm llvm-objcopy llvm-readelf
do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required NuttX build tool is unavailable: $tool" >&2
    exit 1
  }
done

OVERLAY_ROOT="$REPOSITORY/firmware/nuttx/overlay"
if [ ! -d "$OVERLAY_ROOT/nuttx" ] || [ ! -d "$OVERLAY_ROOT/apps" ]
then
  echo "Compukter NuttX overlay is incomplete: expected nuttx and apps trees" >&2
  exit 1
fi

if [ -n "${NUTTX_KCONFIG_BIN:-}" ]
then
  KCONFIG_TOOLS=$NUTTX_KCONFIG_BIN
elif command -v kconfig-conf >/dev/null 2>&1 && \
     command -v kconfig-tweak >/dev/null 2>&1
then
  KCONFIG_TOOLS=$(dirname -- "$(command -v kconfig-conf)")
else
  echo "NuttX requires kconfig-frontends; install it or set NUTTX_KCONFIG_BIN" >&2
  exit 1
fi

for tool in kconfig-conf kconfig-tweak
do
  [ -x "$KCONFIG_TOOLS/$tool" ] || {
    echo "NUTTX_KCONFIG_BIN is missing executable $tool: $KCONFIG_TOOLS" >&2
    exit 1
  }
done

SOURCE_CACHE="${TMPDIR:-/tmp}/compukter-playground-nuttx/sources"

verify_revision()
{
  source_path=$1
  expected_revision=$2
  source_name=$3
  actual_revision=$(git -C "$source_path" rev-parse HEAD 2>/dev/null) || {
    echo "$source_name source is not a Git checkout: $source_path" >&2
    exit 1
  }
  if [ "$actual_revision" != "$expected_revision" ]
  then
    echo "$source_name source revision mismatch: expected $expected_revision, got $actual_revision" >&2
    exit 1
  fi
  if ! git -C "$source_path" diff --quiet || \
     ! git -C "$source_path" diff --cached --quiet || \
     [ -n "$(git -C "$source_path" ls-files --others --exclude-standard)" ]
  then
    echo "$source_name source checkout is dirty: $source_path" >&2
    exit 1
  fi
}

acquire_source()
{
  repository_url=$1
  expected_revision=$2
  destination=$3
  source_name=$4
  if [ ! -d "$destination/.git" ]
  then
    mkdir -p "$(dirname -- "$destination")"
    git clone --filter=blob:none --no-checkout "$repository_url" "$destination"
  fi
  git -C "$destination" fetch --depth 1 origin "$expected_revision"
  git -C "$destination" checkout --detach "$expected_revision"
  verify_revision "$destination" "$expected_revision" "$source_name"
}

if [ -n "${NUTTX_SOURCE:-}" ]
then
  verify_revision "$NUTTX_SOURCE" "$NUTTX_REV" NuttX
else
  NUTTX_SOURCE="$SOURCE_CACHE/nuttx"
  acquire_source https://github.com/apache/nuttx.git "$NUTTX_REV" "$NUTTX_SOURCE" NuttX
fi

if [ -n "${NUTTX_APPS_SOURCE:-}" ]
then
  verify_revision "$NUTTX_APPS_SOURCE" "$NUTTX_APPS_REV" nuttx-apps
else
  NUTTX_APPS_SOURCE="$SOURCE_CACHE/apps"
  acquire_source https://github.com/apache/nuttx-apps.git "$NUTTX_APPS_REV" "$NUTTX_APPS_SOURCE" nuttx-apps
fi

BUILD_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/compukter-nuttx-build.XXXXXX")
trap 'rm -rf "$BUILD_ROOT"' EXIT HUP INT TERM

cp -a "$NUTTX_SOURCE/." "$BUILD_ROOT/nuttx"
cp -a "$NUTTX_APPS_SOURCE/." "$BUILD_ROOT/apps"
cp -a "$OVERLAY_ROOT/nuttx/." "$BUILD_ROOT/nuttx"
cp -a "$OVERLAY_ROOT/apps/." "$BUILD_ROOT/apps"

if [ -f "$REPOSITORY/firmware/nuttx/patches/nuttx-kconfig.patch" ]
then
  git -C "$BUILD_ROOT/nuttx" apply --check \
    "$REPOSITORY/firmware/nuttx/patches/nuttx-kconfig.patch"
  git -C "$BUILD_ROOT/nuttx" apply \
    "$REPOSITORY/firmware/nuttx/patches/nuttx-kconfig.patch"
fi

(cd "$BUILD_ROOT/nuttx" && PATH="$KCONFIG_TOOLS:$PATH" \
  ./tools/configure.sh -a ../apps compukter-vm:nsh)

for expected in \
  'CONFIG_ARCH_CHIP_COMPUKTER=y' \
  'CONFIG_ARCH_CHIP="compukter"' \
  'CONFIG_ARCH_BOARD_COMPUKTER_VM=y' \
  'CONFIG_ARCH_BOARD="compukter-vm"'
do
  grep -Fqx "$expected" "$BUILD_ROOT/nuttx/.config" || {
    echo "generated NuttX configuration is missing: $expected" >&2
    exit 1
  }
done

echo "Compukter NuttX platform configuration generated successfully"
