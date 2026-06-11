// App Variables
let map;
let metadata = null;
let currentEns = 1;
let currentTimeIndex = 0;
let isPlaying = false;
let playInterval = null;
let radarOverlay = null;

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
const ensembleGrid = document.getElementById('ensemble-grid');
const hoverPanel = document.getElementById('hover-panel');
const hoverValue = document.getElementById('hover-value');
const hoverCoords = document.getElementById('hover-coords');

// Web Mercator to Lat/Lon Projection
function mercatorToLonLat(x, y) {
    const r_major = 6378137.0;
    const lon = (x / r_major) * (180.0 / Math.PI);
    const lat = (2.0 * Math.atan(Math.exp(y / r_major)) - Math.PI / 2.0) * (180.0 / Math.PI);
    return [lat, lon];
}

// Format relative time step
function formatRelativeTime(seconds) {
    const mins = Math.round(seconds / 60);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    if (h > 0) {
        return `+${h}h ${m.toString().padStart(2, '0')}m`;
    }
    return `+${m}m`;
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
    }) + ' (NL Time)';
}

// Initialize Leaflet Map
function initMap() {
    map = L.map('map', {
        center: [52.1, 5.2], // Center on Netherlands
        zoom: 7,
        minZoom: 6,
        maxZoom: 10
    });

    // Base Layer: CartoDB Dark Matter (Recommended)
    const darkLayer = L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png', {
        attribution: '© <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors © <a href="https://carto.com/attributions">CARTO</a>',
        subdomains: 'abcd',
        maxZoom: 20
    }).addTo(map);

    // Base Layer: Standard OpenStreetMap
    const osmLayer = L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
        attribution: '© <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
        maxZoom: 19
    });

    // Layer Controls
    const baseMaps = {
        "Dark Theme (OSM)": darkLayer,
        "Standard OpenStreetMap": osmLayer
    };
    L.control.layers(baseMaps).addTo(map);
}

// Fetch Metadata and Load App
async function loadApp() {
    try {
        const response = await fetch('/api/metadata');
        if (!response.ok) throw new Error("Metadata request failed");
        metadata = await response.json();
        
        // Display reference time
        refTimeVal.textContent = metadata.reference_time_str;

        // Create Ensemble Selector Buttons
        ensembleGrid.innerHTML = '';
        
        // Add E1-E20
        metadata.ensembles.forEach(ens => {
            const btn = document.createElement('button');
            btn.textContent = `E${ens}`;
            if (ens === currentEns) btn.classList.add('active');
            btn.addEventListener('click', () => selectEnsemble(ens));
            ensembleGrid.appendChild(btn);
        });

        // Add special statistics buttons
        const stats = ['med', 'max', 'prob'];
        const statLabels = { 'med': 'MED', 'max': 'MAX', 'prob': 'PROB' };
        stats.forEach(stat => {
            const btn = document.createElement('button');
            btn.textContent = statLabels[stat];
            btn.classList.add('stat-btn');
            if (stat === currentEns) btn.classList.add('active');
            btn.addEventListener('click', () => selectEnsemble(stat));
            ensembleGrid.appendChild(btn);
        });

        // Initialize Timeline Slider
        timeSlider.min = 0;
        timeSlider.max = metadata.times.length - 1;
        timeSlider.value = currentTimeIndex;

        // Draw ticks on timeline
        drawSliderTicks();

        // Load initial radar overlay
        updateRadarOverlay();
        updateTimeStepDisplay();

    } catch (e) {
        console.error(e);
        refTimeVal.textContent = "Error loading data!";
    }
}

// Render ticks on the timeline slider
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

// Update the map ImageOverlay
function updateRadarOverlay() {
    if (!metadata) return;

    const timeVal = metadata.times[currentTimeIndex];
    const imageUrl = `/api/map/${currentEns}/${timeVal}`;

    // Compute bounding box coordinates in Lat/Lon
    const sw = mercatorToLonLat(metadata.left, metadata.bottom);
    const ne = mercatorToLonLat(metadata.right, metadata.top);
    const imageBounds = [sw, ne];

    const opacity = parseFloat(opacitySlider.value) / 100;

    if (radarOverlay) {
        // Update existing overlay URL and bounds
        radarOverlay.setUrl(imageUrl);
        radarOverlay.setOpacity(opacity);
    } else {
        // Create new overlay
        radarOverlay = L.imageOverlay(imageUrl, imageBounds, {
            opacity: opacity,
            interactive: false // We capture clicks/moves on map level for hover value queries
        }).addTo(map);
    }
}

// Update time text displays
function updateTimeStepDisplay() {
    if (!metadata) return;
    const timeVal = metadata.times[currentTimeIndex];
    currentTimeStep.textContent = formatAbsoluteTime(metadata.reference_time_str, timeVal);
    timeStepRelative.textContent = formatRelativeTime(timeVal);
}

// Handle Ensemble Switch
function selectEnsemble(ens) {
    currentEns = ens;
    document.querySelectorAll('.ensemble-grid button').forEach((btn) => {
        const text = btn.textContent;
        const isTarget = (ens === 'med' && text === 'MED') ||
                         (ens === 'max' && text === 'MAX') ||
                         (ens === 'prob' && text === 'PROB') ||
                         (typeof ens === 'number' && text === `E${ens}`);
        
        if (isTarget) {
            btn.classList.add('active');
        } else {
            btn.classList.remove('active');
        }
    });
    updateRadarOverlay();
    triggerHoverQuery(); // update hover panel if mouse is over map
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
    if (radarOverlay) {
        radarOverlay.setOpacity(parseFloat(val) / 100);
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
    const latlng = e.latlng;
    lastLat = latlng.lat;
    lastLon = latlng.lng;

    // Show coordinates in panel
    hoverCoords.textContent = `lat: ${lastLat.toFixed(4)}, lon: ${lastLon.toFixed(4)}`;
    hoverPanel.classList.remove('glass-panel', 'hidden');
    hoverPanel.classList.add('glass-panel'); // Make sure it's shown

    // Throttle queries to 100ms
    if (hoverTimeout) return;
    hoverTimeout = setTimeout(() => {
        hoverTimeout = null;
        triggerHoverQuery();
    }, 100);
}

// Hide hover panel when mouse leaves map
function handleMapMouseOut() {
    hoverPanel.classList.add('hidden');
    lastLat = null;
    lastLon = null;
}

// Performs fetch to API value endpoint
async function triggerHoverQuery() {
    if (lastLat === null || lastLon === null || !metadata) return;

    const timeVal = metadata.times[currentTimeIndex];
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

// App Entry Point
window.addEventListener('DOMContentLoaded', () => {
    initMap();
    loadApp();

    // Attach map event listeners
    map.on('mousemove', handleMapMouseMove);
    map.on('mouseout', handleMapMouseOut);
});
