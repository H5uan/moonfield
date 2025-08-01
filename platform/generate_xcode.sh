#!/bin/bash

# Simple script to generate Xcode project for moonfield example binary
# This script wraps the cargo xcode command

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
XCODE_DIR="$SCRIPT_DIR/xcode"

echo "Generating Xcode project for moonfield example binary..."
echo "Project will be generated at: $XCODE_DIR/moonfield-window-example.xcodeproj"

# Change to xcode directory and generate project
cd "$XCODE_DIR"
cargo xcode --output-dir .

echo "Xcode project generated successfully!"
echo "To open the project, run:"
echo "open $XCODE_DIR/moonfield-window-example.xcodeproj"
