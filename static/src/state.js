import { CONFIG } from './config.js';

export const state = {
    map: null,
    metadata: null,
    rainMetadata: null,
    tempMetadata: null,
    windMetadata: null,
    solarMetadata: null,
    currentLayerMode: 'rain',
    currentEns: CONFIG.defaults.ensemble,
    selectedWindHeight: 10,
    currentTimeIndex: CONFIG.defaults.timeIndex,
    isPlaying: false,
    playInterval: null,
    clickedMarker: null,
    chartInstance: null,
    activeCoords: null,

    // Network & Error UI State
    isLoading: false,
    activeRequests: 0,
    currentError: null,

    // WebGL Custom Layer variables
    radarProgram: null,
    positionBuffer: null,
    texcoordBuffer: null,
    glContext: null,
    textureCache: {},

    // WebGL Wind Layer variables
    windProgram: null,
    windPositionBuffer: null,
    windTexcoordBuffer: null,
    particleProgram: null,
    particleBuffer: null,
    windPixelData: null, // Uint8ClampedArray for CPU particle lookups
    maxParticles: 3000,
    TRAIL_LENGTH: 24,
    particles: [],
    lastAnimTime: 0,

    // Compare Mode (Split Screen) variables
    isCompareModeActive: false,
    mapRight: null,
    compareLayerMode: 'temp',
    compareEns: 'med',
    compareSelectedWindHeight: 10,
    dividerPosition: 50, // percentage (0-100)
    textureCacheRight: {},
    radarProgramRight: null,
    positionBufferRight: null,
    texcoordBufferRight: null,
    glContextRight: null,
    windProgramRight: null,
    windPositionBufferRight: null,
    windTexcoordBufferRight: null,
    particleProgramRight: null,
    particleBufferRight: null,
    windPixelDataRight: null,
    particlesRight: []
};

export function syncStateToURL() {
    const url = new URL(window.location.href);
    
    // Viewport
    if (state.map) {
        try {
            const center = state.map.getCenter();
            const zoom = state.map.getZoom();
            url.searchParams.set('lat', center.lat.toFixed(4));
            url.searchParams.set('lon', center.lng.toFixed(4));
            url.searchParams.set('zoom', zoom.toFixed(1));
        } catch (e) {
            // Map might not be fully initialized or getCenter/getZoom failed
        }
    }
    
    // Options
    if (state.currentLayerMode) {
        url.searchParams.set('mode', state.currentLayerMode);
    }
    if (state.currentEns !== undefined && state.currentEns !== null) {
        url.searchParams.set('ens', state.currentEns.toString());
    }
    if (state.selectedWindHeight) {
        url.searchParams.set('height', state.selectedWindHeight.toString());
    }
    
    // Selected Location
    if (state.activeCoords) {
        url.searchParams.set('slat', state.activeCoords.lat.toFixed(4));
        url.searchParams.set('slon', state.activeCoords.lon.toFixed(4));
    } else {
        url.searchParams.delete('slat');
        url.searchParams.delete('slon');
    }
    
    window.history.replaceState({}, '', url.pathname + url.search);
}

export function parseURLState() {
    const urlParams = new URLSearchParams(window.location.search);
    
    const mode = urlParams.get('mode');
    if (mode && ['rain', 'temp', 'wind', 'solar'].includes(mode)) {
        state.currentLayerMode = mode;
    }
    
    const ens = urlParams.get('ens');
    if (ens) {
        if (!isNaN(ens)) {
            state.currentEns = parseInt(ens);
        } else if (['pmm', 'med', 'max', 'prob', 'spread'].includes(ens)) {
            state.currentEns = ens;
        }
    }
    
    const height = urlParams.get('height');
    if (height && ['10', '50', '100', '200', '300'].includes(height)) {
        state.selectedWindHeight = parseInt(height);
    }
}
