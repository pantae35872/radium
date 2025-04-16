#!/bin/bash

set -e

KERNEL_BIN=$1
BUILD_DIR=../../build

cp "$KERNEL_BIN" "$BUILD_DIR/kernel.bin"
cd ../.. && make test-run
