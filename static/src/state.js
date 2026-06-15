import { CONFIG } from './config.js';

export const state = {
    map: null,
    metadata: null,
    rainMetadata: null,
    tempMetadata: null,
    windMetadata: null,
    currentLayerMode: 'rain',
    currentEns: CONFIG.defaults.ensemble,
    selectedWindHeight: 10,
    currentTimeIndex: CONFIG.defaults.timeIndex,
    isPlaying: false,
    playInterval: null,
    clickedMarker: null,
    chartInstance: null,
    activeCoords: null,

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
    lastAnimTime: 0
};
