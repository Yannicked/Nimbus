// App Configuration
const CONFIG = {
    // Map settings
    map: {
        defaultCenter: [5.2, 52.1], // Center on Netherlands (lon, lat for MapLibre!)
        defaultZoom: 7, // MapLibre zoom 7 roughly matches Leaflet zoom 8
        minZoom: 0,
        maxZoom: 14,
        // Standard CartoDB vector basemaps (Dark Matter and Voyager)
        styles: {
            dark: 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json',
            osm: 'https://basemaps.cartocdn.com/gl/voyager-gl-style/style.json'
        }
    },
    // Application defaults & behavior
    defaults: {
        ensemble: 'med', // Default selected ensemble/statistic ('med', 'max', 'prob', or ensemble member number)
        opacity: 70, // Default radar layer opacity (%)
        speed: 3, // Default playback speed (fps)
        timeIndex: 0 // Default starting time index
    },
    // Timing and performance
    intervals: {
        metadataPollingMs: 5000, // Metadata check interval
        hoverThrottleMs: 100, // Hover values API query throttle
    },
    // Sliding-window layer cache config
    cache: {
        preloadAhead: 2, // Number of future frames to pre-load
    },
    // Legend and visualization colors
    radarVisualization: {
        prob: {
            title: "Rain Probability",
            colors: [
                "rgba(180, 200, 220, 0.0)",
                "rgba(100, 160, 255, 0.5)",
                "rgba(0, 100, 255, 0.65)",
                "rgba(0, 200, 100, 0.75)",
                "rgba(220, 0, 220, 0.85)",
                "rgba(255, 255, 255, 0.95)"
            ],
            labels: ["10%", "30%", "50%", "70%", "90%", "100%"]
        },
        rate: {
            title: "Rainfall Rate (mm/h)",
            colors: [
                "rgba(120, 200, 255, 0.0)",
                "rgba(0, 100, 255, 0.7)",
                "rgba(0, 200, 0, 0.7)",
                "rgba(255, 230, 0, 0.8)",
                "rgba(255, 120, 0, 0.9)",
                "rgba(255, 0, 0, 0.95)",
                "rgba(200, 0, 200, 1.0)",
                "rgba(255, 255, 255, 1.0)"
            ],
            labels: ["0.05", "0.2", "1", "5", "15", "30", "100", "250+"]
        }
    },
    // Chart options
    chart: {
        tension: 0.3,
        borderWidth: 2,
        pointRadius: 0,
        pointHoverRadius: 4,
        maxTicksLimit: 6,
        colors: {
            prob: {
                border: "#a855f7",
                background: "rgba(168, 85, 247, 0.15)"
            },
            rate: {
                border: "#38bdf8",
                background: "rgba(56, 189, 248, 0.15)"
            }
        }
    }
};

// App State
let map;
let metadata = null;
let rainMetadata = null;
let tempMetadata = null;
let windMetadata = null;
let currentLayerMode = 'rain';
let currentEns = CONFIG.defaults.ensemble;
let currentTimeIndex = CONFIG.defaults.timeIndex;
let isPlaying = false;
let playInterval = null;
let clickedMarker = null;
let chartInstance = null;
let activeCoords = null;

// WebGL Custom Layer variables
let radarProgram = null;
let positionBuffer = null;
let texcoordBuffer = null;
let glContext = null;
let textureCache = {};

// WebGL Wind Layer variables
let windProgram = null;
let windPositionBuffer = null;
let windTexcoordBuffer = null;
let particleProgram = null;
let particleBuffer = null;
let windPixelData = null; // Uint8ClampedArray for CPU particle lookups
const maxParticles = 3000;
const TRAIL_LENGTH = 24;
let particles = [];
let lastAnimTime = 0;

// DOM Elements
const refTimeVal = document.getElementById('ref-time-value');
const currentTimeStep = document.getElementById('current-time-step');
const timeStepRelative = document.getElementById('time-step-relative');
const timeSlider = document.getElementById('time-slider');
const btnPlay = document.getElementById('btn-play');
const btnPrev = document.getElementById('btn-prev');
const btnNext = document.getElementById('btn-next');
const speedSlider = document.getElementById('speed-slider');
const speedValue = document.getElementById('speed-value');
const opacitySlider = document.getElementById('opacity-slider');
const opacityValue = document.getElementById('opacity-value');
const ensembleSelect = document.getElementById('ensemble-select');
const hoverPanel = document.getElementById('hover-panel');
const hoverValue = document.getElementById('hover-value');
const hoverCoords = document.getElementById('hover-coords');
const chartPanel = document.getElementById('chart-panel');
const chartCloseBtn = document.getElementById('chart-close');
const chartCoords = document.getElementById('chart-coords');
const chartStatPeak = document.getElementById('chart-stat-peak');
const chartStatTotal = document.getElementById('chart-stat-total');
const themeSelect = document.getElementById('theme-select');
const btnSettingsToggle = document.getElementById('btn-settings-toggle');
const settingsContent = document.getElementById('settings-content');

// Web Mercator to Lat/Lon Projection
function mercatorToLonLat(x, y) {
    const r_major = 6378137.0;
    const lon = (x / r_major) * (180.0 / Math.PI);
    const lat = (2.0 * Math.atan(Math.exp(y / r_major)) - Math.PI / 2.0) * (180.0 / Math.PI);
    return [lat, lon];
}

// Lat/Lon to Web Mercator Projection
function lonLatToMercator(lat, lon) {
    const r_major = 6378137.0;
    const x = lon * (Math.PI / 180.0) * r_major;
    const y = Math.log(Math.tan((Math.PI / 4.0) + (lat * (Math.PI / 360.0)))) * r_major;
    return [x, y];
}

// Update Wind Pixel Data cache from loaded PNG image
function updateWindPixelData(img) {
    console.log("Extracting wind pixel data for CPU simulation...");
    const canvas = document.createElement('canvas');
    canvas.width = 700;
    canvas.height = 1530;
    const ctx = canvas.getContext('2d');
    ctx.drawImage(img, 0, 0);
    const imageData = ctx.getImageData(0, 0, 700, 1530);
    windPixelData = imageData.data;
}

// Sample u and v velocities at a Mercator coordinate
function getWindVelocity(mx, my) {
    if (!windPixelData) return [0, 0];
    
    // Bounding box: MERCATOR_LEFT: 0.0, MERCATOR_RIGHT: 1210000.0, MERCATOR_BOTTOM: 6250000.0, MERCATOR_TOP: 7560000.0
    const col = Math.floor((mx - 0.0) / 1210000.0 * 700);
    const row = Math.floor((7560000.0 - my) / (7560000.0 - 6250000.0) * 765);
    
    if (col < 0 || col >= 700 || row < 0 || row >= 765) {
        return [0, 0];
    }
    
    // Sample u (top half)
    const idx_u = (row * 700 + col) * 4;
    const r_u = windPixelData[idx_u];
    const g_u = windPixelData[idx_u + 1];
    const raw_u = r_u * 256 + g_u;
    if (raw_u >= 65535 || raw_u === 0) return [0, 0];
    const u = raw_u / 100.0 - 100.0;
    
    // Sample v (bottom half)
    const idx_v = (((row + 765) * 700) + col) * 4;
    const r_v = windPixelData[idx_v];
    const g_v = windPixelData[idx_v + 1];
    const raw_v = r_v * 256 + g_v;
    if (raw_v >= 65535 || raw_v === 0) return [0, 0];
    const v = raw_v / 100.0 - 100.0;
    
    return [u, v];
}

// Convert wind speed to Beaufort scale
function mpsToBeaufort(mps) {
    if (mps < 0.3) return 0;
    if (mps < 1.6) return 1;
    if (mps < 3.4) return 2;
    if (mps < 5.5) return 3;
    if (mps < 8.0) return 4;
    if (mps < 10.8) return 5;
    if (mps < 13.9) return 6;
    if (mps < 17.2) return 7;
    if (mps < 20.8) return 8;
    if (mps < 24.5) return 9;
    if (mps < 28.5) return 10;
    if (mps < 32.7) return 11;
    return 12;
}

// Convert wind direction in degrees to cardinal direction
function degreesToCardinal(deg) {
    const index = Math.round(deg / 45) % 8;
    const cardinals = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    return cardinals[index];
}

// Generate a random particle
function randomParticle() {
    const mx = Math.random() * 1210000.0;
    const my = 6250000.0 + Math.random() * (7560000.0 - 6250000.0);
    const maxAge = 150 + Math.random() * 150;
    const age = Math.random() * maxAge;
    const history = [];
    for (let i = 0; i < TRAIL_LENGTH; i++) {
        history.push({ mx: mx, my: my });
    }
    return {
        mx: mx,
        my: my,
        age: age,
        maxAge: maxAge,
        history: history,
        activeLength: 1,
        lastBreadcrumb: { mx: mx, my: my }
    };
}

// Initialize the particle list
function initParticles() {
    particles = [];
    for (let i = 0; i < maxParticles; i++) {
        particles.push(randomParticle());
    }
}

// Update particle positions based on wind velocities
function updateParticles(dt, minDistance) {
    const speedFactor = 2.5; // Controls the movement speed of particles
    
    for (let i = 0; i < particles.length; i++) {
        const p = particles[i];
        p.age += dt * 60; // Age in frames
        
        if (p.age >= p.maxAge) {
            particles[i] = randomParticle();
            particles[i].age = 0; // Reborn particles start at age 0 to fade in smoothly
            continue;
        }
        
        const [u, v] = getWindVelocity(p.mx, p.my);
        
        // Update positions using velocity (meters per second)
        p.mx += u * dt * speedFactor * 1200.0;
        p.my += v * dt * speedFactor * 1200.0;
        
        // Bounds checking
        if (p.mx < 0.0 || p.mx > 1210000.0 || p.my < 6250000.0 || p.my > 7560000.0) {
            particles[i] = randomParticle();
            particles[i].age = 0; // Reborn particles start at age 0 to fade in smoothly
            continue;
        }
        
        // Overwrite the head position to the current position
        p.history[0] = { mx: p.mx, my: p.my };
        
        // Push a new trail point if the head has moved far enough from the last recorded breadcrumb
        const dx = p.mx - p.lastBreadcrumb.mx;
        const dy = p.my - p.lastBreadcrumb.my;
        const dist = Math.sqrt(dx * dx + dy * dy);
        
        if (dist >= minDistance) {
            p.history.splice(1, 0, { mx: p.mx, my: p.my });
            p.activeLength = Math.min(p.activeLength + 1, TRAIL_LENGTH);
            p.lastBreadcrumb = { mx: p.mx, my: p.my };
            if (p.history.length > TRAIL_LENGTH) {
                p.history.pop();
            }
        }
        
        // Collapse unused history points to the head position so they don't form a dot at the start
        for (let j = p.activeLength; j < TRAIL_LENGTH; j++) {
            p.history[j] = { mx: p.mx, my: p.my };
        }
    }
}

// Format relative time step
function formatRelativeTime(seconds) {
    const mins = Math.round(seconds / 60);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    if (h > 0) {
        return `+${h}h ${m.toString().padStart(2, '0')}m`;
    }
    return `+${m}`;
}

// Format absolute forecast time
function formatAbsoluteTime(refTimeStr, secondsOffset) {
    // Expected refTimeStr format: "seconds since YYYY-MM-DD HH:MM:SS"
    const match = refTimeStr.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
    if (!match) return `+${Math.round(secondsOffset / 60)} mins`;
    
    const refDate = new Date(`${match[1]}T${match[2]}Z`); // UTC parsed
    const targetDate = new Date(refDate.getTime() + secondsOffset * 1000);
    
    return targetDate.toLocaleString('en-GB', {
        timeZone: 'Europe/Amsterdam',
        day: '2-digit',
        month: 'short',
        hour: '2-digit',
        minute: '2-digit',
        hour12: false
    });
}

// Initialize MapLibre Map
function initMap() {
    // Parse URL query parameters for initial viewport
    const urlParams = new URLSearchParams(window.location.search);
    const initialLat = parseFloat(urlParams.get('lat'));
    const initialLon = parseFloat(urlParams.get('lon'));
    const initialZoom = parseFloat(urlParams.get('zoom'));

    const center = (!isNaN(initialLat) && !isNaN(initialLon)) 
        ? [initialLon, initialLat] // lon, lat for MapLibre!
        : CONFIG.map.defaultCenter;

    const zoom = !isNaN(initialZoom) 
        ? initialZoom 
        : CONFIG.map.defaultZoom;

    map = new maplibregl.Map({
        container: 'map',
        style: CONFIG.map.styles.dark,
        center: center,
        zoom: zoom,
        minZoom: CONFIG.map.minZoom,
        maxZoom: CONFIG.map.maxZoom
    });

    // Add map navigation controls (zoom, compass)
    map.addControl(new maplibregl.NavigationControl(), 'top-left');

    map.on('load', () => {
        setupRadarSourceAndLayer();
    });

    // Recreate custom WebGL layer on style changes
    map.on('style.load', () => {
        setupRadarSourceAndLayer();
    });

    // Attach map mouse events for hover & click
    map.on('mousemove', handleMapMouseMove);
    map.on('mouseout', handleMapMouseLeave);
    map.on('click', handleMapClick);

    // Sync viewport state to URL query parameters
    map.on('moveend', () => {
        const center = map.getCenter();
        const zoom = map.getZoom();
        
        const url = new URL(window.location.href);
        url.searchParams.set('lat', center.lat.toFixed(4));
        url.searchParams.set('lon', center.lng.toFixed(4));
        url.searchParams.set('zoom', zoom.toFixed(1));
        
        window.history.replaceState({}, '', url.pathname + url.search);
    });
}

// Fetch Metadata and Load App
async function loadApp() {
    try {
        // Fetch rain metadata
        const responseRain = await fetch('/api/metadata');
        if (!responseRain.ok) throw new Error("Rain metadata request failed");
        rainMetadata = await responseRain.json();
        
        // Fetch temp metadata
        const responseTemp = await fetch('/api/metadata/temp');
        if (!responseTemp.ok) throw new Error("Temp metadata request failed");
        tempMetadata = await responseTemp.json();

        // Fetch wind metadata
        const responseWind = await fetch('/api/metadata/wind');
        if (!responseWind.ok) throw new Error("Wind metadata request failed");
        windMetadata = await responseWind.json();
        
        // Default active metadata
        if (currentLayerMode === 'temp') {
            metadata = tempMetadata;
        } else if (currentLayerMode === 'wind') {
            metadata = windMetadata;
        } else {
            metadata = rainMetadata;
        }
        
        // Find index closest to current system time
        const refMatch = metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
        let refTimeMs = Date.now();
        if (refMatch) {
            refTimeMs = new Date(`${refMatch[1]}T${refMatch[2]}Z`).getTime();
        }
        const targetOffset = (Date.now() - refTimeMs) / 1000;
        let closestIndex = 0;
        let minDiff = Infinity;
        for (let i = 0; i < metadata.times.length; i++) {
            const diff = Math.abs(metadata.times[i] - targetOffset);
            if (diff < minDiff) {
                minDiff = diff;
                closestIndex = i;
            }
        }
        currentTimeIndex = closestIndex;
        
        // Display reference time
        refTimeVal.textContent = formatAbsoluteTime(metadata.reference_time_str, 0);

        // Create Ensemble Selector Options Grouped by Category
        ensembleSelect.innerHTML = '';
        
        // Add statistics first (separate / at the beginning)
        const statsGroup = document.createElement('optgroup');
        statsGroup.label = 'Statistics / Summary';
        
        const stats = ['med', 'max', 'prob'];
        const statLabels = { 
            'med': 'Median Forecast (MED)', 
            'max': 'Maximum Forecast (MAX)', 
            'prob': 'Precipitation Probability (PROB)' 
        };
        stats.forEach(stat => {
            const opt = document.createElement('option');
            opt.value = stat;
            opt.textContent = statLabels[stat];
            if (stat === currentEns) opt.selected = true;
            statsGroup.appendChild(opt);
        });
        ensembleSelect.appendChild(statsGroup);

        // Add individual ensemble members
        const membersGroup = document.createElement('optgroup');
        membersGroup.label = 'Ensemble Members';
        
        rainMetadata.ensembles.forEach(ens => {
            const opt = document.createElement('option');
            opt.value = ens.toString();
            opt.textContent = `Ensemble Member E${ens}`;
            if (ens === currentEns) opt.selected = true;
            membersGroup.appendChild(opt);
        });
        ensembleSelect.appendChild(membersGroup);

        // Add change event listener if not already added
        if (!ensembleSelect._hasChangeListener) {
            ensembleSelect.addEventListener('change', (e) => {
                let val = e.target.value;
                if (!isNaN(val)) {
                    val = parseInt(val);
                }
                selectEnsemble(val);
            });
            ensembleSelect._hasChangeListener = true;
        }

        // Initialize Timeline Slider
        timeSlider.min = 0;
        timeSlider.max = metadata.times.length - 1;
        timeSlider.value = currentTimeIndex;

        // Draw ticks on timeline
        drawSliderTicks();

        // Load initial overlay
        updateRadarOverlay();
        updateTimeStepDisplay();
        updateLegend();

    } catch (e) {
        console.error(e);
        refTimeVal.textContent = "Error loading data!";
    }
}

// Render ticks on the timeline slider
// Every hourly step is marked as a larger tick
function drawSliderTicks() {
    const ticksContainer = document.getElementById('slider-ticks');
    ticksContainer.innerHTML = '';
    
    const stepCount = metadata.times.length;
    for (let i = 0; i < stepCount; i++) {
        const span = document.createElement('span');
        const secs = metadata.times[i];
        
        // Mark every hour as a larger tick
        if (secs % 3600 === 0) {
            span.classList.add('hour-tick');
        }
        ticksContainer.appendChild(span);
    }
}

// Helper to load/bind WebGL textures asynchronously
let activeWindCacheKey = null;
function getOrLoadTexture(gl, timeVal) {
    if (!metadata) return null;
    
    const cacheKey = `${currentLayerMode}-${currentEns}-${timeVal}-${metadata.version}`;
    
    if (textureCache[cacheKey]) {
        const entry = textureCache[cacheKey];
        if (!entry.loaded) {
            return null; // Still loading the image
        }
        if (currentLayerMode === 'wind' && timeVal === metadata.times[currentTimeIndex]) {
            if (windPixelData === null || activeWindCacheKey !== cacheKey) {
                activeWindCacheKey = cacheKey;
                updateWindPixelData(entry.image);
            }
        }
        if (!entry.uploaded) {
            console.log(`Uploading texture to GPU for ${cacheKey}...`);
            gl.bindTexture(gl.TEXTURE_2D, entry.texture);
            gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, entry.image);
            
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
            gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
            
            entry.uploaded = true;
            console.log(`Texture uploaded successfully for ${cacheKey}.`);
        }
        return entry.texture;
    }
    
    // Keep cache size bounded to prevent memory bloat
    const keys = Object.keys(textureCache);
    if (keys.length > 250) {
        const oldestKey = keys[0];
        const oldestEntry = textureCache[oldestKey];
        if (oldestEntry) {
            console.log(`Evicting cached texture: ${oldestKey}`);
            if (gl && oldestEntry.texture) {
                gl.deleteTexture(oldestEntry.texture);
            }
            delete textureCache[oldestKey];
        }
    }

    // Create texture slot and load image asynchronously
    const texture = gl.createTexture();
    const entry = {
        texture: texture,
        loaded: false,
        uploaded: false,
        image: null
    };
    textureCache[cacheKey] = entry;
    
    console.log(`Starting image load for ${cacheKey}...`);
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
        console.log(`Image loaded successfully for ${cacheKey}.`);
        entry.image = img;
        entry.loaded = true;
        if (currentLayerMode === 'wind' && timeVal === metadata.times[currentTimeIndex]) {
            activeWindCacheKey = cacheKey;
            updateWindPixelData(img);
        }
        if (map) map.triggerRepaint();
    };
    img.onerror = (err) => {
        console.error(`Failed to load image for ${cacheKey}:`, err);
    };
    const srcPath = currentLayerMode === 'temp'
        ? `/api/data/temp/${timeVal}`
        : (currentLayerMode === 'wind' ? `/api/data/wind/${timeVal}` : `/api/data/${currentEns}/${timeVal}`);
    img.src = `${window.location.origin}${srcPath}?v=${metadata.version}`;
    
    return null;
}

// Custom MapLibre WebGL Layer Interface
const webglRadarLayer = {
    id: 'radar-webgl-layer',
    type: 'custom',
    renderingMode: '2d',
    
    onAdd: function (mapInstance, gl) {
        glContext = gl;
        console.log("Initializing WebGL Radar Layer shaders and buffers...");
        
        // 1. Compile Shaders
        const vertexShaderSource = `
            attribute vec2 a_position;
            attribute vec2 a_texcoord;
            varying vec2 v_texcoord;
            uniform mat4 u_matrix;
            void main() {
                gl_Position = u_matrix * vec4(a_position, 0.0, 1.0);
                v_texcoord = a_texcoord;
            }
        `;
        
        const fragmentShaderSource = `
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_texture;
            uniform float u_opacity;
            uniform vec4 u_colors[8];
            uniform float u_values[8];
            uniform int u_layer_mode;
            
            vec4 getColor(float val) {
                if (val < u_values[0]) return vec4(0.0);
                if (val <= u_values[1]) {
                    float t = (val - u_values[0]) / (u_values[1] - u_values[0]);
                    return mix(u_colors[0], u_colors[1], t);
                }
                if (val <= u_values[2]) {
                    float t = (val - u_values[1]) / (u_values[2] - u_values[1]);
                    return mix(u_colors[1], u_colors[2], t);
                }
                if (val <= u_values[3]) {
                    float t = (val - u_values[2]) / (u_values[3] - u_values[2]);
                    return mix(u_colors[2], u_colors[3], t);
                }
                if (val <= u_values[4]) {
                    float t = (val - u_values[3]) / (u_values[4] - u_values[3]);
                    return mix(u_colors[3], u_colors[4], t);
                }
                if (val <= u_values[5]) {
                    float t = (val - u_values[4]) / (u_values[5] - u_values[4]);
                    return mix(u_colors[4], u_colors[5], t);
                }
                if (val <= u_values[6]) {
                    float t = (val - u_values[5]) / (u_values[6] - u_values[5]);
                    return mix(u_colors[5], u_colors[6], t);
                }
                if (val <= u_values[7]) {
                    float t = (val - u_values[6]) / (u_values[7] - u_values[6]);
                    return mix(u_colors[6], u_colors[7], t);
                }
                return u_colors[7];
            }
            
            void main() {
                // Avoid border/interpolation artifacts by clamping coordinates to pixel centers
                vec2 clamped_coord = vec2(
                    0.5 / 700.0 + v_texcoord.x * (699.0 / 700.0),
                    0.5 / 765.0 + v_texcoord.y * (764.0 / 765.0)
                );
                vec4 tex = texture2D(u_texture, clamped_coord);
                if (tex.a < 0.99) {
                    discard;
                }
                float r = tex.r * 255.0;
                float g = tex.g * 255.0;
                float raw_val = r * 256.0 + g;
                if (raw_val >= 65535.0 || raw_val == 0.0) {
                    discard;
                }
                float val;
                if (u_layer_mode == 1) {
                    val = raw_val / 10.0 - 273.15;
                } else {
                    val = raw_val * 0.01;
                }
                vec4 c = getColor(val);
                if (c.a == 0.0) {
                    discard;
                }
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        `;
        
        function compileShader(source, type) {
            const shader = gl.createShader(type);
            gl.shaderSource(shader, source);
            gl.compileShader(shader);
            if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
                console.error("Shader compilation error:", gl.getShaderInfoLog(shader));
            }
            return shader;
        }
        
        const vs = compileShader(vertexShaderSource, gl.VERTEX_SHADER);
        const fs = compileShader(fragmentShaderSource, gl.FRAGMENT_SHADER);
        
        radarProgram = gl.createProgram();
        gl.attachShader(radarProgram, vs);
        gl.attachShader(radarProgram, fs);
        gl.linkProgram(radarProgram);
        
        if (!gl.getProgramParameter(radarProgram, gl.LINK_STATUS)) {
            console.error("Program linking error:", gl.getProgramInfoLog(radarProgram));
        }
        
        // 2. Set up Mercator projection vertex buffer
        const MAP_LIMIT = 20037508.342789244;
        function toMerc(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }
        
        // Bounding box: MERCATOR_LEFT: 0.0, MERCATOR_RIGHT: 1210000.0, MERCATOR_BOTTOM: 6250000.0, MERCATOR_TOP: 7560000.0
        const BL = toMerc(0.0, 6250000.0);
        const BR = toMerc(1210000.0, 6250000.0);
        const TR = toMerc(1210000.0, 7560000.0);
        const TL = toMerc(0.0, 7560000.0);
        
        // Define two triangles forming a quad (counter-clockwise order)
        const vertices = new Float32Array([
            BL[0], BL[1], // SW
            BR[0], BR[1], // SE
            TL[0], TL[1], // NW
            TL[0], TL[1], // NW
            BR[0], BR[1], // SE
            TR[0], TR[1]  // NE
        ]);
        
        positionBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, positionBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
        
        // Texture Coordinates (corrected to match UNPACK_FLIP_Y_WEBGL=true orientation)
        const texcoords = new Float32Array([
            0, 0, // BL
            1, 0, // BR
            0, 1, // TL
            0, 1, // TL
            1, 0, // BR
            1, 1  // TR
        ]);
        
        texcoordBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, texcoordBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, texcoords, gl.STATIC_DRAW);
    },
    
    render: function (gl, matrix) {
        if (!metadata || !radarProgram) return;
        
        const timeVal = metadata.times[currentTimeIndex];
        const texture = getOrLoadTexture(gl, timeVal);
        if (!texture) return; // Wait for texture load
        
        gl.useProgram(radarProgram);
        
        // 1. Save and disable depth test to ensure it always renders on top of the base map
        const depthTestEnabled = gl.isEnabled(gl.DEPTH_TEST);
        if (depthTestEnabled) {
            gl.disable(gl.DEPTH_TEST);
        }
        
        // 2. Bind default VAO to prevent mutating MapLibre's internal VAO state in WebGL 2
        if (gl.bindVertexArray) {
            gl.bindVertexArray(null);
        }
        
        // Bind position attribute
        const aPosition = gl.getAttribLocation(radarProgram, 'a_position');
        gl.enableVertexAttribArray(aPosition);
        gl.bindBuffer(gl.ARRAY_BUFFER, positionBuffer);
        gl.vertexAttribPointer(aPosition, 2, gl.FLOAT, false, 0, 0);
        
        // Bind texture coordinates attribute
        const aTexcoord = gl.getAttribLocation(radarProgram, 'a_texcoord');
        gl.enableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, texcoordBuffer);
        gl.vertexAttribPointer(aTexcoord, 2, gl.FLOAT, false, 0, 0);
        
        // Set projection matrix
        gl.uniformMatrix4fv(gl.getUniformLocation(radarProgram, 'u_matrix'), false, matrix);
        
        // Bind texture sampler
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.uniform1i(gl.getUniformLocation(radarProgram, 'u_texture'), 0);
        
        // Opacity uniform
        const opacity = parseFloat(opacitySlider.value) / 100;
        gl.uniform1f(gl.getUniformLocation(radarProgram, 'u_opacity'), opacity);
        
        // u_layer_mode uniform
        gl.uniform1i(gl.getUniformLocation(radarProgram, 'u_layer_mode'), currentLayerMode === 'temp' ? 1 : 0);
        
        // Dynamic color stops uniform configuration
        const isProb = currentEns === 'prob';
        let colors, values;
        
        if (currentLayerMode === 'temp') {
            colors = [
                [0/255, 43/255, 128/255, 0.8],
                [0/255, 204/255, 255/255, 0.8],
                [0/255, 255/255, 102/255, 0.8],
                [255/255, 255/255, 0/255, 0.8],
                [255/255, 153/255, 0/255, 0.85],
                [255/255, 77/255, 77/255, 0.9],
                [204/255, 0/255, 0/255, 0.95],
                [153/255, 0/255, 77/255, 1.0]
            ];
            values = [-10.0, 0.0, 10.0, 20.0, 25.0, 30.0, 35.0, 40.0];
        } else if (isProb) {
            colors = [
                [180/255, 200/255, 220/255, 0.0],
                [100/255, 160/255, 255/255, 0.5],
                [0/255, 100/255, 255/255, 0.65],
                [0/255, 200/255, 100/255, 0.75],
                [220/255, 0/255, 220/255, 0.85],
                [255/255, 255/255, 255/255, 0.95],
                [255/255, 255/255, 255/255, 0.95], // padding
                [255/255, 255/255, 255/255, 0.95]  // padding
            ];
            values = [0.10, 0.30, 0.50, 0.70, 0.90, 1.00, 1.00, 1.00];
        } else {
            colors = [
                [120/255, 200/255, 255/255, 0.0],
                [0/255, 100/255, 255/255, 0.7],
                [0/255, 200/255, 0/255, 0.7],
                [255/255, 230/255, 0/255, 0.8],
                [255/255, 120/255, 0/255, 0.9],
                [255/255, 0/255, 0/255, 0.95],
                [200/255, 0/255, 200/255, 1.0],
                [255/255, 255/255, 255/255, 1.0]
            ];
            values = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 100.0, 250.0];
        }
        
        const flatColors = new Float32Array(colors.reduce((acc, val) => acc.concat(val), []));
        const flatValues = new Float32Array(values);
        
        gl.uniform4fv(gl.getUniformLocation(radarProgram, 'u_colors[0]'), flatColors);
        gl.uniform1fv(gl.getUniformLocation(radarProgram, 'u_values[0]'), flatValues);
        
        // Alpha Blending config
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        
        // 3. Cleanup state to prevent leaks
        gl.disableVertexAttribArray(aPosition);
        gl.disableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        if (depthTestEnabled) {
            gl.enable(gl.DEPTH_TEST);
        }
    },
    
    onRemove: function (map, gl) {
        if (radarProgram) {
            gl.deleteProgram(radarProgram);
            radarProgram = null;
        }
        if (positionBuffer) {
            gl.deleteBuffer(positionBuffer);
            positionBuffer = null;
        }
        if (texcoordBuffer) {
            gl.deleteBuffer(texcoordBuffer);
            texcoordBuffer = null;
        }
    }
};

// Custom MapLibre WebGL Layer Interface for Wind
const webglWindLayer = {
    id: 'wind-webgl-layer',
    type: 'custom',
    renderingMode: '2d',
    
    onAdd: function (mapInstance, gl) {
        glContext = gl;
        console.log("Initializing WebGL Wind Layer shaders and buffers...");
        
        // 1. Compile background color shader
        const vertexShaderSource = `
            attribute vec2 a_position;
            attribute vec2 a_texcoord;
            varying vec2 v_texcoord;
            uniform mat4 u_matrix;
            void main() {
                gl_Position = u_matrix * vec4(a_position, 0.0, 1.0);
                v_texcoord = a_texcoord;
            }
        `;
        
        const fragmentShaderSource = `
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_texture;
            uniform float u_opacity;
            
            vec4 getColor(float val) {
                if (val < 0.0) return vec4(0.0);
                if (val <= 2.0) {
                    float t = val / 2.0;
                    return mix(vec4(96.0/255.0, 165.0/255.0, 250.0/255.0, 0.02), vec4(34.0/255.0, 211.0/255.0, 238.0/255.0, 0.35), t);
                }
                if (val <= 5.0) {
                    float t = (val - 2.0) / 3.0;
                    return mix(vec4(34.0/255.0, 211.0/255.0, 238.0/255.0, 0.35), vec4(74.0/255.0, 222.0/255.0, 128.0/255.0, 0.55), t);
                }
                if (val <= 10.0) {
                    float t = (val - 5.0) / 5.0;
                    return mix(vec4(74.0/255.0, 222.0/255.0, 128.0/255.0, 0.55), vec4(250.0/255.0, 204.0/255.0, 21.0/255.0, 0.7), t);
                }
                if (val <= 15.0) {
                    float t = (val - 10.0) / 5.0;
                    return mix(vec4(250.0/255.0, 204.0/255.0, 21.0/255.0, 0.7), vec4(251.0/255.0, 146.0/255.0, 60.0/255.0, 0.8), t);
                }
                if (val <= 20.0) {
                    float t = (val - 15.0) / 5.0;
                    return mix(vec4(251.0/255.0, 146.0/255.0, 60.0/255.0, 0.8), vec4(248.0/255.0, 113.0/255.0, 113.0/255.0, 0.85), t);
                }
                if (val <= 25.0) {
                    float t = (val - 20.0) / 5.0;
                    return mix(vec4(248.0/255.0, 113.0/255.0, 113.0/255.0, 0.85), vec4(236.0/255.0, 72.0/255.0, 153.0/255.0, 0.9), t);
                }
                return vec4(236.0/255.0, 72.0/255.0, 153.0/255.0, 0.9);
            }
            
            void main() {
                // Avoid border/interpolation artifacts by clamping coordinates to pixel centers
                float clamped_x = 0.5 / 700.0 + v_texcoord.x * (699.0 / 700.0);
                float clamped_y = 0.5 / 765.0 + v_texcoord.y * (764.0 / 765.0);
                
                // Top half: u-component, Bottom half: v-component
                vec2 texcoord_u = vec2(clamped_x, clamped_y * 0.5 + 0.5);
                vec2 texcoord_v = vec2(clamped_x, clamped_y * 0.5);
                
                vec4 tex_u = texture2D(u_texture, texcoord_u);
                vec4 tex_v = texture2D(u_texture, texcoord_v);
                
                if (tex_u.a < 0.99 || tex_v.a < 0.99) {
                    discard;
                }
                
                float u_raw = (tex_u.r * 255.0) * 256.0 + (tex_u.g * 255.0);
                float v_raw = (tex_v.r * 255.0) * 256.0 + (tex_v.g * 255.0);
                
                if (u_raw >= 65535.0 || v_raw >= 65535.0 || u_raw == 0.0 || v_raw == 0.0) {
                    discard;
                }
                
                float u = u_raw / 100.0 - 100.0;
                float v = v_raw / 100.0 - 100.0;
                float speed = sqrt(u * u + v * v);
                
                vec4 c = getColor(speed);
                gl_FragColor = vec4(c.rgb, c.a * u_opacity);
            }
        `;
        
        function compileShader(source, type) {
            const shader = gl.createShader(type);
            gl.shaderSource(shader, source);
            gl.compileShader(shader);
            if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
                console.error("Wind shader compilation error:", gl.getShaderInfoLog(shader));
            }
            return shader;
        }
        
        const vs = compileShader(vertexShaderSource, gl.VERTEX_SHADER);
        const fs = compileShader(fragmentShaderSource, gl.FRAGMENT_SHADER);
        
        windProgram = gl.createProgram();
        gl.attachShader(windProgram, vs);
        gl.attachShader(windProgram, fs);
        gl.linkProgram(windProgram);
        if (!gl.getProgramParameter(windProgram, gl.LINK_STATUS)) {
            console.error("Wind program linking error:", gl.getProgramInfoLog(windProgram));
        }
        
        // 2. Compile particle (arrows) shader program
        const particleVsSource = `
            attribute vec2 a_position;
            attribute float a_fade;
            attribute float a_trail;
            varying float v_fade;
            varying float v_trail;
            uniform mat4 u_matrix;
            uniform float u_point_size;
            void main() {
                gl_Position = u_matrix * vec4(a_position, 0.0, 1.0);
                gl_PointSize = u_point_size * (0.3 + 0.7 * a_trail);
                v_fade = a_fade;
                v_trail = a_trail;
            }
        `;
        
        const particleFsSource = `
            precision mediump float;
            varying float v_fade;
            varying float v_trail;
            uniform float u_arrow_opacity;
            
            void main() {
                vec2 p = gl_PointCoord - vec2(0.5);
                float dist = length(p);
                if (dist > 0.5) {
                    discard;
                }
                float edgeAlpha = smoothstep(0.5, 0.25, dist);
                float opacity = edgeAlpha * v_fade * v_trail * u_arrow_opacity;
                gl_FragColor = vec4(1.0, 1.0, 1.0, opacity);
            }
        `;
        
        const pVs = compileShader(particleVsSource, gl.VERTEX_SHADER);
        const pFs = compileShader(particleFsSource, gl.FRAGMENT_SHADER);
        
        particleProgram = gl.createProgram();
        gl.attachShader(particleProgram, pVs);
        gl.attachShader(particleProgram, pFs);
        gl.linkProgram(particleProgram);
        if (!gl.getProgramParameter(particleProgram, gl.LINK_STATUS)) {
            console.error("Particle program linking error:", gl.getProgramInfoLog(particleProgram));
        }
        
        // 3. Set up Mercator quad buffers
        const MAP_LIMIT = 20037508.342789244;
        function toMerc(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }
        
        const BL = toMerc(0.0, 6250000.0);
        const BR = toMerc(1210000.0, 6250000.0);
        const TR = toMerc(1210000.0, 7560000.0);
        const TL = toMerc(0.0, 7560000.0);
        
        const vertices = new Float32Array([
            BL[0], BL[1], // SW
            BR[0], BR[1], // SE
            TL[0], TL[1], // NW
            TL[0], TL[1], // NW
            BR[0], BR[1], // SE
            TR[0], TR[1]  // NE
        ]);
        
        windPositionBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, windPositionBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
        
        const texcoords = new Float32Array([
            0, 0, // BL
            1, 0, // BR
            0, 1, // TL
            0, 1, // TL
            1, 0, // BR
            1, 1  // TR
        ]);
        
        windTexcoordBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, windTexcoordBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, texcoords, gl.STATIC_DRAW);
        
        // 4. Set up dynamic buffer for particles
        particleBuffer = gl.createBuffer();
        
        // Seed particles on startup
        initParticles();
        lastAnimTime = performance.now();
    },
    
    render: function (gl, matrix) {
        if (!metadata || !windProgram || !particleProgram) return;
        
        const timeVal = metadata.times[currentTimeIndex];
        const texture = getOrLoadTexture(gl, timeVal);
        if (!texture) return; // Wait for texture load
        
        // 1. Update Particle positions on CPU
        const now = performance.now();
        let dt = (now - lastAnimTime) / 1000.0;
        if (dt > 0.1) dt = 0.1; // Cap dt to prevent warp jumps
        lastAnimTime = now;
        
        const zoom = map ? map.getZoom() : 6;
        const lat = 52.0;
        const metersPerPixel = 156543.03 * Math.cos(lat * Math.PI / 180) / Math.pow(2, zoom);
        const minDistance = 1.2 * metersPerPixel;
        
        if (windPixelData) {
            updateParticles(dt, minDistance);
        }
        
        // Disable depth test
        const depthTestEnabled = gl.isEnabled(gl.DEPTH_TEST);
        if (depthTestEnabled) {
            gl.disable(gl.DEPTH_TEST);
        }
        if (gl.bindVertexArray) {
            gl.bindVertexArray(null);
        }
        
        // -------------------------------------------------------------
        // Step A: Draw Background Vector Speed Field Overlay
        // -------------------------------------------------------------
        gl.useProgram(windProgram);
        
        // Bind position quad attributes
        const aPosition = gl.getAttribLocation(windProgram, 'a_position');
        gl.enableVertexAttribArray(aPosition);
        gl.bindBuffer(gl.ARRAY_BUFFER, windPositionBuffer);
        gl.vertexAttribPointer(aPosition, 2, gl.FLOAT, false, 0, 0);
        
        const aTexcoord = gl.getAttribLocation(windProgram, 'a_texcoord');
        gl.enableVertexAttribArray(aTexcoord);
        gl.bindBuffer(gl.ARRAY_BUFFER, windTexcoordBuffer);
        gl.vertexAttribPointer(aTexcoord, 2, gl.FLOAT, false, 0, 0);
        
        // Set uniforms
        gl.uniformMatrix4fv(gl.getUniformLocation(windProgram, 'u_matrix'), false, matrix);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.uniform1i(gl.getUniformLocation(windProgram, 'u_texture'), 0);
        
        const opacity = parseFloat(opacitySlider.value) / 100;
        gl.uniform1f(gl.getUniformLocation(windProgram, 'u_opacity'), opacity);
        
        // Blend mode configuration
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        
        gl.disableVertexAttribArray(aPosition);
        gl.disableVertexAttribArray(aTexcoord);
        
        // -------------------------------------------------------------
        // Step B: Draw Moving Particles (Arrows) on Top
        // -------------------------------------------------------------
        gl.useProgram(particleProgram);
        
        // Helper to convert Mercator meters to normalized coordinate space [0, 1]
        const MAP_LIMIT = 20037508.342789244;
        function toMerc(x, y) {
            const ux = (x + MAP_LIMIT) / (2.0 * MAP_LIMIT);
            const uy = (MAP_LIMIT - y) / (2.0 * MAP_LIMIT);
            return [ux, uy];
        }
        
        // Pack active particle variables: positions, fade, trail factor
        const bufferData = new Float32Array(maxParticles * TRAIL_LENGTH * 4);
        let offset = 0;
        for (let i = 0; i < maxParticles; i++) {
            const p = particles[i];
            
            // Calculate fade envelope (sinusoidal fade in/out)
            const progress = Math.min(Math.max(p.age / p.maxAge, 0.0), 1.0);
            const fade = Math.sin(progress * Math.PI);
            
            for (let j = 0; j < TRAIL_LENGTH; j++) {
                const pos = p.history[j];
                const [ux, uy] = toMerc(pos.mx, pos.my);
                const trailFactor = 1.0 - (j / (TRAIL_LENGTH - 1));
                
                bufferData[offset++] = ux;
                bufferData[offset++] = uy;
                bufferData[offset++] = fade;
                bufferData[offset++] = trailFactor;
            }
        }
        
        // Upload dynamic particle buffer data to VBO
        gl.bindBuffer(gl.ARRAY_BUFFER, particleBuffer);
        gl.bufferData(gl.ARRAY_BUFFER, bufferData, gl.DYNAMIC_DRAW);
        
        // Set attributes
        const stride = 16; // 4 floats * 4 bytes/float = 16
        const aPartPos = gl.getAttribLocation(particleProgram, 'a_position');
        gl.enableVertexAttribArray(aPartPos);
        gl.vertexAttribPointer(aPartPos, 2, gl.FLOAT, false, stride, 0);
        
        const aPartFade = gl.getAttribLocation(particleProgram, 'a_fade');
        gl.enableVertexAttribArray(aPartFade);
        gl.vertexAttribPointer(aPartFade, 1, gl.FLOAT, false, stride, 8);
        
        const aPartTrail = gl.getAttribLocation(particleProgram, 'a_trail');
        gl.enableVertexAttribArray(aPartTrail);
        gl.vertexAttribPointer(aPartTrail, 1, gl.FLOAT, false, stride, 12);
        
        // Set uniforms
        gl.uniformMatrix4fv(gl.getUniformLocation(particleProgram, 'u_matrix'), false, matrix);
        // Base streak point size: 7.5px for the head
        gl.uniform1f(gl.getUniformLocation(particleProgram, 'u_point_size'), 7.5);
        gl.uniform1f(gl.getUniformLocation(particleProgram, 'u_arrow_opacity'), 0.85);
        
        // Draw particle arrays
        gl.drawArrays(gl.POINTS, 0, maxParticles * TRAIL_LENGTH);
        
        // Clean attributes
        gl.disableVertexAttribArray(aPartPos);
        gl.disableVertexAttribArray(aPartFade);
        gl.disableVertexAttribArray(aPartTrail);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        
        if (depthTestEnabled) {
            gl.enable(gl.DEPTH_TEST);
        }
        
        // Trigger repaint to run animation loop if Wind is the active layer
        if (currentLayerMode === 'wind' && map) {
            map.triggerRepaint();
        }
    },
    
    onRemove: function (map, gl) {
        if (windProgram) {
            gl.deleteProgram(windProgram);
            windProgram = null;
        }
        if (particleProgram) {
            gl.deleteProgram(particleProgram);
            particleProgram = null;
        }
        if (windPositionBuffer) {
            gl.deleteBuffer(windPositionBuffer);
            windPositionBuffer = null;
        }
        if (windTexcoordBuffer) {
            gl.deleteBuffer(windTexcoordBuffer);
            windTexcoordBuffer = null;
        }
        if (particleBuffer) {
            gl.deleteBuffer(particleBuffer);
            particleBuffer = null;
        }
    }
};

// Clear cached textures and release GPU memory
function clearRadarLayers() {
    if (glContext) {
        for (const cacheKey in textureCache) {
            if (textureCache[cacheKey].texture) {
                glContext.deleteTexture(textureCache[cacheKey].texture);
            }
        }
    }
    textureCache = {};
    if (map) map.triggerRepaint();
}

// Add the custom WebGL layer to style
function setupRadarSourceAndLayer() {
    if (!metadata || !map || !map.isStyleLoaded()) return;

    if (map.getLayer('radar-webgl-layer')) {
        map.removeLayer('radar-webgl-layer');
    }
    if (map.getLayer('wind-webgl-layer')) {
        map.removeLayer('wind-webgl-layer');
    }

    if (currentLayerMode === 'wind') {
        map.addLayer(webglWindLayer);
        initParticles();
        lastAnimTime = performance.now();
        map.triggerRepaint();
    } else {
        map.addLayer(webglRadarLayer);
    }
}

// Trigger map repaint and preload future frames
function updateRadarOverlay() {
    if (!metadata || !map || !map.isStyleLoaded()) return;

    map.triggerRepaint();

    if (glContext) {
        const len = metadata.times.length;
        for (let i = 1; i <= CONFIG.cache.preloadAhead; i++) {
            const nextIndex = (currentTimeIndex + i) % len;
            const nextTimeVal = metadata.times[nextIndex];
            getOrLoadTexture(glContext, nextTimeVal);
        }
    }
}

// Update time text displays
// Format absolute and relative forecast times from metadata
function updateTimeStepDisplay() {
    if (!metadata) return;
    const timeVal = metadata.times[currentTimeIndex];
    currentTimeStep.textContent = formatAbsoluteTime(metadata.reference_time_str, timeVal);
    timeStepRelative.textContent = formatRelativeTime(timeVal);
}

// Update the legend colors and labels dynamically
function updateLegend() {
    const legendTitle = document.querySelector('#legend-rain .section-label');
    const legendBar = document.querySelector('#legend-rain .legend-bar');
    const legendLabels = document.querySelector('#legend-rain .legend-labels');
    if (!legendTitle || !legendBar || !legendLabels) return;

    const visConfig = (currentEns === 'prob') ? CONFIG.radarVisualization.prob : CONFIG.radarVisualization.rate;
    
    legendTitle.textContent = visConfig.title;
    
    legendBar.innerHTML = visConfig.colors
        .map(color => `<span style="background: ${color};"></span>`)
        .join('');
        
    legendLabels.innerHTML = visConfig.labels
        .map(label => `<span>${label}</span>`)
        .join('');
}

// Handle Ensemble Switch
function selectEnsemble(ens) {
    currentEns = ens;
    clearRadarLayers();
    
    // Update dropdown value if it differs
    if (ensembleSelect.value !== ens.toString()) {
        ensembleSelect.value = ens.toString();
    }

    // Update quick selector buttons active state and sliding indicator
    const viewMap = { 'med': '0', 'max': '1', 'prob': '2' };
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === ens.toString());
    });
    const selector = document.querySelector('.view-selector');
    if (selector && viewMap[ens] !== undefined) {
        selector.dataset.active = viewMap[ens];
    }
    
    updateRadarOverlay();
    updateLegend();
    triggerHoverQuery(); // update hover panel if mouse is over map
    
    // If timeseries chart is open, reload it for the new ensemble selection
    if (activeCoords) {
        showTimeseriesChart(activeCoords.lat, activeCoords.lon);
    }
}

// Handle Slider Input
timeSlider.addEventListener('input', (e) => {
    currentTimeIndex = parseInt(e.target.value);
    updateRadarOverlay();
    updateTimeStepDisplay();
    triggerHoverQuery();
});

// Opacity Slider handler
opacitySlider.addEventListener('input', (e) => {
    const val = e.target.value;
    opacityValue.textContent = `${val}%`;
    if (map) {
        map.triggerRepaint();
    }
});

// Speed Slider handler
speedSlider.addEventListener('input', (e) => {
    const val = e.target.value;
    speedValue.textContent = `${val} fps`;
    if (isPlaying) {
        stopPlayer();
        startPlayer();
    }
});

// Step Controls
btnPrev.addEventListener('click', () => {
    stopPlayer();
    if (currentTimeIndex > 0) {
        currentTimeIndex--;
    } else if (metadata) {
        currentTimeIndex = metadata.times.length - 1; // loop
    }
    timeSlider.value = currentTimeIndex;
    updateRadarOverlay();
    updateTimeStepDisplay();
    triggerHoverQuery();
});

btnNext.addEventListener('click', () => {
    stopPlayer();
    stepForward();
});

function stepForward() {
    if (!metadata) return;
    if (currentTimeIndex < metadata.times.length - 1) {
        currentTimeIndex++;
    } else {
        currentTimeIndex = 0; // loop
    }
    timeSlider.value = currentTimeIndex;
    updateRadarOverlay();
    updateTimeStepDisplay();
    triggerHoverQuery();
}

// Player state control
btnPlay.addEventListener('click', () => {
    if (isPlaying) {
        stopPlayer();
    } else {
        startPlayer();
    }
});

function startPlayer() {
    isPlaying = true;
    btnPlay.innerHTML = '<i class="fa-solid fa-pause"></i>';
    btnPlay.classList.add('btn-active');
    
    const fps = parseInt(speedSlider.value);
    const intervalMs = 1000 / fps;
    playInterval = setInterval(stepForward, intervalMs);
}

// Stop playback
function stopPlayer() {
    if (!isPlaying) return;
    isPlaying = false;
    btnPlay.innerHTML = '<i class="fa-solid fa-play"></i>';
    btnPlay.classList.remove('btn-active');
    clearInterval(playInterval);
}

// Map Hover Values Logic
let lastLat = null;
let lastLon = null;
let hoverTimeout = null;

// Throttled mouse listener on map
function handleMapMouseMove(e) {
    const lat = e.lngLat.lat;
    const lon = e.lngLat.lng;
    lastLat = lat;
    lastLon = lon;

    // Show coordinates in panel
    hoverCoords.textContent = `lat: ${lastLat.toFixed(4)}, lon: ${lastLon.toFixed(4)}`;
    hoverPanel.classList.remove('glass-panel', 'hidden');
    hoverPanel.classList.add('glass-panel'); // Make sure it's shown

    // Throttle queries
    if (hoverTimeout) return;
    hoverTimeout = setTimeout(() => {
        hoverTimeout = null;
        triggerHoverQuery();
    }, CONFIG.intervals.hoverThrottleMs);
}

// Hide hover panel when mouse leaves map
function handleMapMouseLeave() {
    hoverPanel.classList.add('hidden');
    lastLat = null;
    lastLon = null;
}

// Performs fetch to API value endpoint
async function triggerHoverQuery() {
    if (lastLat === null || lastLon === null || !metadata) return;

    const timeVal = metadata.times[currentTimeIndex];
    if (currentLayerMode === 'temp') {
        try {
            const response = await fetch(`/api/value/temp?time=${timeVal}&lat=${lastLat}&lon=${lastLon}`);
            if (!response.ok) throw new Error("Temp value query failed");
            const res = await response.json();

            if (res.status === "out_of_bounds") {
                hoverValue.textContent = "Out of Grid";
                hoverValue.style.color = "var(--text-secondary)";
            } else if (res.value === null) {
                hoverValue.textContent = "No Data";
                hoverValue.style.color = "var(--text-secondary)";
            } else {
                hoverValue.textContent = `${res.value.toFixed(1)} °C`;
                // Color code temperature hover value
                if (res.value < 0) hoverValue.style.color = "#38bdf8"; // Freezing: sky blue
                else if (res.value < 10) hoverValue.style.color = "#60a5fa"; // Cool: blue
                else if (res.value < 20) hoverValue.style.color = "#4ade80"; // Mild: green
                else if (res.value < 28) hoverValue.style.color = "#facc15"; // Warm: yellow
                else if (res.value < 33) hoverValue.style.color = "#fb923c"; // Hot: orange
                else hoverValue.style.color = "#f87171"; // Very hot: red
            }
        } catch (e) {
            console.error("Temp Hover error:", e);
            hoverValue.textContent = "Error";
            hoverValue.style.color = "#f87171";
        }
    } else if (currentLayerMode === 'wind') {
        try {
            const response = await fetch(`/api/value/wind?time=${timeVal}&lat=${lastLat}&lon=${lastLon}`);
            if (!response.ok) throw new Error("Wind value query failed");
            const res = await response.json();

            if (res.status === "out_of_bounds") {
                hoverValue.textContent = "Out of Grid";
                hoverValue.style.color = "var(--text-secondary)";
            } else if (res.speed === null) {
                hoverValue.textContent = "No Data";
                hoverValue.style.color = "var(--text-secondary)";
            } else {
                const bft = mpsToBeaufort(res.speed);
                const cardinal = degreesToCardinal(res.direction);
                hoverValue.textContent = `${res.speed.toFixed(1)} m/s (${bft} Bft) ${cardinal}`;
                
                // Color code wind speed
                if (res.speed < 2.0) hoverValue.style.color = "#94a3b8"; // Calm: blue-gray
                else if (res.speed < 5.0) hoverValue.style.color = "#22d3ee"; // Light: cyan
                else if (res.speed < 10.0) hoverValue.style.color = "#4ade80"; // Moderate: green
                else if (res.speed < 15.0) hoverValue.style.color = "#facc15"; // Strong: yellow
                else if (res.speed < 20.0) hoverValue.style.color = "#fb923c"; // Gale: orange
                else hoverValue.style.color = "#f87171"; // Storm: red
            }
        } catch (e) {
            console.error("Wind Hover error:", e);
            hoverValue.textContent = "Error";
            hoverValue.style.color = "#f87171";
        }
    } else {
        try {
            const response = await fetch(`/api/value?ens=${currentEns}&time=${timeVal}&lat=${lastLat}&lon=${lastLon}`);
            if (!response.ok) throw new Error("Value query failed");
            const res = await response.json();

            if (res.status === "out_of_bounds") {
                hoverValue.textContent = "Out of Grid";
                hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "no_rain" || res.value === 0.0) {
                if (currentEns === 'prob') {
                    hoverValue.textContent = "0% Chance";
                } else {
                    hoverValue.textContent = "0.00 mm/h";
                }
                hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "probability") {
                hoverValue.textContent = `${Math.round(res.value)}% Chance`;
                // Color code probability
                if (res.value < 30) hoverValue.style.color = "#94a3b8"; // Grey-blue
                else if (res.value < 70) hoverValue.style.color = "#3b82f6"; // Blue
                else hoverValue.style.color = "#a855f7"; // Purple / High probability
            } else {
                hoverValue.textContent = `${res.value.toFixed(2)} mm/h`;
                // Color code value dynamically in panel based on intensity
                if (res.value < 0.2) hoverValue.style.color = "#38bdf8"; // Light sky-blue
                else if (res.value < 1.0) hoverValue.style.color = "#60a5fa"; // Blue
                else if (res.value < 5.0) hoverValue.style.color = "#4ade80"; // Green
                else if (res.value < 15.0) hoverValue.style.color = "#facc15"; // Yellow
                else if (res.value < 30.0) hoverValue.style.color = "#fb923c"; // Orange
                else hoverValue.style.color = "#f87171"; // Red
            }
        } catch (e) {
            console.error("Hover error:", e);
            hoverValue.textContent = "Error";
            hoverValue.style.color = "#f87171";
        }
    }
}

// Poll for metadata updates to detect new NetCDF file
function startMetadataPolling() {
    setInterval(async () => {
        try {
            const endpoint = currentLayerMode === 'temp' 
                ? '/api/metadata/temp' 
                : (currentLayerMode === 'wind' ? '/api/metadata/wind' : '/api/metadata');
            const response = await fetch(endpoint);
            if (!response.ok) return;
            const newMetadata = await response.json();

            let targetMetadata = currentLayerMode === 'temp' 
                ? tempMetadata 
                : (currentLayerMode === 'wind' ? windMetadata : rainMetadata);
                
            if (targetMetadata && newMetadata.version !== targetMetadata.version) {
                console.log(`New ${currentLayerMode} forecast run detected! Reloading...`);
                clearRadarLayers();
                
                if (currentLayerMode === 'temp') {
                    tempMetadata = newMetadata;
                    metadata = tempMetadata;
                } else if (currentLayerMode === 'wind') {
                    windMetadata = newMetadata;
                    metadata = windMetadata;
                    windPixelData = null;
                    activeWindCacheKey = null;
                } else {
                    rainMetadata = newMetadata;
                    metadata = rainMetadata;
                }

                // Re-render timeline slider (just in case the number of times changed)
                timeSlider.max = metadata.times.length - 1;
                if (currentTimeIndex >= metadata.times.length) {
                    currentTimeIndex = 0;
                    timeSlider.value = 0;
                }
                drawSliderTicks();
                updateTimeStepDisplay();
                updateRadarOverlay();

                // Update reference time display
                refTimeVal.textContent = formatAbsoluteTime(metadata.reference_time_str, 0);
            }
        } catch (e) {
            console.error("Failed to check for metadata update:", e);
        }
    }, CONFIG.intervals.metadataPollingMs);
}

// Renders the interactive timeseries chart using Chart.js
async function showTimeseriesChart(lat, lon) {
    if (!metadata) return;
    activeCoords = { lat, lon };
    
    // Show the panel
    chartPanel.classList.remove('hidden');
    chartCoords.textContent = `lat: ${lat.toFixed(4)}, lon: ${lon.toFixed(4)}`;
    
    try {
        const url = currentLayerMode === 'temp'
            ? `/api/timeseries/temp?lat=${lat}&lon=${lon}`
            : (currentLayerMode === 'wind' ? `/api/timeseries/wind?lat=${lat}&lon=${lon}` : `/api/timeseries?ens=${currentEns}&lat=${lat}&lon=${lon}`);
        const res = await fetch(url);
        if (!res.ok) throw new Error("Timeseries request failed");
        const data = await res.json();
        
        const chartValues = currentLayerMode === 'wind' ? data.speeds : data.values;
        if (data.status === "out_of_bounds" || chartValues.length === 0) {
            chartCoords.textContent = currentLayerMode === 'temp'
                ? "Selected point is out of bounds"
                : (currentLayerMode === 'wind' ? "Selected point is out of wind bounds" : "Selected point is out of radar bounds");
            if (chartInstance) {
                chartInstance.destroy();
                chartInstance = null;
            }
            chartStatPeak.textContent = "--";
            chartStatTotal.textContent = "--";
            return;
        }
        
        const peakVal = Math.max(...chartValues);
        let totalVal = 0.0;
        
        if (currentLayerMode === 'temp') {
            const minVal = Math.min(...chartValues);
            chartStatPeak.textContent = `${peakVal.toFixed(1)} °C`;
            chartStatTotal.textContent = `${minVal.toFixed(1)} °C`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Max Temp";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Min Temp";
            document.querySelector('#chart-panel h3').innerHTML = '<i class="fa-solid fa-temperature-half chart-header-icon"></i> Temperature Forecast Trend';
        } else if (currentLayerMode === 'wind') {
            const avgVal = chartValues.reduce((a, b) => a + b, 0) / chartValues.length;
            chartStatPeak.textContent = `${peakVal.toFixed(1)} m/s`;
            chartStatTotal.textContent = `${avgVal.toFixed(1)} m/s`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Max Wind";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Avg Wind";
            document.querySelector('#chart-panel h3').innerHTML = '<i class="fa-solid fa-wind chart-header-icon"></i> Wind Speed Forecast Trend';
        } else if (currentEns === 'prob') {
            chartStatPeak.textContent = `${Math.round(peakVal)}%`;
            const avgVal = chartValues.reduce((a, b) => a + b, 0) / chartValues.length;
            chartStatTotal.textContent = `${Math.round(avgVal)}% (avg)`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Peak Probability";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Avg Probability";
            document.querySelector('#chart-panel h3').innerHTML = '<i class="fa-solid fa-chart-line chart-header-icon"></i> Rainfall Forecast Trend';
        } else {
            // total_mm = sum(rates) / 12 (5 mins intervals)
            totalVal = chartValues.reduce((a, b) => a + b, 0) / 12.0;
            chartStatPeak.textContent = `${peakVal.toFixed(2)} mm/h`;
            chartStatTotal.textContent = `${totalVal.toFixed(2)} mm`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Peak Intensity";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Total Accumulation";
            document.querySelector('#chart-panel h3').innerHTML = '<i class="fa-solid fa-chart-line chart-header-icon"></i> Rainfall Forecast Trend';
        }
        
        const labels = data.times.map(secs => {
            const timeStr = formatAbsoluteTime(metadata.reference_time_str, secs);
            if (currentLayerMode === 'temp' || currentLayerMode === 'wind') {
                // Include day for multi-day temperature and wind forecasts, e.g. "Mon 08:00"
                const match = timeStr.match(/(\d{2})\s+(\w+).*?(\d{2}:\d{2})/);
                if (match) {
                    // Parse to get short weekday name
                    const refMatch = metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
                    if (refMatch) {
                        const refDate = new Date(`${refMatch[1]}T${refMatch[2]}Z`);
                        const targetDate = new Date(refDate.getTime() + secs * 1000);
                        const dayName = targetDate.toLocaleDateString('en-GB', { timeZone: 'Europe/Amsterdam', weekday: 'short' });
                        return `${dayName} ${match[3]}`;
                    }
                    return `${match[1]} ${match[2]} ${match[3]}`;
                }
            }
            const match = timeStr.match(/(\d{2}:\d{2})/);
            return match ? match[1] : `+${Math.round(secs/60)}m`;
        });
        
        const isProb = currentEns === 'prob';
        let labelText, borderColor, backgroundColor;
        
        if (currentLayerMode === 'temp') {
            labelText = "2m Temperature (°C)";
            borderColor = "#f87171"; // Warm red
            backgroundColor = "rgba(248, 113, 113, 0.15)";
        } else if (currentLayerMode === 'wind') {
            labelText = "10m Wind Speed (m/s)";
            borderColor = "#22d3ee"; // Neon cyan
            backgroundColor = "rgba(34, 211, 238, 0.15)";
        } else {
            labelText = isProb ? CONFIG.radarVisualization.prob.title + " (%)" : CONFIG.radarVisualization.rate.title;
            const chartColors = isProb ? CONFIG.chart.colors.prob : CONFIG.chart.colors.rate;
            borderColor = chartColors.border;
            backgroundColor = chartColors.background;
        }
        
        const ctx = document.getElementById('rainfall-chart').getContext('2d');
        
        if (chartInstance) {
            chartInstance.data.labels = labels;
            chartInstance.data.datasets[0].label = labelText;
            chartInstance.data.datasets[0].data = chartValues;
            chartInstance.data.datasets[0].borderColor = borderColor;
            chartInstance.data.datasets[0].backgroundColor = backgroundColor;
            chartInstance.options.scales.y.title.text = labelText;
            chartInstance.options.scales.y.max = (currentLayerMode === 'temp' || currentLayerMode === 'wind') ? undefined : (isProb ? 100 : undefined);
            chartInstance.options.scales.y.min = (currentLayerMode === 'temp' || currentLayerMode === 'wind') ? undefined : 0;
            chartInstance.update();
        } else {
            chartInstance = new Chart(ctx, {
                type: 'line',
                data: {
                    labels: labels,
                    datasets: [{
                        label: labelText,
                        data: chartValues,
                        borderColor: borderColor,
                        backgroundColor: backgroundColor,
                        borderWidth: CONFIG.chart.borderWidth,
                        fill: true,
                        tension: CONFIG.chart.tension,
                        pointRadius: CONFIG.chart.pointRadius,
                        pointHoverRadius: CONFIG.chart.pointHoverRadius
                    }]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    plugins: {
                        legend: {
                            display: false
                        },
                        tooltip: {
                            mode: 'index',
                            intersect: false,
                            backgroundColor: '#1e1e24',
                            titleColor: '#f8fafc',
                            bodyColor: '#f8fafc',
                            borderColor: 'rgba(255,255,255,0.1)',
                            borderWidth: 1,
                            callbacks: {
                                label: function(context) {
                                    if (currentLayerMode === 'temp') {
                                        return ` ${context.parsed.y.toFixed(1)} °C`;
                                    } else if (currentLayerMode === 'wind') {
                                        return ` ${context.parsed.y.toFixed(1)} m/s (${mpsToBeaufort(context.parsed.y)} Bft)`;
                                    }
                                    return ` ${context.parsed.y.toFixed(2)}${isProb ? '%' : ' mm/h'}`;
                                }
                            }
                        }
                    },
                    scales: {
                        x: {
                            grid: {
                                color: 'rgba(255, 255, 255, 0.05)'
                            },
                            ticks: {
                                color: '#94a3b8',
                                font: {
                                    size: 9
                                },
                                maxTicksLimit: CONFIG.chart.maxTicksLimit
                            }
                        },
                        y: {
                            grid: {
                                color: 'rgba(255, 255, 255, 0.05)'
                            },
                            ticks: {
                                color: '#94a3b8',
                                font: {
                                    size: 9
                                }
                            },
                            title: {
                                display: true,
                                text: labelText,
                                color: '#94a3b8',
                                font: {
                                    size: 9,
                                    weight: 'bold'
                                }
                            },
                            min: (currentLayerMode === 'temp' || currentLayerMode === 'wind') ? undefined : 0,
                            max: (currentLayerMode === 'temp' || currentLayerMode === 'wind') ? undefined : (isProb ? 100 : undefined)
                        }
                    }
                }
            });
        }
    } catch (e) {
        console.error("Timeseries error:", e);
        chartCoords.textContent = "Error loading trend chart";
    }
}

// Close chart, destroy chart instance and remove MapLibre pin marker
function closeTimeseriesChart() {
    chartPanel.classList.add('hidden');
    activeCoords = null;
    
    if (clickedMarker) {
        clickedMarker.remove();
        clickedMarker = null;
    }
    
    if (chartInstance) {
        chartInstance.destroy();
        chartInstance = null;
    }
}

// Map Click Listener
function handleMapClick(e) {
    const lat = e.lngLat.lat;
    const lon = e.lngLat.lng;
    
    if (clickedMarker) {
        clickedMarker.setLngLat(e.lngLat);
    } else {
        clickedMarker = new maplibregl.Marker()
            .setLngLat(e.lngLat)
            .addTo(map);
    }
    
    showTimeseriesChart(lat, lon);
}

// Select Layer Mode (Rain vs Temp vs Wind)
function selectLayerMode(mode) {
    if (mode === currentLayerMode) return;
    currentLayerMode = mode;
    
    // Update button active state
    document.querySelectorAll('.layer-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.mode === mode);
    });
    
    const selector = document.querySelector('.layer-selector');
    if (selector) {
        selector.dataset.active = mode === 'rain' ? '0' : (mode === 'temp' ? '1' : '2');
    }
    
    // Toggle UI visibility depending on layer mode
    const viewSelector = document.getElementById('rain-view-selector');
    const ensembleContainer = document.querySelector('.ensemble-select-container');
    const legendRain = document.getElementById('legend-rain');
    const legendTemp = document.getElementById('legend-temp');
    const legendWind = document.getElementById('legend-wind');
    
    if (mode === 'temp') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        if (legendTemp) legendTemp.classList.remove('hidden');
        
        metadata = tempMetadata;
    } else if (mode === 'wind') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.remove('hidden');
        
        metadata = windMetadata;
    } else {
        if (viewSelector) viewSelector.classList.remove('hidden');
        if (ensembleContainer) ensembleContainer.classList.remove('hidden');
        if (legendRain) legendRain.classList.remove('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        
        metadata = rainMetadata;
    }
    
    // Re-initialize slider and select index closest to current time
    if (metadata) {
        timeSlider.max = metadata.times.length - 1;
        
        const refMatch = metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
        let refTimeMs = Date.now();
        if (refMatch) {
            refTimeMs = new Date(`${refMatch[1]}T${refMatch[2]}Z`).getTime();
        }
        const targetOffset = (Date.now() - refTimeMs) / 1000;
        let closestIndex = 0;
        let minDiff = Infinity;
        for (let i = 0; i < metadata.times.length; i++) {
            const diff = Math.abs(metadata.times[i] - targetOffset);
            if (diff < minDiff) {
                minDiff = diff;
                closestIndex = i;
            }
        }
        currentTimeIndex = closestIndex;
        
        timeSlider.value = currentTimeIndex;
        drawSliderTicks();
        updateTimeStepDisplay();
    }
    
    clearRadarLayers();
    setupRadarSourceAndLayer();
    updateRadarOverlay();
    
    // Update hover panel label
    const hoverLabel = document.getElementById('hover-label');
    if (hoverLabel) {
        hoverLabel.textContent = mode === 'temp' ? 'TEMPERATURE' : (mode === 'wind' ? '10M WIND' : 'PRECIPITATION');
    }
    
    // Update hover panel & trend chart if open
    triggerHoverQuery();
    if (activeCoords) {
        showTimeseriesChart(activeCoords.lat, activeCoords.lon);
    }
}

// Switch Map Styles
function switchMapStyle(styleKey) {
    if (map && CONFIG.map.styles[styleKey]) {
        map.setStyle(CONFIG.map.styles[styleKey]);
    }
}

// App Entry Point
window.addEventListener('DOMContentLoaded', () => {
    // Sync default UI control values from CONFIG
    speedSlider.value = CONFIG.defaults.speed;
    speedValue.textContent = `${CONFIG.defaults.speed} fps`;
    opacitySlider.value = CONFIG.defaults.opacity;
    opacityValue.textContent = `${CONFIG.defaults.opacity}%`;

    // Restore settings drawer state
    const savedExpanded = localStorage.getItem('nimbus_settings_expanded');
    const isMobile = window.innerWidth <= 768;
    if (savedExpanded === 'true' && !isMobile) {
        settingsContent.classList.add('expanded');
        btnSettingsToggle.classList.add('active');
    } else {
        settingsContent.classList.remove('expanded');
        btnSettingsToggle.classList.remove('active');
    }

    // Set initial quick view selector state
    const viewMap = { 'med': '0', 'max': '1', 'prob': '2' };
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === currentEns.toString());
    });
    const viewSelector = document.querySelector('.view-selector');
    if (viewSelector && viewMap[currentEns] !== undefined) {
        viewSelector.dataset.active = viewMap[currentEns];
    }

    initMap();
    loadApp();
    startMetadataPolling();

    // Attach local controls listeners
    chartCloseBtn.addEventListener('click', closeTimeseriesChart);

    themeSelect.addEventListener('change', (e) => {
        switchMapStyle(e.target.value);
    });

    btnSettingsToggle.addEventListener('click', () => {
        const isExpanded = settingsContent.classList.toggle('expanded');
        btnSettingsToggle.classList.toggle('active', isExpanded);
        localStorage.setItem('nimbus_settings_expanded', isExpanded);
    });

    // Attach quick view toggle event listeners
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            selectEnsemble(btn.dataset.view);
        });
    });

    // Attach layer toggle event listeners
    document.querySelectorAll('.layer-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            selectLayerMode(btn.dataset.mode);
        });
    });
});
