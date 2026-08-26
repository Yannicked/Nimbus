# 🤖 AI Agent Guide: Nimbus

Welcome! This document provides a comprehensive developer and agent guide to the **Nimbus** codebase, detailing the architecture, directories, domain concepts, data pathways, and rules of engagement to help you understand and contribute to this repository efficiently.

---

## 📌 Project Overview

**Nimbus** is a real-time precipitation ensemble forecast, temperature, wind vector, and solar irradiance viewer for the Netherlands. It connects to the KNMI Open Data platform, ingests raw datasets (NetCDF and GRIB1), processes and caches them in memory, serves them via a high-performance Rust Axum HTTP API, and renders them interactively using WebGL and MapLibre GL JS on the frontend as well as in a companion Flutter mobile app.

Key performance optimizations in this repository:
1. **Lossless RG-Packed WebPs**: High-precision meteorological grid values (`u16` values) are packed into the Red and Green channels of WebP files. The browser loads these WebPs as textures and uses custom WebGL shaders to decode the values on the GPU with zero overhead.
2. **Precalculated Look-Up Tables (LUTs)**: Coordinate transformations (Mercator EPSG:3857 ↔ Polar Stereographic ↔ GRIB1 Lat/Lon) are heavy. The server precalculates bilinear interpolation LUTs on startup, enabling sub-millisecond multi-threaded projection using Rayon.
3. **GPU-Bound Wind Particle Simulation**: Wind particle trajectories and trails are simulated entirely on the GPU using Ping-Pong Framebuffer Objects (FBOs) and 12-bit coordinate packing.
4. **Push-Based MQTT Architecture**: Direct WebSocket subscriptions to KNMI MQTT notification channels ensure new radar files and model cycles are downloaded and precomputed immediately upon publication without polling.

---

## 🏗️ Architecture

```
                                ┌────────────────────────────────────────┐
                                │            KNMI MQTT Broker            │
                                └───────────────────┬────────────────────┘
                                                    │
                                         Push Notification Event
                                                    │
                                                    ▼
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Rust Axum Server Backend                                                                       │
│                                                                                                │
│  ┌────────────────────────┐      Downloads       ┌─────────────────┐                           │
│  │   MQTT Listeners       ├─────────────────────►│  Local Storage  │                           │
│  │  (rumqttc WebSockets)  │                      │  (.nc / .bin)   │                           │
│  └────────────────────────┘                      └────────┬────────┘                           │
│                                                           │                                    │
│                                                           ▼                                    │
│  ┌────────────────────────┐                      ┌─────────────────┐      Precalculation       │
│  │    Axum API Router     │◄─────────────────────┤   Data Cache    │◄─────────────────────────┐│
│  │  (Handlers & Endpoints)│                      │ (WebP / Grid)   │                          ││
│  └───────────▲────────────┘                      └─────────────────┘                          ││
│              │                                                                                ││
│              │ HTTP Requests                                                                  ││
└──────────────┼────────────────────────────────────────────────────────────────────────────────┼┘
               │
               │ (/api/metadata, /api/data/*, /api/timeseries, /api/value)
               │
               ▼
┌────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Client Visualisation Layer                                                                     │
│                                                                                                │
│  ┌────────────────────────┐      Load WebPs      ┌─────────────────┐                          ││
│  │   MapLibre GL JS /     ├─────────────────────►│  WebGL Textures │                          ││
│  │   Flutter Map View     │   (Decode RG to u16) └────────┬────────┘                          ││
│  └────────────────────────┘                               │                                   ││
│                                                           ▼                                   ││
│  ┌────────────────────────┐                      ┌─────────────────┐                          ││
│  │      Chart.js          │◄─────────────────────┤ WebGL Custom    ├──────────────────────────┘│
│  │   Timeseries Graph     │                      │ Shaders Layer   │                           │
│  └────────────────────────┘                      └─────────────────┘                           │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 📁 Project Directory Structure

```
Nimbus/
├── src/
│   ├── main.rs            # Server entrypoint, router configuration, background worker orchestration.
│   ├── constants.rs       # Grid dimensions, coordinates, scale factors, and dataset names.
│   ├── models.rs          # Data structures: metadata models, GRIB/NetCDF types, ensemble statistics.
│   ├── state.rs           # Shared thread-safe state: RwLocks and DashMaps for metadata, cache, and LUTs.
│   ├── handlers.rs        # Axum HTTP handlers: endpoint routing, query parsing, serving image/JSON data.
│   ├── radar.rs           # NetCDF parsing: reading slices, reducing ensembles, downloading files.
│   ├── harmonie.rs        # GRIB1 parsing: extracts temperature, wind, solar, and rain forecasts.
│   ├── rtcor.rs           # 5-minute real-time radar observation actuals (KNMI RTCOR).
│   ├── mqtt.rs            # MQTT clients: subscribes to KNMI topics for real-time notifications.
│   ├── projection.rs      # Math: Web Mercator (EPSG:3857) ↔ Polar Stereographic coordinate conversion.
│   ├── interpolation.rs   # Interpolation: Bilinear grid mapping and LUT initialization.
│   └── rendering.rs       # Image rendering: packing u16 grids into Red/Green channels of WebPs.
├── static/
│   ├── index.html         # Main client layout: HTML/CSS setup.
│   ├── style.css          # Glassmorphic dark styling.
│   └── src/               # Frontend scripts:
│       ├── config.js      # Map and Layer configurations.
│       ├── state.js       # Core state management for active index, layers, and settings.
│       ├── api.js         # HTTP API fetch client.
│       ├── main.js        # Entrypoint: sets up MapLibre, triggers loops, coordinates timeline events.
│       ├── map/
│       │   ├── index.js   # Texture caching and MapLibre custom layers management.
│       │   ├── WebGLRadar.js   # Custom WebGL layer rendering precipitation and temperature.
│       │   └── WebGLWind.js    # Custom WebGL layer managing GPU particle simulation.
│       └── ui/
│           ├── dom.js     # DOM element selectors.
│           ├── controls.js# Interactive controls (sliders, ensemble lists, play buttons).
│           └── chart.js   # Location-specific timeseries plot using Chart.js.
├── mobile/                # Cross-platform Flutter companion application (Android / iOS).
├── docs/                  # Screenshots and architectural diagrams.
├── Dockerfile             # Multi-stage production container build.
├── docker-compose.yml     # Orchestration configuration.
└── Cargo.toml             # Rust package configuration and dependencies.
```

---

## 🛰️ API Endpoints

The server listens on **`http://localhost:8080`**. Below is a summary of the backend endpoints:

### Precipitation Radar
| Endpoint | Description |
|---|---|
| `GET /api/metadata` | Dimensions, timestamps, and ensemble list for precipitation. |
| `GET /api/data/{ens}/{time}` | Serves lossless RG-packed radar WebP for the given ensemble/time step. |
| `GET /api/value` | Precipitation rate query at `lat`/`lon`/`time`/`ens` in mm/h. |
| `GET /api/timeseries` | Complete 6-hour precipitation forecast across all times at `lat`/`lon`. |

*Ensemble selectors (`ens`):* `med` (median), `max` (maximum), `prob` (probability of rain), or `1`–`20` (individual members).

### Temperature (Harmonie-AROME)
| Endpoint | Description |
|---|---|
| `GET /api/metadata/temp` | Dimensions and forecast timestamps for 2m temperature. |
| `GET /api/data/temp/{time}` | Lossless RG-packed temperature WebP. |
| `GET /api/value/temp` | Temperature query at `lat`/`lon`/`time` in Celsius. |
| `GET /api/timeseries/temp` | Complete 48-hour temperature forecast across all times at `lat`/`lon`. |

### Wind Vectors (Harmonie-AROME)
| Endpoint | Description |
|---|---|
| `GET /api/metadata/wind` | Dimensions, timestamps, and height levels for wind. |
| `GET /api/data/wind/{height}/{time}` | Lossless RG/BA-packed wind vector WebP (`u` in RG, `v` in BA). |
| `GET /api/value/wind` | Wind query (`u`, `v`, speed in m/s & Bft, direction) at `lat`/`lon`/`time`/`height`. |
| `GET /api/timeseries/wind` | Wind speed and direction forecast across all times at `lat`/`lon`. |

### Solar Radiation (Harmonie-AROME)
| Endpoint | Description |
|---|---|
| `GET /api/metadata/solar` | Dimensions and forecast timestamps for global radiation. |
| `GET /api/data/solar/{time}` | Lossless RG-packed solar irradiance WebP ($W/m^2$). |
| `GET /api/value/solar` | Solar irradiance query at `lat`/`lon`/`time`. |
| `GET /api/timeseries/solar` | 48-hour solar irradiance forecast across all times at `lat`/`lon`. |

---

## ⚙️ Core Technical Implementations

### 1. Lossless RG-Packed WebPs
The raw coordinates/wind/temperature values are stored as high-precision floats or integers. To avoid transporting bloated raw arrays, values are scaled into a `u16` space, split into high and low bytes, and stored in the Red and Green channels of a WebP:
* `pixel[0] = (val_raw >> 8) as u8` (Red = High Byte)
* `pixel[1] = (val_raw & 0xFF) as u8` (Green = Low Byte)
* `pixel[2] = 0` (Blue)
* `pixel[3] = 255` (Alpha)

In the client's WebGL fragment shaders, this value is unpacked:
```glsl
vec4 tex = texture2D(u_texture, clamped_coord);
float r = tex.r * 255.0;
float g = tex.g * 255.0;
float raw_val = r * 256.0 + g;
```

### 2. Packed Wind Vector WebPs
Wind has two vector components: `u` (zonal/eastward wind) and `v` (meridional/northward wind).
* The backend renders a single-height WebP of dimensions `GRID_W * GRID_H`.
* The `u` component is packed into the Red and Green channels (Red = High Byte, Green = Low Byte).
* The `v` component is packed into the Blue and Alpha channels (Blue = High Byte, Alpha = Low Byte).
* The frontend fragment and vertex shaders sample both components simultaneously in a single texture fetch, eliminating coordinate offsets.

### 3. GPU Wind Particle Simulation (`WebGLWind.js`)
To simulate thousands of wind particles without CPU bottlenecks, particles are simulated on the GPU using two textures in a Ping-Pong FBO configuration.
* **FBO Texture Dimensions**: `numParticles` (columns) $\times$ `trailLength` (rows).
* **Coordinate Packing (12-bit)**: Since a standard RGBA8 pixel only has 8 bits per channel, particle coordinates `(x, y)` are mapped `0..1` and packed with 12-bit precision across the RGB channels:
  * `x` is multiplied by `4095.0` $\rightarrow$ `x_hi` (stored in `R`), `x_lo` (stored in the upper 4 bits of `G`).
  * `y` is multiplied by `4095.0` $\rightarrow$ `y_hi` (stored in `B`), `y_lo` (stored in the lower 4 bits of `G`).
  * `age` of the particle is stored as a float in `A`.
* **Update Pass**: A fragment shader runs over the state texture. Row 0 (head particle) reads its previous state, samples the wind velocity texture, updates its position based on the vector `(u, v)`, increments its `age`, and writes the packed coordinates. All other rows `[1..trailLength-1]` copy the state of the row above them (`row - 1`), shifting the particle trail history.
* **Draw Pass**: The particle rendering vertex shader reads this state texture at coordinates specific to the particle and trail index, unpacks the position, converts the coordinates to Mercator, and projects them on the screen using `gl_Position`. Stationary particles are faded out using a `smoothstep` on wind speed.

---

## 🛠️ How to Build and Run

### Prerequisites
* **Rust (1.70+)**
* **KNMI Open Data API credentials**: Create a free developer account at [KNMI Developer Console](https://developer.dataplatform.knmi.nl/).
* **System Libraries**: NetCDF requires the HDF5 library.
  ```bash
  # Debian / Ubuntu
  sudo apt install libnetcdf-dev libhdf5-dev

  # macOS
  brew install netcdf hdf5
  ```

### Environment Variables
Configure your credentials in a `.env` file in the root directory:
```env
KNMI_OPEN_DATA_API_KEY="your_api_key_here"
KNMI_MQTT_PASSWORD="your_mqtt_password_here"
```

### Cargo Commands
* **Run Development Server**: `cargo run`
* **Build Optimized Release**: `cargo build --release`
* **Run Backend Tests**: `cargo test`

---

## 💡 Guidelines & Rules of Engagement

1. **Maintain Precalculated LUT Alignment**: If you modify target grid size (`GRID_W`/`GRID_H` in `constants.rs`), verify coordinate bindings. The projection lookup tables (`projection_lut`, `temp_projection_lut`, `wind_projection_lut`, `solar_projection_lut`) map exactly to these dimensions.
2. **Avoid Float-Sampling Artifacts in Shaders**: Both `WebGLRadar.js` and `WebGLWind.js` clamp texture coordinates to pixel centers:
   ```glsl
   vec2 clamped_coord = vec2(
       0.5 / 700.0 + v_texcoord.x * (699.0 / 700.0),
       0.5 / 765.0 + v_texcoord.y * (764.0 / 765.0)
   );
   ```
   Native GPU texture filtering (e.g. `GL_LINEAR`) would blur byte boundaries, corrupting unpacked values. Always set the texture filter parameters to `GL_NEAREST` when sampling raw packed data.
3. **Rust Concurrency & State**: Shared state uses a combination of `tokio::sync::RwLock` for state swapping (active NetCDF file or active metadata) and `dashmap::DashMap` for concurrent cache lookup. Always drop read locks before initiating heavy compute/background tasks to prevent deadlocks.
4. **Frontend Module Standards**: The frontend utilizes standard ES Modules. Do not introduce bundler build steps unless requested. Add modular code directly under `static/src/`.
