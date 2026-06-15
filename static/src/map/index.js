import { CONFIG } from '../config.js';
import { state } from '../state.js';
import { DOM } from '../ui/dom.js';
import { fetchHoverValue } from '../api.js';
import { mpsToBeaufort, degreesToCardinal, showTimeseriesChart } from '../ui/chart.js';
import { WebGLRadarLayer } from './WebGLRadar.js';
import { WebGLWindLayer } from './WebGLWind.js';

let radarLayerInstance = new WebGLRadarLayer();
let windLayerInstance = new WebGLWindLayer();

let activeWindCacheKey = null;

// Helper to load/bind WebGL textures asynchronously
export function getOrLoadTexture(gl, timeVal) {
    if (!state.metadata) return null;
    
    const cacheKey = `${state.currentLayerMode}-${state.currentEns}-${timeVal}-${state.metadata.version}`;
    
    if (state.textureCache[cacheKey]) {
        const entry = state.textureCache[cacheKey];
        if (!entry.loaded) {
            return null; // Still loading the image
        }
        if (state.currentLayerMode === 'wind' && timeVal === state.metadata.times[state.currentTimeIndex]) {
            if (state.windPixelData === null || activeWindCacheKey !== cacheKey) {
                activeWindCacheKey = cacheKey;
                windLayerInstance.updateWindPixelData(entry.image);
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
    const keys = Object.keys(state.textureCache);
    if (keys.length > 250) {
        const oldestKey = keys[0];
        const oldestEntry = state.textureCache[oldestKey];
        if (oldestEntry) {
            console.log(`Evicting cached texture: ${oldestKey}`);
            if (gl && oldestEntry.texture) {
                gl.deleteTexture(oldestEntry.texture);
            }
            delete state.textureCache[oldestKey];
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
    state.textureCache[cacheKey] = entry;
    
    console.log(`Starting image load for ${cacheKey}...`);
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
        console.log(`Image loaded successfully for ${cacheKey}.`);
        entry.image = img;
        entry.loaded = true;
        if (state.currentLayerMode === 'wind' && timeVal === state.metadata.times[state.currentTimeIndex]) {
            activeWindCacheKey = cacheKey;
            windLayerInstance.updateWindPixelData(img);
        }
        if (state.map) state.map.triggerRepaint();
    };
    img.onerror = (err) => {
        console.error(`Failed to load image for ${cacheKey}:`, err);
    };
    const srcPath = state.currentLayerMode === 'temp'
        ? `/api/data/temp/${timeVal}`
        : (state.currentLayerMode === 'wind' ? `/api/data/wind/${timeVal}` : `/api/data/${state.currentEns}/${timeVal}`);
    img.src = `${window.location.origin}${srcPath}?v=${state.metadata.version}`;
    
    return null;
}

// Web Mercator to Lat/Lon Projection
export function mercatorToLonLat(x, y) {
    const r_major = 6378137.0;
    const lon = (x / r_major) * (180.0 / Math.PI);
    const lat = (2.0 * Math.atan(Math.exp(y / r_major)) - Math.PI / 2.0) * (180.0 / Math.PI);
    return [lat, lon];
}

// Lat/Lon to Web Mercator Projection
export function lonLatToMercator(lat, lon) {
    const r_major = 6378137.0;
    const x = lon * (Math.PI / 180.0) * r_major;
    const y = Math.log(Math.tan((Math.PI / 4.0) + (lat * (Math.PI / 360.0)))) * r_major;
    return [x, y];
}

// Clear cached textures and release GPU memory
export function clearRadarLayers() {
    if (state.glContext) {
        for (const cacheKey in state.textureCache) {
            if (state.textureCache[cacheKey].texture) {
                state.glContext.deleteTexture(state.textureCache[cacheKey].texture);
            }
        }
    }
    state.textureCache = {};
    if (state.map) state.map.triggerRepaint();
}

// Add the custom WebGL layer to style
export function setupRadarSourceAndLayer() {
    if (!state.metadata || !state.map || !state.map.isStyleLoaded()) return;

    if (state.map.getLayer('radar-webgl-layer')) {
        state.map.removeLayer('radar-webgl-layer');
    }
    if (state.map.getLayer('wind-webgl-layer')) {
        state.map.removeLayer('wind-webgl-layer');
    }

    if (state.currentLayerMode === 'wind') {
        state.map.addLayer(windLayerInstance);
        windLayerInstance.initParticles();
        state.lastAnimTime = performance.now();
        state.map.triggerRepaint();
    } else {
        state.map.addLayer(radarLayerInstance);
    }
}

// Trigger map repaint and preload future frames
export function updateRadarOverlay() {
    if (!state.metadata || !state.map || !state.map.isStyleLoaded()) return;

    const activeLayerId = state.currentLayerMode === 'wind' ? 'wind-webgl-layer' : 'radar-webgl-layer';
    if (!state.map.getLayer(activeLayerId)) {
        setupRadarSourceAndLayer();
    }

    state.map.triggerRepaint();

    if (state.glContext) {
        const len = state.metadata.times.length;
        for (let i = 1; i <= CONFIG.cache.preloadAhead; i++) {
            const nextIndex = (state.currentTimeIndex + i) % len;
            const nextTimeVal = state.metadata.times[nextIndex];
            getOrLoadTexture(state.glContext, nextTimeVal);
        }
    }
}

let lastLat = null;
let lastLon = null;
let hoverTimeout = null;

// Throttled mouse listener on map
export function handleMapMouseMove(e) {
    const lat = e.lngLat.lat;
    const lon = e.lngLat.lng;
    lastLat = lat;
    lastLon = lon;

    // Show coordinates in panel
    DOM.hoverCoords.textContent = `lat: ${lastLat.toFixed(4)}, lon: ${lastLon.toFixed(4)}`;
    DOM.hoverPanel.classList.remove('glass-panel', 'hidden');
    DOM.hoverPanel.classList.add('glass-panel'); // Make sure it's shown

    // Throttle queries
    if (hoverTimeout) return;
    hoverTimeout = setTimeout(() => {
        hoverTimeout = null;
        triggerHoverQuery();
    }, CONFIG.intervals.hoverThrottleMs);
}

// Hide hover panel when mouse leaves map
export function handleMapMouseLeave() {
    DOM.hoverPanel.classList.add('hidden');
    lastLat = null;
    lastLon = null;
}

// Performs fetch to API value endpoint
export async function triggerHoverQuery() {
    if (lastLat === null || lastLon === null || !state.metadata) return;

    const timeVal = state.metadata.times[state.currentTimeIndex];
    if (state.currentLayerMode === 'temp') {
        try {
            const res = await fetchHoverValue(state.currentLayerMode, state.currentEns, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.value === null) {
                DOM.hoverValue.textContent = "No Data";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else {
                DOM.hoverValue.textContent = `${res.value.toFixed(1)} °C`;
                // Color code temperature hover value
                if (res.value < 0) DOM.hoverValue.style.color = "#38bdf8"; // Freezing: sky blue
                else if (res.value < 10) DOM.hoverValue.style.color = "#60a5fa"; // Cool: blue
                else if (res.value < 20) DOM.hoverValue.style.color = "#4ade80"; // Mild: green
                else if (res.value < 28) DOM.hoverValue.style.color = "#facc15"; // Warm: yellow
                else if (res.value < 33) DOM.hoverValue.style.color = "#fb923c"; // Hot: orange
                else DOM.hoverValue.style.color = "#f87171"; // Very hot: red
            }
        } catch (e) {
            console.error("Temp Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    } else if (state.currentLayerMode === 'wind') {
        try {
            const res = await fetchHoverValue(state.currentLayerMode, state.currentEns, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.speed === null) {
                DOM.hoverValue.textContent = "No Data";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else {
                const bft = mpsToBeaufort(res.speed);
                const cardinal = degreesToCardinal(res.direction);
                DOM.hoverValue.textContent = `${res.speed.toFixed(1)} m/s (${bft} Bft) ${cardinal}`;
                
                // Color code wind speed
                if (res.speed < 2.0) DOM.hoverValue.style.color = "#94a3b8"; // Calm: blue-gray
                else if (res.speed < 5.0) DOM.hoverValue.style.color = "#22d3ee"; // Light: cyan
                else if (res.speed < 10.0) DOM.hoverValue.style.color = "#4ade80"; // Moderate: green
                else if (res.speed < 15.0) DOM.hoverValue.style.color = "#facc15"; // Strong: yellow
                else if (res.speed < 20.0) DOM.hoverValue.style.color = "#fb923c"; // Gale: orange
                else DOM.hoverValue.style.color = "#f87171"; // Storm: red
            }
        } catch (e) {
            console.error("Wind Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    } else {
        try {
            const res = await fetchHoverValue(state.currentLayerMode, state.currentEns, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "no_rain" || res.value === 0.0) {
                if (state.currentEns === 'prob') {
                    DOM.hoverValue.textContent = "0% Chance";
                } else {
                    DOM.hoverValue.textContent = "0.00 mm/h";
                }
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "probability") {
                DOM.hoverValue.textContent = `${Math.round(res.value)}% Chance`;
                // Color code probability
                if (res.value < 30) DOM.hoverValue.style.color = "#94a3b8"; // Grey-blue
                else if (res.value < 70) DOM.hoverValue.style.color = "#3b82f6"; // Blue
                else DOM.hoverValue.style.color = "#a855f7"; // Purple / High probability
            } else {
                DOM.hoverValue.textContent = `${res.value.toFixed(2)} mm/h`;
                // Color code value dynamically in panel based on intensity
                if (res.value < 0.2) DOM.hoverValue.style.color = "#38bdf8"; // Light sky-blue
                else if (res.value < 1.0) DOM.hoverValue.style.color = "#60a5fa"; // Blue
                else if (res.value < 5.0) DOM.hoverValue.style.color = "#4ade80"; // Green
                else if (res.value < 15.0) DOM.hoverValue.style.color = "#facc15"; // Yellow
                else if (res.value < 30.0) DOM.hoverValue.style.color = "#fb923c"; // Orange
                else DOM.hoverValue.style.color = "#f87171"; // Red
            }
        } catch (e) {
            console.error("Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    }
}

// Map Click Listener
export function handleMapClick(e) {
    const lat = e.lngLat.lat;
    const lon = e.lngLat.lng;
    
    if (state.clickedMarker) {
        state.clickedMarker.setLngLat(e.lngLat);
    } else {
        state.clickedMarker = new maplibregl.Marker()
            .setLngLat(e.lngLat)
            .addTo(state.map);
    }
    
    showTimeseriesChart(lat, lon);
}

// Initialize MapLibre Map
export function initMap() {
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

    state.map = new maplibregl.Map({
        container: 'map',
        style: CONFIG.map.styles.dark,
        center: center,
        zoom: zoom,
        minZoom: CONFIG.map.minZoom,
        maxZoom: CONFIG.map.maxZoom
    });

    // Add map navigation controls (zoom, compass)
    state.map.addControl(new maplibregl.NavigationControl(), 'top-left');

    state.map.on('load', () => {
        setupRadarSourceAndLayer();
    });

    // Recreate custom WebGL layer on style changes
    state.map.on('style.load', () => {
        setupRadarSourceAndLayer();
    });

    // Attach map mouse events for hover & click
    state.map.on('mousemove', handleMapMouseMove);
    state.map.on('mouseout', handleMapMouseLeave);
    state.map.on('click', handleMapClick);

    // Sync viewport state to URL query parameters
    state.map.on('moveend', () => {
        const center = state.map.getCenter();
        const zoom = state.map.getZoom();
        
        const url = new URL(window.location.href);
        url.searchParams.set('lat', center.lat.toFixed(4));
        url.searchParams.set('lon', center.lng.toFixed(4));
        url.searchParams.set('zoom', zoom.toFixed(1));
        
        window.history.replaceState({}, '', url.pathname + url.search);
    });
}
