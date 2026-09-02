# Project: Nimbus Weather Intelligence Platform

## Architecture
Nimbus is a full-stack high-performance weather intelligence platform comprising:
- **Backend (Rust / Axum / Tokio / Rayon)**:
  - High-throughput meteorological data ingestion for NetCDF-4 (radar ensembles), GRIB1/Tar (Harmonie-AROME 2m temp, solar irradiance, precip, multi-level wind), and HDF5 (RTCOR 5-minute radar actuals).
  - Parallel statistical reductions (median, max, probability, spread, probability matched mean - PMM) via Rayon.
  - Lossless 16-bit WebP encoding with bilinear projection lookup tables (EPSG:3857 Web Mercator and Polar Stereographic).
  - Background MQTT event listeners on KNMI Data Platform with atomic staged swapping of cached forecast grids.
  - High-performance REST API with Axum exposing metadata, raw/processed tiles, point inspection values, and timeseries.
- **Frontend WebGL Rendering Engine (MapLibre GL JS + Custom GLSL)**:
  - Custom WebGL layers (`WebGLRadar.js`, `WebGLWind.js`) rendering precipitation, temperature, solar irradiance, spread, and 60fps GPU particle advection wind vectors using Ping-Pong FBOs.
  - Dual-texture temporal interpolation for continuous 60fps frame blending during timeline scrubbing.
  - High-precision GLSL fragment shaders handling 16-bit and 12-bit unpacked physical units.
  - Dynamic WebGL context loss recovery and bounded LRU texture memory management.
- **Frontend UI/UX & State Architecture**:
  - Modular vanilla ES module architecture (`static/src/`) without bundling overhead.
  - Centralized reactive state store synchronized with URL query parameters.
  - Split-screen swipe comparison mode with synced dual camera viewports.
  - Interactive location hover inspection and Chart.js forecast timeseries graphs with graceful nodata handling.
  - Standalone 4-quadrant analytics dashboard (`graphs.html`) with browser notification alerts.

---

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---|---|---|---|
| F01 | NetCDF-4 Ensemble Decoding & PMM Reductions | Robust ingestion of 20-member precipitation ensemble runs, calculating Median, Max, Prob, Spread, and PMM without panics. | M1 | Survey (Backend) |
| F02 | Harmonie GRIB1 Multi-Variable Extraction | Decompression of Tar/GRIB1 files, extracting Temp, Solar, Precip, and 5-level Wind (10m, 50m, 100m, 200m, 300m). | M1 | Survey (Backend) |
| F03 | RTCOR HDF5 Actuals Ingestion & History Bounding | Ingest 5-minute radar composites, South-Up row inversion, and strict 24-frame history retention with disk cleanup. | M1 | Survey (Backend) |
| F04 | Binary Cache Deserializer Memory Bounding | Validate magic bytes (`HRMT`, `HRW2`, `HRMS`, `HRMR`) and strictly bound `steps_len` allocations in `models.rs` to prevent OOM panics. | M1 | Survey (Backend) |
| F05 | MQTT Cancellation & Cold Boot Resilience | Wire atomic precalculation cancellation tracker in `mqtt.rs` and eliminate infinite blocking retry loops on cold startup. | M1 | Survey (Backend) |
| F06 | Async Worker Error Recovery & Zero Panics | Replace bare `.unwrap()` / `.expect()` in async worker tasks with recoverable logging to ensure 100% panic-free backend. | M1 | Survey (Backend) |
| F07 | Axum API Resilience & Error Boundaries | Defensive error responses for missing, out-of-grid, corrupted, or delayed dataset queries across all `/api/` routes. | M1 | Survey (Backend) |
| F08 | GLSL Shader Precision & Unpack Hardening | High-precision float declarations (`precision highp float`) in `WebGLRadar.js` and `WebGLWind.js` preventing quantization banding. | M2 | Survey (WebGL) |
| F09 | 60fps WebGL Render Loop Optimization | Cache uniform/attribute locations in `onAdd()` and eliminate per-frame allocations (`Float32Array`) in render passes. | M2 | Survey (WebGL) |
| F10 | WebGL Context Loss & Restoration Lifecycle | Implement `webglcontextlost` and `webglcontextrestored` listeners with `resetResources()` / `rebuildPrograms()` recovery. | M2 | Survey (WebGL) |
| F11 | Dual-Texture Temporal Blending | Implement GLSL linear blending between consecutive forecast frames for smooth timeline scrubbing and 60fps playback. | M2 | Survey (WebGL) |
| F12 | Bounded LRU GPU Texture Cache | Replace 250-item FIFO cache with true LRU cache bounded to 48 frames (desktop) / 24 frames (mobile) preventing VRAM OOM. | M2 | Survey (WebGL) |
| F13 | High-DPI Display Particle Scaling | Scale wind particle point size dynamically by `window.devicePixelRatio` for crisp rendering across Retina / 4K displays. | M2 | Survey (WebGL) |
| F14 | Interactive Map Controls & Layer Switching | Dynamic mode switching (Rain, Temp, Solar, Wind), sub-selectors, opacity/speed sliders, and split-screen comparison mode. | M3 | Survey (Frontend) |
| F15 | Timeline Scrubbing & Animation Controls | Dynamic timeline bounding, loop playback, forward preloading, and synchronized time index updates. | M3 | Survey (Frontend) |
| F16 | Location Analytics Charts & Nodata Handling | Hover inspection, Chart.js multi-parameter timeseries graphs, Beaufort wind formatting, and resilient nodata / out-of-grid fallbacks. | M3 | Survey (Frontend) |
| F17 | State Store & URL Synchronization | Centralized singleton state store, bi-directional URL parameter syncing, and live 5s model run polling. | M3 | Survey (Frontend) |
| F18 | UI Loading Indicators, Error Boundaries & Mobile Layout | Visual loading spinners, network delay indicators, error banners, and responsive mobile bottom-sheet styling. | M3 | Survey (Frontend) |
| F19 | Comprehensive Backend Unit & Integration Tests | Rust test suite covering binary cache corruption, PMM reductions, API edge cases, and continuous memory stability. | M4 | Survey (All) |
| F20 | Automated WebGL & Frontend Test Suite | Automated shader compilation test, 16-bit unpack verification, LRU eviction tests, and state/api resilience tests. | M4 | Survey (All) |
| F21 | E2E Test Suite 100% Pass & Adversarial Hardening | Pass 100% of the 4-tier E2E test suite published by the E2E Testing Track, followed by Tier 5 adversarial hardening. | M4 | Survey (All) |

---

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|---|---|---|---|
| M1 | Backend Pipeline Hardening & Memory Resilience | F01, F02, F03, F04, F05, F06, F07 | none | DONE |
| M2 | WebGL Rendering Engine & Shader Hardening | F08, F09, F10, F11, F12, F13 | none | DONE |
| M3 | UI/UX Interaction, State Management & Mobile UX | F14, F15, F16, F17, F18 | none | DONE |
| M4 | Full-Stack Verification, E2E Pass & Adversarial Hardening | F19, F20, F21 | M1, M2, M3 | DONE |

---

## Interface Contracts

### Backend API ↔ Frontend Web Client (`/api/*`)
- **Metadata Endpoints**:
  - `GET /api/metadata` -> JSON `{ run: string, times: string[], ensembles: string[], grid: { width, height, bbox } }`
  - `GET /api/metadata/{temp|wind|solar}` -> JSON `{ run: string, times: string[], heights?: string[], grid: ... }`
- **Tile Endpoints**:
  - `GET /api/data/{ens}/{time}` -> Image (image/webp, 16-bit packed RG channels: $R \times 256 + G = \text{val}$)
  - `GET /api/data/{temp|solar}/{time}`, `GET /api/data/wind/{height}/{time}` -> Image (image/webp)
  - Missing/out-of-bounds timestamp: HTTP 404 with JSON `{ error: "Not found" }` (graceful handling, never HTTP 500).
- **Point Value & Timeseries Endpoints**:
  - `GET /api/value?lat=..&lon=..&time=..` -> JSON `{ value: number | null, unit: string }`
  - `GET /api/timeseries?lat=..&lon=..` -> JSON `{ times: string[], values: (number | null)[] }`
  - Out of grid coordinates: returns `{ value: null }` or `{ values: [null, ...] }` with HTTP 200.

### WebGL Layer ↔ Texture Pipeline
- WebP RG channels: $R \in [0, 255], G \in [0, 255]$, physical unit unpack:
  $$\text{raw} = \text{floor}(R \times 255.0 + 0.5) \times 256.0 + \text{floor}(G \times 255.0 + 0.5)$$
- Uniforms for temporal blending: `u_texture_curr`, `u_texture_next`, `u_blend_factor` ($\in [0.0, 1.0]$).

---

## Code Layout
- `src/`: Backend Rust codebase
  - `src/main.rs`: Axum server bootstrap, CLI args, route binding
  - `src/handlers.rs`: REST API route handlers
  - `src/models.rs`: Domain data structures, binary serialization, ensemble reductions
  - `src/radar.rs`: NetCDF ingestion, ensemble precalculations, WebP encoding
  - `src/harmonie.rs`: GRIB1/Tar ingestion and extraction
  - `src/rtcor.rs`: HDF5 5-min radar actuals ingestion and row flipping
  - `src/mqtt.rs`: WebSocket TLS MQTT listeners
  - `src/state.rs`: Shared application state (`AppState`, `Arc<RwLock<...>>`)
  - `src/constants.rs`: Grid dimensions, bounding boxes, colormaps
- `static/`: Frontend web application
  - `static/index.html`, `static/style.css`: Main UI & design system
  - `static/graphs.html`, `static/graphs.css`: Standalone 4-quadrant forecast dashboard
  - `static/src/config.js`: Configuration constants
  - `static/src/state.js`: Singleton application state & URL sync
  - `static/src/api.js`: REST client
  - `static/src/main.js`: Main lifecycle & polling
  - `static/src/notifications.js`: Rain alerts & browser notifications
  - `static/src/graphs.js`: Dashboard charts & analytics
  - `static/src/map/index.js`: MapLibre integration & LRU texture cache
  - `static/src/map/WebGLRadar.js`: WebGL custom layer for radar/temp/solar
  - `static/src/map/WebGLWind.js`: WebGL custom particle advection layer
  - `static/src/ui/controls.js`: DOM controls, sliders, player loop, split-screen
  - `static/src/ui/chart.js`: Point inspection timeseries chart
- `tests/`: Integration & E2E test suites
