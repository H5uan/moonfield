#!/bin/bash

# Xcode Project Generator for Moonfield
# This script automatically generates Xcode projects for development

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo -e "${GREEN}🚀 Moonfield Xcode Project Generator${NC}"
echo "Project root: ${PROJECT_ROOT}"

# Check if cargo-xcode is installed
if ! command -v cargo-xcode &> /dev/null; then
    echo -e "${YELLOW}⚠️  cargo-xcode is not installed. Installing...${NC}"
    cargo install cargo-xcode
fi

# Change to examples directory
cd "${PROJECT_ROOT}/examples"

echo -e "${GREEN}📱 Generating Xcode project...${NC}"

# Generate Xcode project
cargo xcode

# Move generated project to platform directory
if [ -d "examples.xcodeproj" ]; then
    echo -e "${GREEN}📁 Moving project to platform/xcode/...${NC}"
    
    # Remove existing project if it exists
    if [ -d "${PROJECT_ROOT}/platform/xcode/examples.xcodeproj" ]; then
        echo -e "${YELLOW}🗑️  Removing existing project...${NC}"
        rm -rf "${PROJECT_ROOT}/platform/xcode/examples.xcodeproj"
    fi
    
    mv examples.xcodeproj "${PROJECT_ROOT}/platform/xcode/"
fi

echo -e "${GREEN}✅ Xcode project generated successfully!${NC}"
echo -e "${YELLOW}📍 Project location: platform/xcode/examples.xcodeproj${NC}"
echo ""
echo "To open the project:"
echo "  open platform/xcode/examples.xcodeproj"
