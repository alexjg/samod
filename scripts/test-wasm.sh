#!/bin/bash

# Script to run WASM tests for samod
# Builds the WASM package and runs Playwright tests

set -e

echo "Running samod WASM tests..."

# Navigate to wasm-tests directory
cd "$(dirname "$0")/../wasm-tests"

# Install dependencies if needed
if [ ! -d "node_modules" ]; then
  echo "Installing dependencies..."
  npm install
fi

# Build WASM package
echo "Building WASM package..."
npm run build-wasm

# Start the test server in WASM mode
echo "Starting test server in WASM mode..."
npm run server:stop
npm run server:start

# Wait for server to start
sleep 2

# Run Playwright tests
echo "Running Playwright tests..."
npm test

# Stop the server
echo "Stopping test server..."
npm run server:stop

echo "✅ WASM tests completed!"

