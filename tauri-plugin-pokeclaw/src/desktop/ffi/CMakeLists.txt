# CMakeLists.txt — Build litertlm_shim as a shared library (litertlm_bridge)
#
# This CMake file builds the C++ shim that wraps LiteRT-LM's Engine/Conversation
# APIs behind extern "C" functions for Rust FFI via libloading.
#
# Prerequisites:
#   1. LiteRT-LM must be built from source via Bazel first (see build_litertlm.sh).
#      This produces the static libraries and headers in the LiteRT-LM source tree.
#   2. Set LITERTLM_SOURCE_DIR to the root of the LiteRT-LM repository.
#      Defaults to ../../../../third_party/LiteRT-LM (relative to this file).
#
# Usage (standalone):
#   mkdir build && cd build
#   cmake .. -DLITERTLM_SOURCE_DIR=/path/to/LiteRT-LM
#   cmake --build . --config Release
#
# The output shared library is named:
#   - litertlm_bridge.dll (Windows)
#   - liblitertlm_bridge.so (Linux)
#   - liblitertlm_bridge.dylib (macOS)

cmake_minimum_required(VERSION 3.16)
project(litertlm_bridge LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ---------------------------------------------------------------------------
# LiteRT-LM source directory (customizable)
# ---------------------------------------------------------------------------

if(NOT DEFINED LITERTLM_SOURCE_DIR)
  # Default: look for LiteRT-LM as a sibling dependency
  set(LITERTLM_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../../third_party/LiteRT-LM"
      CACHE PATH "Root of the LiteRT-LM source tree")
endif()

message(STATUS "LiteRT-LM source dir: ${LITERTLM_SOURCE_DIR}")

# ---------------------------------------------------------------------------
# Shim library
# ---------------------------------------------------------------------------

add_library(litertlm_bridge SHARED
  litertlm_shim.cpp
)

target_include_directories(litertlm_bridge PRIVATE
  ${LITERTLM_SOURCE_DIR}
  ${LITERTLM_SOURCE_DIR}/bazel-bin
  ${LITERTLM_SOURCE_DIR}/runtime
)

# ---------------------------------------------------------------------------
# Link against LiteRT-LM static libraries (built by Bazel)
# ---------------------------------------------------------------------------

# The Bazel build produces these static libraries. Exact paths may vary
# depending on the Bazel output layout. The build script (build_litertlm.sh)
# copies them to ${LITERTLM_SOURCE_DIR}/bazel-bin/lib/ for easy linking.
set(LITERTLM_LIB_DIR "${LITERTLM_SOURCE_DIR}/bazel-bin"
    CACHE PATH "Directory containing LiteRT-LM .lib/.a files")

# On Windows, we expect .lib files. On Linux/macOS, .a files.
if(WIN32)
  # Link against the main LiteRT-LM engine library (static)
  # The exact library name depends on the Bazel build target.
  # Typical: _deps/litert_lm/engine/libengine.lib or similar.
  #
  # For now, we declare these as optional — the build_litertlm.sh script
  # is responsible for placing the correct libraries in LITERTLM_LIB_DIR.
  find_library(LITERTLM_ENGINE_LIB
      NAMES engine litert_lm_engine litert_lm
      PATHS ${LITERTLM_LIB_DIR} ${LITERTLM_LIB_DIR}/runtime/engine
      NO_DEFAULT_PATH)
else()
  find_library(LITERTLM_ENGINE_LIB
      NAMES engine litert_lm_engine litert_lm
      PATHS ${LITERTLM_LIB_DIR} ${LITERTLM_LIB_DIR}/runtime/engine
      NO_DEFAULT_PATH)
endif()

if(LITERTLM_ENGINE_LIB)
  message(STATUS "Found LiteRT-LM engine library: ${LITERTLM_ENGINE_LIB}")
  target_link_libraries(litertlm_bridge PRIVATE ${LITERTLM_ENGINE_LIB})
else()
  message(WARNING
    "LiteRT-LM engine library not found in ${LITERTLM_LIB_DIR}. "
    "Build LiteRT-LM first with build_litertlm.sh, or set LITERTLM_LIB_DIR. "
    "The shim will build but linking will fail.")
endif()

# ---------------------------------------------------------------------------
# Platform-specific settings
# ---------------------------------------------------------------------------

if(WIN32)
  # Ensure proper DLL export/import
  target_compile_definitions(litertlm_bridge PRIVATE LITERTLM_EXPORTS)
  # Link common Windows system libraries
  target_link_libraries(litertlm_bridge PRIVATE
      ${LITERTLM_ENGINE_LIB}
      ws2_32 shlwapi)
elseif(APPLE)
  # macOS
  target_link_libraries(litertlm_bridge PRIVATE
      ${LITERTLM_ENGINE_LIB})
else()
  # Linux
  target_link_libraries(litertlm_bridge PRIVATE
      ${LITERTLM_ENGINE_LIB}
      pthread dl)
endif()

# ---------------------------------------------------------------------------
# Output directory
# ---------------------------------------------------------------------------

# Place the built shared library in a predictable location
set_target_properties(litertlm_bridge PROPERTIES
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/output"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/output"
    ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/output"
)

# Windows: copy the DLL next to the .lib for easy discovery
if(WIN32)
  set_target_properties(litertlm_bridge PROPERTIES
      LIBRARY_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/output"
      LIBRARY_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/output"
      RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/output"
      RUNTIME_OUTPUT_DIRECTORY_DEBUG "${CMAKE_BINARY_DIR}/output"
  )
endif()

# ---------------------------------------------------------------------------
# Suppress C++ exceptions and RTTI for smaller binary and C-compatible ABI
# ---------------------------------------------------------------------------

if(MSVC)
  target_compile_options(litertlm_bridge PRIVATE /EHs-c- /GR-)
else()
  target_compile_options(litertlm_bridge PRIVATE -fno-exceptions -fno-rtti)
endif()

# ---------------------------------------------------------------------------
# Install rules
# ---------------------------------------------------------------------------

install(TARGETS litertlm_bridge
    LIBRARY DESTINATION lib
    RUNTIME DESTINATION bin
    ARCHIVE DESTINATION lib)
install(FILES litertlm_shim.h DESTINATION include)
