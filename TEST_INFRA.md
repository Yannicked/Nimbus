# E2E Test Infra: Nimbus Weather Intelligence Platform

## Test Philosophy
- **Opaque-box & Requirement-driven**: Derived directly from `ORIGINAL_REQUEST.md` specifications and user-facing REST API / WebGL contracts.
- **Methodology**: 4-tier systematic verification (Category-Partition, Boundary Value Analysis, Pairwise Combinatorial Testing, Real-World Workload Testing) followed by Tier 5 white-box adversarial stress-testing.
- **Progressive Testability**: Tier 1 tests verify basic happy paths; higher tiers verify boundaries, pairwise feature interactions, and real-world complex scenarios.

---

## Feature Inventory & Test Matrix
| # | Feature | Requirement | Tier 1 (Feature) | Tier 2 (Boundary) | Tier 3 (Pairwise) |
|---|---|---|:---:|:---:|:---:|
| F01 | NetCDF-4 Ensemble Decoding & PMM Reductions | R1, R4 | 5 | 5 | ✓ |
| F02 | Harmonie GRIB1 Multi-Variable Extraction | R1, R4 | 5 | 5 | ✓ |
| F03 | RTCOR HDF5 Actuals Ingestion & History Bounding | R1, R4 | 5 | 5 | ✓ |
| F04 | Binary Cache Deserializer Memory Bounding | R1, R4 | 5 | 5 | ✓ |
| F05 | MQTT Cancellation & Cold Boot Resilience | R1, R4 | 5 | 5 | ✓ |
| F06 | Async Worker Error Recovery & Zero Panics | R1, R4 | 5 | 5 | ✓ |
| F07 | Axum API Resilience & Error Boundaries | R1, R4 | 5 | 5 | ✓ |
| F08 | GLSL Shader Precision & Unpack Hardening | R2, R4 | 5 | 5 | ✓ |
| F09 | 60fps WebGL Render Loop Optimization | R2, R4 | 5 | 5 | ✓ |
| F10 | WebGL Context Loss & Restoration Lifecycle | R2, R4 | 5 | 5 | ✓ |
| F11 | Dual-Texture Temporal Blending | R2, R4 | 5 | 5 | ✓ |
| F12 | Bounded LRU GPU Texture Cache | R2, R4 | 5 | 5 | ✓ |
| F13 | High-DPI Display Particle Scaling | R2, R4 | 5 | 5 | ✓ |
| F14 | Interactive Map Controls & Layer Switching | R3, R4 | 5 | 5 | ✓ |
| F15 | Timeline Scrubbing & Animation Controls | R3, R4 | 5 | 5 | ✓ |
| F16 | Location Analytics Charts & Nodata Handling | R3, R4 | 5 | 5 | ✓ |
| F17 | State Store & URL Synchronization | R3, R4 | 5 | 5 | ✓ |
| F18 | UI Loading Indicators, Error Boundaries & Mobile Layout | R3, R4 | 5 | 5 | ✓ |

---

## Real-World Application Scenarios (Tier 4)
| # | Scenario | Features Exercised | Complexity |
|---|---|---|---|
| S1 | Extreme Convective Storm Event (Rapid Rain & Wind Shift) | F01, F02, F03, F08, F11, F14, F15, F16 | High |
| S2 | Multi-Model Run Transition During Active User Scrubbing | F05, F07, F10, F11, F12, F15, F17 | High |
| S3 | Mobile Low-Memory Device Extended Exploration & Compare Mode | F09, F10, F12, F13, F14, F18 | High |
| S4 | Offline / Corrupted Data Network Interruption & Recovery | F04, F05, F06, F07, F16, F18 | High |
| S5 | High-Resolution Solar Radiation & 2m Temperature Spatial Inspection | F02, F07, F08, F14, F16, F17 | Medium |

---

## Test Architecture & Directory Layout
- `tests/e2e/`: End-to-end integration and simulation tests
  - `tests/e2e/api_resilience_test.rs`: API endpoint error boundaries, missing/delayed payload tests
  - `tests/e2e/binary_deserialization_test.rs`: Corrupted binary cache bounds and OOM prevention tests
  - `tests/e2e/ensemble_pmm_test.rs`: PMM reductions across edge values and single-cell cases
  - `tests/e2e/memory_bounding_test.rs`: Continuous ingestion memory leak and cache eviction simulation
- `tests/webgl/`: Automated WebGL, shader compilation, and client-side unit tests (Node.js runner script or Rust test harness executing headless verification)
  - `tests/webgl/shader_compilation_test.js`: Validates all vertex, fragment, and particle shaders syntax and precision
  - `tests/webgl/unpack_precision_test.js`: Verifies exact lossless 16-bit and 12-bit coordinate decoding without precision drop
  - `tests/webgl/lru_cache_test.js`: Verifies strict LRU eviction bounds and memory cleanup

---

## Coverage Thresholds
- **Tier 1 (Feature Coverage)**: $\ge 5 \times 18 = 90$ test assertions/cases covering all features in isolation.
- **Tier 2 (Boundary & Corner Cases)**: $\ge 5 \times 18 = 90$ test cases testing extremes, bounds, missing fields, zero/overflow.
- **Tier 3 (Cross-Feature Combinations)**: $\ge 18$ pairwise integration tests.
- **Tier 4 (Real-World Application Scenarios)**: $\ge 5$ end-to-end scenario simulations.
- **Tier 5 (Adversarial Stress Testing)**: Automated white-box edge case generation.
