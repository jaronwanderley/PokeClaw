#!/usr/bin/env bash
# build_litertlm.sh — Orchestrate Bazel build of LiteRT-LM + CMake build of the FFI shim.
#
# Prerequisites:
#   - Bazel (https://bazel.build/install)
#   - CMake >= 3.16
#   - C++17 compiler (MSVC on Windows, GCC/Clang on Linux/macOS)
#   - Git
#
# Usage:
#   ./build_litertlm.sh                          # Build everything
#   ./build_litertlm.sh --litertlm-dir /path/to  # Specify LiteRT-LM source
#   ./build_litertlm.sh --skip-bazel              # Skip Bazel build (if already built)
#   ./build_litertlm.sh --clean                   # Clean and rebuild
#
# The output shared library ends up at:
#   tauri-plugin-pokeclaw/src/desktop/ffi/build/output/litertlm_bridge.dll (Windows)
#   tauri-plugin-pokeclaw/src/desktop/ffi/build/output/liblitertlm_bridge.so (Linux)
#   tauri-plugin-pokeclaw/src/desktop/ffi/build/output/liblitertlm_bridge.dylib (macOS)

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROJECT_ROOT="$(cd "${PLUGIN_DIR}/../.." && pwd)"

# Default LiteRT-LM source location
DEFAULT_LITERTLM_DIR="${PROJECT_ROOT}/third_party/LiteRT-LM"

LITERTLM_DIR="${DEFAULT_LITERTLM_DIR}"
SKIP_BAZEL=false
CLEAN=false
BUILD_TYPE="Release"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
  case "$1" in
    --litertlm-dir)
      LITERTLM_DIR="$2"
      shift 2
      ;;
    --skip-bazel)
      SKIP_BAZEL=true
      shift
      ;;
    --clean)
      CLEAN=true
      shift
      ;;
    --debug)
      BUILD_TYPE="Debug"
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --litertlm-dir DIR  Path to LiteRT-LM source (default: ${DEFAULT_LITERTLM_DIR})"
      echo "  --skip-bazel        Skip Bazel build (assume already built)"
      echo "  --clean             Clean and rebuild from scratch"
      echo "  --debug             Build in Debug mode (default: Release)"
      echo "  -h, --help          Show this help"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Validate environment
# ---------------------------------------------------------------------------

echo "=== build_litertlm.sh ==="
echo "LiteRT-LM source: ${LITERTLM_DIR}"
echo "Plugin dir:       ${PLUGIN_DIR}"
echo "Project root:     ${PROJECT_ROOT}"
echo "Build type:       ${BUILD_TYPE}"
echo "Skip Bazel:       ${SKIP_BAZEL}"
echo ""

command -v cmake >/dev/null 2>&1 || { echo "ERROR: cmake not found. Install CMake >= 3.16." >&2; exit 1; }

# ---------------------------------------------------------------------------
# Step 1: Clone LiteRT-LM if needed
# ---------------------------------------------------------------------------

if [[ ! -d "${LITERTLM_DIR}" ]]; then
  echo ">>> LiteRT-LM source not found at ${LITERTLM_DIR}"
  echo ">>> Cloning from https://github.com/google-ai-edge/LiteRT-LM ..."
  mkdir -p "$(dirname "${LITERTLM_DIR}")"
  git clone --depth 1 https://github.com/google-ai-edge/LiteRT-LM.git "${LITERTLM_DIR}"
  echo ">>> Clone complete."
else
  echo ">>> LiteRT-LM source found at ${LITERTLM_DIR}"
fi

# ---------------------------------------------------------------------------
# Step 2: Bazel build of LiteRT-LM
# ---------------------------------------------------------------------------

if [[ "${SKIP_BAZEL}" == "false" ]]; then
  echo ""
  echo ">>> Step 2: Building LiteRT-LM with Bazel ..."

  command -v bazel >/dev/null 2>&1 || { echo "ERROR: bazel not found. Install Bazel." >&2; exit 1; }

  pushd "${LITERTLM_DIR}" >/dev/null

  # Build the main engine target and the CLI binary (which pulls in all deps)
  # The //runtime/engine:engine target builds the core engine library.
  echo ">>> Building //runtime/engine:litert_lm_main ..."
  bazel build -c opt //runtime/engine:litert_lm_main

  echo ">>> Bazel build complete."
  popd >/dev/null
else
  echo ""
  echo ">>> Step 2: Skipping Bazel build (--skip-bazel)"
fi

# ---------------------------------------------------------------------------
# Step 3: CMake build of the FFI shim
# ---------------------------------------------------------------------------

SHIM_DIR="${PLUGIN_DIR}/src/desktop/ffi"
BUILD_DIR="${SHIM_DIR}/build"

echo ""
echo ">>> Step 3: Building litertlm_bridge shim with CMake ..."

if [[ "${CLEAN}" == "true" ]]; then
  echo ">>> Cleaning build directory: ${BUILD_DIR}"
  rm -rf "${BUILD_DIR}"
fi

mkdir -p "${BUILD_DIR}"
pushd "${BUILD_DIR}" >/dev/null

# Configure
cmake "${SHIM_DIR}" \
  -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
  -DLITERTLM_SOURCE_DIR="${LITERTLM_DIR}"

# Build
cmake --build . --config "${BUILD_TYPE}" -- -j"$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"

echo ">>> CMake build complete."
popd >/dev/null

# ---------------------------------------------------------------------------
# Step 4: Verify output
# ---------------------------------------------------------------------------

OUTPUT_DIR="${BUILD_DIR}/output"

echo ""
echo ">>> Step 4: Verifying output ..."

if [[ -f "${OUTPUT_DIR}/litertlm_bridge.dll" ]] || \
   [[ -f "${OUTPUT_DIR}/liblitertlm_bridge.so" ]] || \
   [[ -f "${OUTPUT_DIR}/liblitertlm_bridge.dylib" ]]; then
  echo ">>> SUCCESS: Shared library found in ${OUTPUT_DIR}/"
  ls -la "${OUTPUT_DIR}/"litertlm_bridge.* "${OUTPUT_DIR}/"liblitertlm_bridge.* 2>/dev/null || true
else
  echo ">>> WARNING: Shared library not found in ${OUTPUT_DIR}/"
  echo ">>> Check CMake output above for errors."
  exit 1
fi

echo ""
echo "=== build_litertlm.sh complete ==="
