#!/bin/sh
# Rebuild the complete LGPL component and offline self-test using bundled sources.
set -eu
SNES_SOURCE_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
SNES_OUTPUT_DIR=${1:-/tmp/tetorica-native-snes-rebuild}
mkdir -p "$SNES_OUTPUT_DIR"
SNES_DSP_DIR="$SNES_SOURCE_DIR/snapshot/third_party/snes-spc/snes_spc"
SNES_BRIDGE_DIR="$SNES_SOURCE_DIR/snapshot/application/src-tauri/src"
"${CXX:-c++}" -std=c++17 -O2 -fwrapv -fno-strict-aliasing -ffp-contract=off \
  -I"$SNES_DSP_DIR" -c "$SNES_DSP_DIR/SPC_DSP.cpp" -o "$SNES_OUTPUT_DIR/SPC_DSP.o"
"${CXX:-c++}" -std=c++17 -O2 -fwrapv -fno-strict-aliasing -ffp-contract=off \
  -I"$SNES_DSP_DIR" -c "$SNES_BRIDGE_DIR/native_genesis_snes.cpp" -o "$SNES_OUTPUT_DIR/native_genesis_snes.o"
for SPC_CPU_UNIT in SNES_SPC SNES_SPC_misc SNES_SPC_state; do
  "${CXX:-c++}" -std=c++17 -O2 -fwrapv -fno-strict-aliasing -ffp-contract=off \
    -I"$SNES_DSP_DIR" -c "$SNES_DSP_DIR/$SPC_CPU_UNIT.cpp" -o "$SNES_OUTPUT_DIR/$SPC_CPU_UNIT.o"
done
"${CXX:-c++}" -std=c++17 -O2 -fwrapv -fno-strict-aliasing -ffp-contract=off \
  -I"$SNES_DSP_DIR" -c "$SNES_BRIDGE_DIR/native_genesis_spc.cpp" -o "$SNES_OUTPUT_DIR/native_genesis_spc.o"
"${AR:-ar}" rcs "$SNES_OUTPUT_DIR/libtetorica_native_snes.a" \
  "$SNES_OUTPUT_DIR/native_genesis_snes.o" "$SNES_OUTPUT_DIR/native_genesis_spc.o" \
  "$SNES_OUTPUT_DIR/SNES_SPC.o" "$SNES_OUTPUT_DIR/SNES_SPC_misc.o" \
  "$SNES_OUTPUT_DIR/SNES_SPC_state.o" "$SNES_OUTPUT_DIR/SPC_DSP.o"
"${CXX:-c++}" -std=c++17 -O2 -I"$SNES_BRIDGE_DIR" \
  "$SNES_BRIDGE_DIR/native_genesis_snes_fixture.cpp" "$SNES_OUTPUT_DIR/libtetorica_native_snes.a" \
  -o "$SNES_OUTPUT_DIR/native-snes-fixture"
"$SNES_OUTPUT_DIR/native-snes-fixture" --self-test
"${CXX:-c++}" -std=c++17 -O2 -I"$SNES_BRIDGE_DIR" \
  "$SNES_BRIDGE_DIR/native_genesis_spc_fixture.cpp" "$SNES_OUTPUT_DIR/libtetorica_native_snes.a" \
  -o "$SNES_OUTPUT_DIR/native-spc-fixture"
"$SNES_OUTPUT_DIR/native-spc-fixture" --self-test
printf '%s\n' "Rebuilt $SNES_OUTPUT_DIR/libtetorica_native_snes.a and offline fixture"
