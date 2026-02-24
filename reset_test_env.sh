#!/bin/bash

# Script to reset test_env to default state (fresh jj repo, no power-workflow structure)

set -e

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Remove existing test_env if it exists
if [ -d "test_env" ]; then
    echo "Removing existing test_env..."
    rm -rf test_env
fi

# Create new test_env directory
echo "Creating fresh test_env..."
mkdir test_env
cd test_env

# Initialize a new jj repo
jj git init

# Create an initial commit
echo "initial content" > README.md
jj file track README.md
jj commit -m "Initial commit"

echo "test_env reset complete. Single workspace at root, ready for power-workflow testing."
