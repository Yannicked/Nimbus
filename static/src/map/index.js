import { CONFIG } from '../config.js';
import { state, syncStateToURL } from '../state.js';
import { DOM } from '../ui/dom.js';
import { fetchHoverValue } from '../api.js';
import { mpsToBeaufort, degreesToCardinal, showTimeseriesChart } from '../ui/chart.js';
import { WebGLRadarLayer } from './WebGLRadar.js';
import { WebGLWindLayer } from './WebGLWind.js';

export let radarLayerInstance = new WebGLRadarLayer();
export let windLayerInstance = new WebGLWindLayer();

export let radarLayerInstanceRight = new WebGLRadarLayer('radar-webgl-layer-right', true);
export let windLayerInstanceRight = new WebGLWindLayer('wind-webgl-layer-right', true);

let activeWindCacheKey = null;
let activeWindCacheKeyRight = null;

// Helper to load/bind WebGL textures asynchronously
export function getOrLoadTexture(gl, timeVal, isCompare = false) {
    const layerMode = isCompare ? state.compareLayerMode : state.currentLayerMode;
    const ens = isCompare ? state.compareEns : state.currentEns;
    const windHeight = isCompare ? state.compareSelectedWindHeight : state.selectedWindHeight;
    const cache = isCompare ? state.textureCacheRight : state.textureCache;
    const metadata = isCompare ? (layerMode === 'temp' ? state.tempMetadata : (layerMode === 'solar' ? state.solarMetadata : (layerMode === 'wind' ? state.windMetadata : state.rainMetadata))) : state.metadata;

    if (!metadata) return null;
    
    const cacheKey = `${layerMode}-${ens}-${timeVal}-${metadata.version}`;
    
    if (cache[cacheKey]) {
        const entry = cache[cacheKey];
        if (!entry.loaded) {
            return null; // Still loading the image
        }
        if (layerMode === 'wind' && timeVal === metadata.times[state.currentTimeIndex]) {
            if (isCompare) {
                if (state.windPixelDataRight === null || activeWindCacheKeyRight !== cacheKey) {
                    activeWindCacheKeyRight = cacheKey;
                    windLayerInstanceRight.updateWindPixelData(entry.image);
                }
            } else {
                if (state.windPixelData === null || activeWindCacheKey !== cacheKey) {
                    activeWindCacheKey = cacheKey;
                    windLayerInstance.updateWindPixelData(entry.image);
                }
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
    const keys = Object.keys(cache);
    if (keys.length > 250) {
        const oldestKey = keys[0];
        const oldestEntry = cache[oldestKey];
        if (oldestEntry) {
            console.log(`Evicting cached texture: ${oldestKey}`);
            if (gl && oldestEntry.texture) {
                gl.deleteTexture(oldestEntry.texture);
            }
            delete cache[oldestKey];
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
    cache[cacheKey] = entry;
    
    console.log(`Starting image load for ${cacheKey}...`);
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
        console.log(`Image loaded successfully for ${cacheKey}.`);
        entry.image = img;
        entry.loaded = true;
        if (layerMode === 'wind' && timeVal === metadata.times[state.currentTimeIndex]) {
            if (isCompare) {
                activeWindCacheKeyRight = cacheKey;
                windLayerInstanceRight.updateWindPixelData(img);
            } else {
                activeWindCacheKey = cacheKey;
                windLayerInstance.updateWindPixelData(img);
            }
        }
        const mapObj = isCompare ? state.mapRight : state.map;
        if (mapObj) mapObj.triggerRepaint();
    };
    img.onerror = (err) => {
        console.error(`Failed to load image for ${cacheKey}:`, err);
    };
    const srcPath = layerMode === 'temp'
        ? `/api/data/temp/${timeVal}`
        : (layerMode === 'solar' ? `/api/data/solar/${timeVal}` : (layerMode === 'wind' ? `/api/data/wind/${windHeight}/${timeVal}` : `/api/data/${ens}/${timeVal}`));
    img.src = `${window.location.origin}${srcPath}?v=${metadata.version}`;
    
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

    if (state.glContextRight) {
        for (const cacheKey in state.textureCacheRight) {
            if (state.textureCacheRight[cacheKey].texture) {
                state.glContextRight.deleteTexture(state.textureCacheRight[cacheKey].texture);
            }
        }
    }
    state.textureCacheRight = {};

    if (state.map) state.map.triggerRepaint();
    if (state.mapRight) state.mapRight.triggerRepaint();
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

// Add the custom WebGL layer to the right style
export function setupRadarSourceAndLayerRight() {
    if (!state.mapRight || !state.mapRight.isStyleLoaded()) return;

    const layerMode = state.compareLayerMode;
    const rightMetadata = layerMode === 'temp' ? state.tempMetadata : (layerMode === 'solar' ? state.solarMetadata : (layerMode === 'wind' ? state.windMetadata : state.rainMetadata));
    if (!rightMetadata) return;

    if (state.mapRight.getLayer('radar-webgl-layer-right')) {
        state.mapRight.removeLayer('radar-webgl-layer-right');
    }
    if (state.mapRight.getLayer('wind-webgl-layer-right')) {
        state.mapRight.removeLayer('wind-webgl-layer-right');
    }

    if (layerMode === 'wind') {
        state.mapRight.addLayer(windLayerInstanceRight);
        windLayerInstanceRight.initParticles();
        state.mapRight.triggerRepaint();
    } else {
        state.mapRight.addLayer(radarLayerInstanceRight);
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
            getOrLoadTexture(state.glContext, nextTimeVal, false);
        }
    }

    if (state.isCompareModeActive && state.mapRight && state.mapRight.isStyleLoaded()) {
        const activeLayerIdRight = state.compareLayerMode === 'wind' ? 'wind-webgl-layer-right' : 'radar-webgl-layer-right';
        if (!state.mapRight.getLayer(activeLayerIdRight)) {
            setupRadarSourceAndLayerRight();
        }

        state.mapRight.triggerRepaint();

        if (state.glContextRight) {
            const layerMode = state.compareLayerMode;
            const rightMetadata = layerMode === 'temp' ? state.tempMetadata : (layerMode === 'solar' ? state.solarMetadata : (layerMode === 'wind' ? state.windMetadata : state.rainMetadata));
            if (rightMetadata) {
                const lenRight = rightMetadata.times.length;
                for (let i = 1; i <= CONFIG.cache.preloadAhead; i++) {
                    const nextIndex = (state.currentTimeIndex + i) % lenRight;
                    const nextTimeVal = rightMetadata.times[nextIndex];
                    getOrLoadTexture(state.glContextRight, nextTimeVal, true);
                }
            }
        }
    }
}

let lastLat = null;
let lastLon = null;
let hoverTimeout = null;
let isHoveringRightGlobal = false;

// Throttled mouse listener on map
export function handleMapMouseMove(e) {
    const lat = e.lngLat.lat;
    const lon = e.lngLat.lng;
    lastLat = lat;
    lastLon = lon;

    isHoveringRightGlobal = false;
    if (state.isCompareModeActive) {
        const parent = DOM.swipeDivider.parentElement;
        if (parent) {
            const rect = parent.getBoundingClientRect();
            const dividerX = (state.dividerPosition / 100) * rect.width;
            const mouseX = e.originalEvent.clientX - rect.left;
            if (mouseX > dividerX) {
                isHoveringRightGlobal = true;
            }
        }
    }

    // Show coordinates in panel
    DOM.hoverCoords.textContent = `lat: ${lastLat.toFixed(4)}, lon: ${lastLon.toFixed(4)}`;
    DOM.hoverPanel.classList.remove('glass-panel', 'hidden');
    DOM.hoverPanel.classList.add('glass-panel');

    // Update hover label immediately based on side
    const mode = isHoveringRightGlobal ? state.compareLayerMode : state.currentLayerMode;
    const windHeight = isHoveringRightGlobal ? state.compareSelectedWindHeight : state.selectedWindHeight;
    const prefix = state.isCompareModeActive ? (isHoveringRightGlobal ? "RIGHT: " : "LEFT: ") : "";
    DOM.hoverLabel.textContent = prefix + (mode === 'temp' ? 'TEMPERATURE' : (mode === 'solar' ? 'SOLAR RADIATION' : (mode === 'wind' ? `${windHeight}M WIND` : 'PRECIPITATION')));

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

    const isRight = isHoveringRightGlobal && state.isCompareModeActive;
    const mode = isRight ? state.compareLayerMode : state.currentLayerMode;
    const ens = isRight ? state.compareEns : state.currentEns;
    const windHeight = isRight ? state.compareSelectedWindHeight : state.selectedWindHeight;
    const metadata = isRight ? (mode === 'temp' ? state.tempMetadata : (mode === 'solar' ? state.solarMetadata : (mode === 'wind' ? state.windMetadata : state.rainMetadata))) : state.metadata;

    if (!metadata) return;
    const timeVal = metadata.times[state.currentTimeIndex];

    // Ensure hover label matches the current queried mode and side
    const prefix = state.isCompareModeActive ? (isRight ? "RIGHT: " : "LEFT: ") : "";
    DOM.hoverLabel.textContent = prefix + (mode === 'temp' ? 'TEMPERATURE' : (mode === 'solar' ? 'SOLAR RADIATION' : (mode === 'wind' ? `${windHeight}M WIND` : 'PRECIPITATION')));

    if (mode === 'temp') {
        try {
            const res = await fetchHoverValue(mode, ens, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.value === null) {
                DOM.hoverValue.textContent = "No Data";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else {
                DOM.hoverValue.textContent = `${res.value.toFixed(1)} °C`;
                if (res.value < 0) DOM.hoverValue.style.color = "#38bdf8";
                else if (res.value < 10) DOM.hoverValue.style.color = "#60a5fa";
                else if (res.value < 20) DOM.hoverValue.style.color = "#4ade80";
                else if (res.value < 28) DOM.hoverValue.style.color = "#facc15";
                else if (res.value < 33) DOM.hoverValue.style.color = "#fb923c";
                else DOM.hoverValue.style.color = "#f87171";
            }
        } catch (e) {
            console.error("Temp Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    } else if (mode === 'solar') {
        try {
            const res = await fetchHoverValue(mode, ens, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.value === null) {
                DOM.hoverValue.textContent = "No Data";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else {
                DOM.hoverValue.textContent = `${Math.round(res.value)} W/m²`;
                if (res.value < 50) DOM.hoverValue.style.color = "#94a3b8";
                else if (res.value < 200) DOM.hoverValue.style.color = "#fef08a";
                else if (res.value < 500) DOM.hoverValue.style.color = "#facc15";
                else if (res.value < 800) DOM.hoverValue.style.color = "#f97316";
                else DOM.hoverValue.style.color = "#ef4444";
            }
        } catch (e) {
            console.error("Solar Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    } else if (mode === 'wind') {
        try {
            const url = `/api/value/wind?time=${timeVal}&lat=${lastLat}&lon=${lastLon}&height=${windHeight}`;
            const response = await fetch(url);
            if (!response.ok) throw new Error("Wind Hover query failed");
            const res = await response.json();

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
                
                if (res.speed < 2.0) DOM.hoverValue.style.color = "#94a3b8";
                else if (res.speed < 5.0) DOM.hoverValue.style.color = "#22d3ee";
                else if (res.speed < 10.0) DOM.hoverValue.style.color = "#4ade80";
                else if (res.speed < 15.0) DOM.hoverValue.style.color = "#facc15";
                else if (res.speed < 20.0) DOM.hoverValue.style.color = "#fb923c";
                else DOM.hoverValue.style.color = "#f87171";
            }
        } catch (e) {
            console.error("Wind Hover error:", e);
            DOM.hoverValue.textContent = "Error";
            DOM.hoverValue.style.color = "#f87171";
        }
    } else {
        try {
            const res = await fetchHoverValue(mode, ens, timeVal, lastLat, lastLon);

            if (res.status === "out_of_bounds") {
                DOM.hoverValue.textContent = "Out of Grid";
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "no_rain" || res.value === 0.0) {
                if (ens === 'prob') {
                    DOM.hoverValue.textContent = "0% Chance";
                } else {
                    DOM.hoverValue.textContent = "0.00 mm/h";
                }
                DOM.hoverValue.style.color = "var(--text-secondary)";
            } else if (res.status === "probability") {
                DOM.hoverValue.textContent = `${Math.round(res.value)}% Chance`;
                if (res.value < 30) DOM.hoverValue.style.color = "#94a3b8";
                else if (res.value < 70) DOM.hoverValue.style.color = "#3b82f6";
                else DOM.hoverValue.style.color = "#a855f7";
            } else {
                DOM.hoverValue.textContent = `${res.value.toFixed(2)} mm/h`;
                if (res.value < 0.2) DOM.hoverValue.style.color = "#38bdf8";
                else if (res.value < 1.0) DOM.hoverValue.style.color = "#60a5fa";
                else if (res.value < 5.0) DOM.hoverValue.style.color = "#4ade80";
                else if (res.value < 15.0) DOM.hoverValue.style.color = "#facc15";
                else if (res.value < 30.0) DOM.hoverValue.style.color = "#fb923c";
                else DOM.hoverValue.style.color = "#f87171";
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
        ? [initialLon, initialLat] 
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

    state.map.addControl(new maplibregl.NavigationControl(), 'top-left');

    state.map.on('load', () => {
        setupRadarSourceAndLayer();
    });

    state.map.on('style.load', () => {
        setupRadarSourceAndLayer();
    });

    state.map.on('mousemove', handleMapMouseMove);
    state.map.on('mouseout', handleMapMouseLeave);
    state.map.on('click', handleMapClick);

    state.map.on('moveend', () => {
        syncStateToURL();
    });
}

// Initialize right-hand compare map
export function initMapRight() {
    if (state.mapRight) return;

    state.mapRight = new maplibregl.Map({
        container: 'map-right',
        style: state.map ? state.map.getStyle() : CONFIG.map.styles.dark,
        center: state.map ? state.map.getCenter() : CONFIG.map.defaultCenter,
        zoom: state.map ? state.map.getZoom() : CONFIG.map.defaultZoom,
        bearing: state.map ? state.map.getBearing() : 0,
        pitch: state.map ? state.map.getPitch() : 0,
        minZoom: CONFIG.map.minZoom,
        maxZoom: CONFIG.map.maxZoom,
        attributionControl: false
    });

    state.mapRight.on('load', () => {
        setupRadarSourceAndLayerRight();
        updateRadarOverlay();
    });

    state.mapRight.on('style.load', () => {
        setupRadarSourceAndLayerRight();
        updateRadarOverlay();
    });

    state.mapRight.on('mousemove', handleMapMouseMove);
    state.mapRight.on('mouseout', handleMapMouseLeave);
    state.mapRight.on('click', handleMapClick);
}

let leftMoveListener = null;
let rightMoveListener = null;
let isSyncing = false;

export function enableMapSync() {
    if (!state.map || !state.mapRight) return;
    
    isSyncing = false;
    
    leftMoveListener = () => {
        if (isSyncing) return;
        isSyncing = true;
        state.mapRight.jumpTo({
            center: state.map.getCenter(),
            zoom: state.map.getZoom(),
            bearing: state.map.getBearing(),
            pitch: state.map.getPitch()
        });
        isSyncing = false;
    };
    
    rightMoveListener = () => {
        if (isSyncing) return;
        isSyncing = true;
        state.map.jumpTo({
            center: state.mapRight.getCenter(),
            zoom: state.mapRight.getZoom(),
            bearing: state.mapRight.getBearing(),
            pitch: state.mapRight.getPitch()
        });
        isSyncing = false;
    };
    
    state.map.on('move', leftMoveListener);
    state.mapRight.on('move', rightMoveListener);
}

export function disableMapSync() {
    if (state.map && leftMoveListener) {
        state.map.off('move', leftMoveListener);
        leftMoveListener = null;
    }
    if (state.mapRight && rightMoveListener) {
        state.mapRight.off('move', rightMoveListener);
        rightMoveListener = null;
    }
}

export function switchMapStyle(styleKey) {
    if (CONFIG.map.styles[styleKey]) {
        if (state.map) {
            state.map.setStyle(CONFIG.map.styles[styleKey]);
        }
        if (state.mapRight) {
            state.mapRight.setStyle(CONFIG.map.styles[styleKey]);
        }
    }
}
