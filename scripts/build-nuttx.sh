#!/usr/bin/env sh
set -eu

REPOSITORY=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$REPOSITORY/firmware/nuttx/revisions.env"

case "${1:-profiles/nuttx.elf}" in
  /*) OUTPUT_ELF=${1:-profiles/nuttx.elf} ;;
  *) OUTPUT_ELF="$REPOSITORY/${1:-profiles/nuttx.elf}" ;;
esac

NUTTX_CONFIG=${2:-nsh}

for tool in git make clang ld.lld llvm-ar llvm-nm llvm-objcopy llvm-readelf \
  riscv64-elf-gcc
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
  if [ -x "$KCONFIG_TOOLS/menuconfig" ] && \
     [ -x "$KCONFIG_TOOLS/olddefconfig" ]
  then
    KCONFIG_FLAVOR=kconfiglib
  else
    KCONFIG_FLAVOR=frontends
  fi
elif command -v kconfig-conf >/dev/null 2>&1 && \
     command -v kconfig-tweak >/dev/null 2>&1
then
  KCONFIG_TOOLS=$(dirname -- "$(command -v kconfig-conf)")
  KCONFIG_FLAVOR=frontends
elif command -v menuconfig >/dev/null 2>&1 && \
     command -v olddefconfig >/dev/null 2>&1
then
  KCONFIG_TOOLS=$(dirname -- "$(command -v menuconfig)")
  KCONFIG_FLAVOR=kconfiglib
else
  echo "NuttX requires kconfig-frontends or Python kconfiglib; install one or set NUTTX_KCONFIG_BIN" >&2
  exit 1
fi

if [ "$KCONFIG_FLAVOR" = frontends ]
then
  for tool in kconfig-conf kconfig-tweak
  do
    [ -x "$KCONFIG_TOOLS/$tool" ] || {
      echo "NUTTX_KCONFIG_BIN is missing executable $tool: $KCONFIG_TOOLS" >&2
      exit 1
    }
  done
fi

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
  ./tools/configure.sh -a ../apps "compukter-vm:$NUTTX_CONFIG")

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

(cd "$BUILD_ROOT/nuttx" && PATH="$KCONFIG_TOOLS:$PATH" \
  make -j"${NUTTX_JOBS:-2}")

mkdir -p "$(dirname -- "$OUTPUT_ELF")"
cp "$BUILD_ROOT/nuttx/nuttx" "$OUTPUT_ELF"

elf_header=$(llvm-readelf -h "$OUTPUT_ELF")
echo "$elf_header" | grep -Fq 'Class:                             ELF32' || {
  echo "NuttX firmware is not ELF32" >&2
  exit 1
}
echo "$elf_header" | grep -Fq "Data:                              2's complement, little endian" || {
  echo "NuttX firmware is not little-endian" >&2
  exit 1
}
echo "$elf_header" | grep -Fq 'Machine:                           RISC-V' || {
  echo "NuttX firmware is not RISC-V" >&2
  exit 1
}
echo "$elf_header" | grep -Eq 'Entry point address:[[:space:]]+0x1000$' || {
  echo "NuttX firmware entry point is not 0x1000" >&2
  exit 1
}

program_headers=$(llvm-readelf -l "$OUTPUT_ELF")
if echo "$program_headers" | grep -E 'LOAD.*W.*E' >/dev/null
then
  echo "NuttX firmware contains a writable-executable PT_LOAD" >&2
  exit 1
fi

attributes=$(llvm-readelf -A "$OUTPUT_ELF")
if echo "$attributes" | grep -E 'Tag_RISCV_arch:.*rv32[^\"]*(_c|c[0-9])' >/dev/null
then
  echo "NuttX firmware unexpectedly requires compressed instructions" >&2
  exit 1
fi

if llvm-nm -u "$OUTPUT_ELF" | grep -E '[[:space:]]U[[:space:]]' >/dev/null
then
  echo "NuttX firmware contains strong unresolved symbols" >&2
  exit 1
fi

echo "Built Compukter NuttX firmware: $OUTPUT_ELF"
