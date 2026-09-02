#!/usr/bin/env bash
set -e

echo "================================================================="
echo "   Running Nimbus Weather Platform Full Test Suite (Cargo + Node)"
echo "================================================================="
echo ""

echo ">>> [1/2] Running Cargo Backend & Integration Tests..."
cargo test

echo ""
echo ">>> [2/2] Running Node.js WebGL & Client Test Harness..."
node tests/webgl/run_all.js

echo ""
echo "================================================================="
echo "   ✅ ALL TESTS PASSED SUCCESSFULLY (100% Pass Rate)"
echo "================================================================="
