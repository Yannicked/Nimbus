// App Variables
let map;
let metadata = null;
let currentEns = 'med';
let currentTimeIndex = 0;
let isPlaying = false;
let playInterval = null;
let radarLayers = {}; // Key: timeVal, Value: L.tileLayer
let activeTimeVal = null;
let clickedMarker = null;
let chartInstance = null;
let activeCoords = null;

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
        zoom: 8,
        minZoom: 0,
        maxZoom: 12
    });

    // Create custom pane for radar overlays to keep them on top of base layers
    map.createPane('radarPane');
    map.getPane('radarPane').style.zIndex = 450;
    map.getPane('radarPane').style.pointerEvents = 'none';

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
        
        metadata.ensembles.forEach(ens => {
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

// Helper to hide all inactive radar tile layers
function hideInactiveLayers(activeTimeVal) {
    for (const timeVal in radarLayers) {
        if (parseInt(timeVal) !== activeTimeVal) {
            radarLayers[timeVal].setOpacity(0);
        }
    }
}

// Clear all active layers when ensemble member changes
function clearRadarLayers() {
    for (const timeVal in radarLayers) {
        map.removeLayer(radarLayers[timeVal]);
    }
    radarLayers = {};
    activeTimeVal = null;
}

// Update the map TileOverlay using a sliding-window layer pool with background preloading
function updateRadarOverlay() {
    if (!metadata) return;

    const timeVal = metadata.times[currentTimeIndex];
    const opacity = parseFloat(opacitySlider.value) / 100;
    const urlTemplate = `/api/map/${currentEns}/${timeVal}/{z}/{x}/{y}?v=${metadata.version || 0}`;

    // Compute bounding box coordinates in Lat/Lon for bounds restriction
    const sw = mercatorToLonLat(metadata.left, metadata.bottom);
    const ne = mercatorToLonLat(metadata.right, metadata.top);
    const bounds = [sw, ne];

    activeTimeVal = timeVal;

    // 1. If layer for this timeVal doesn't exist, create it in background
    if (!radarLayers[timeVal]) {
        const layer = L.tileLayer(urlTemplate, {
            pane: 'radarPane',
            opacity: 0, // start hidden to avoid grey flashes
            bounds: bounds,
            minZoom: 0,
            maxZoom: 12,
            tileSize: 256,
            updateWhenIdle: false
        }).addTo(map);

        layer._isLoaded = false;
        layer.on('load', () => {
            layer._isLoaded = true;
            // Only show if this timeVal is still the active one
            if (activeTimeVal === timeVal) {
                layer.setOpacity(opacity);
                hideInactiveLayers(timeVal);
            }
        });

        radarLayers[timeVal] = layer;
    } else {
        // 2. If it already exists, show it immediately
        const layer = radarLayers[timeVal];
        layer.setOpacity(opacity);
        hideInactiveLayers(timeVal);
    }

    // 3. Active Preloading: load the next 2 frames in the background
    for (let i = 1; i <= 2; i++) {
        const nextIndex = (currentTimeIndex + i) % metadata.times.length;
        const nextTimeVal = metadata.times[nextIndex];
        if (!radarLayers[nextTimeVal]) {
            const nextUrlTemplate = `/api/map/${currentEns}/${nextTimeVal}/{z}/{x}/{y}?v=${metadata.version || 0}`;
            const nextLayer = L.tileLayer(nextUrlTemplate, {
                pane: 'radarPane',
                opacity: 0, // Keep hidden in the background
                bounds: bounds,
                minZoom: 0,
                maxZoom: 12,
                tileSize: 256,
                updateWhenIdle: false
            }).addTo(map);

            nextLayer._isLoaded = false;
            nextLayer.on('load', () => {
                nextLayer._isLoaded = true;
            });

            radarLayers[nextTimeVal] = nextLayer;
        }
    }

    // 4. Sliding-window Garbage Collection: prune layers outside the window
    // Keep window: [currentTimeIndex - 2, currentTimeIndex + 4] (with circular wrapping)
    const keepIndices = new Set();
    const len = metadata.times.length;
    for (let i = -2; i <= 4; i++) {
        const idx = (currentTimeIndex + i + len) % len;
        keepIndices.add(metadata.times[idx]);
    }

    for (const tVal in radarLayers) {
        const tValInt = parseInt(tVal);
        if (!keepIndices.has(tValInt)) {
            map.removeLayer(radarLayers[tVal]);
            delete radarLayers[tVal];
        }
    }
}

// Update time text displays
function updateTimeStepDisplay() {
    if (!metadata) return;
    const timeVal = metadata.times[currentTimeIndex];
    currentTimeStep.textContent = formatAbsoluteTime(metadata.reference_time_str, timeVal);
    timeStepRelative.textContent = formatRelativeTime(timeVal);
}

// Update the legend colors and labels dynamically
function updateLegend() {
    const legendTitle = document.querySelector('.legend-container .section-label');
    const legendBar = document.querySelector('.legend-bar');
    const legendLabels = document.querySelector('.legend-labels');
    if (!legendTitle || !legendBar || !legendLabels) return;

    if (currentEns === 'prob') {
        legendTitle.textContent = "Rain Probability";
        legendBar.innerHTML = `
            <span style="background: rgba(180, 200, 220, 0.35);"></span>
            <span style="background: rgba(100, 160, 255, 0.5);"></span>
            <span style="background: rgba(0, 100, 255, 0.65);"></span>
            <span style="background: rgba(0, 200, 100, 0.75);"></span>
            <span style="background: rgba(220, 0, 220, 0.85);"></span>
            <span style="background: rgba(255, 255, 255, 0.95);"></span>
        `;
        legendLabels.innerHTML = `
            <span>10%</span>
            <span>30%</span>
            <span>50%</span>
            <span>70%</span>
            <span>90%</span>
            <span>100%</span>
        `;
    } else {
        legendTitle.textContent = "Rainfall Rate (mm/h)";
        legendBar.innerHTML = `
            <span style="background: rgba(120, 200, 255, 0.5);"></span>
            <span style="background: rgba(0, 100, 255, 0.7);"></span>
            <span style="background: rgba(0, 200, 0, 0.7);"></span>
            <span style="background: rgba(255, 230, 0, 0.8);"></span>
            <span style="background: rgba(255, 120, 0, 0.9);"></span>
            <span style="background: rgba(255, 0, 0, 0.95);"></span>
            <span style="background: rgba(200, 0, 200, 1.0);"></span>
            <span style="background: rgba(255, 255, 255, 1.0);"></span>
        `;
        legendLabels.innerHTML = `
            <span>0.05</span>
            <span>0.2</span>
            <span>1</span>
            <span>5</span>
            <span>15</span>
            <span>30</span>
            <span>100</span>
            <span>250+</span>
        `;
    }
}

// Handle Ensemble Switch
function selectEnsemble(ens) {
    currentEns = ens;
    clearRadarLayers();
    
    // Update dropdown value if it differs
    if (ensembleSelect.value !== ens.toString()) {
        ensembleSelect.value = ens.toString();
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
    const activeLayer = radarLayers[activeTimeVal];
    if (activeLayer) {
        activeLayer.setOpacity(parseFloat(val) / 100);
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

// Poll for metadata updates every 5 seconds to detect new NetCDF file
function startMetadataPolling() {
    setInterval(async () => {
        try {
            const response = await fetch('/api/metadata');
            if (!response.ok) return;
            const newMetadata = await response.json();

            if (metadata && newMetadata.version !== metadata.version) {
                console.log("New NetCDF file detected! Reloading metadata and invalidating cache...");
                metadata = newMetadata;

                // Invalidate everything on client
                clearRadarLayers();

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
                refTimeVal.textContent = metadata.reference_time_str;
            }
        } catch (e) {
            console.error("Failed to check for metadata update:", e);
        }
    }, 5000);
}

async function showTimeseriesChart(lat, lon) {
    if (!metadata) return;
    activeCoords = { lat, lon };
    
    // Show the panel
    chartPanel.classList.remove('hidden');
    chartCoords.textContent = `lat: ${lat.toFixed(4)}, lon: ${lon.toFixed(4)}`;
    
    try {
        const url = `/api/timeseries?ens=${currentEns}&lat=${lat}&lon=${lon}`;
        const res = await fetch(url);
        if (!res.ok) throw new Error("Timeseries request failed");
        const data = await res.json();
        
        if (data.status === "out_of_bounds" || data.values.length === 0) {
            chartCoords.textContent = "Selected point is out of radar bounds";
            if (chartInstance) {
                chartInstance.destroy();
                chartInstance = null;
            }
            chartStatPeak.textContent = "-- mm/h";
            chartStatTotal.textContent = "-- mm";
            return;
        }
        
        const peakVal = Math.max(...data.values);
        let totalVal = 0.0;
        
        if (currentEns === 'prob') {
            chartStatPeak.textContent = `${Math.round(peakVal)}%`;
            const avgVal = data.values.reduce((a, b) => a + b, 0) / data.values.length;
            chartStatTotal.textContent = `${Math.round(avgVal)}% (avg)`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Peak Probability";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Avg Probability";
        } else {
            // total_mm = sum(rates) / 12 (5 mins intervals)
            totalVal = data.values.reduce((a, b) => a + b, 0) / 12.0;
            chartStatPeak.textContent = `${peakVal.toFixed(2)} mm/h`;
            chartStatTotal.textContent = `${totalVal.toFixed(2)} mm`;
            
            document.querySelector('.stat-box:nth-child(1) .stat-label').textContent = "Peak Intensity";
            document.querySelector('.stat-box:nth-child(2) .stat-label').textContent = "Total Accumulation";
        }
        
        const labels = data.times.map(secs => {
            const timeStr = formatAbsoluteTime(metadata.reference_time_str, secs);
            const match = timeStr.match(/(\d{2}:\d{2})/);
            return match ? match[1] : `+${Math.round(secs/60)}m`;
        });
        
        const isProb = currentEns === 'prob';
        const labelText = isProb ? "Rain Probability (%)" : "Rainfall Rate (mm/h)";
        const borderColor = isProb ? "#a855f7" : "#38bdf8";
        const backgroundColor = isProb ? "rgba(168, 85, 247, 0.15)" : "rgba(56, 189, 248, 0.15)";
        
        const ctx = document.getElementById('rainfall-chart').getContext('2d');
        
        if (chartInstance) {
            chartInstance.data.labels = labels;
            chartInstance.data.datasets[0].label = labelText;
            chartInstance.data.datasets[0].data = data.values;
            chartInstance.data.datasets[0].borderColor = borderColor;
            chartInstance.data.datasets[0].backgroundColor = backgroundColor;
            chartInstance.options.scales.y.title.text = labelText;
            chartInstance.options.scales.y.max = isProb ? 100 : undefined;
            chartInstance.update();
        } else {
            chartInstance = new Chart(ctx, {
                type: 'line',
                data: {
                    labels: labels,
                    datasets: [{
                        label: labelText,
                        data: data.values,
                        borderColor: borderColor,
                        backgroundColor: backgroundColor,
                        borderWidth: 2,
                        fill: true,
                        tension: 0.3,
                        pointRadius: 0,
                        pointHoverRadius: 4
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
                                maxTicksLimit: 6
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
                            min: 0,
                            max: isProb ? 100 : undefined
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

function closeTimeseriesChart() {
    chartPanel.classList.add('hidden');
    activeCoords = null;
    
    if (clickedMarker) {
        map.removeLayer(clickedMarker);
        clickedMarker = null;
    }
    
    if (chartInstance) {
        chartInstance.destroy();
        chartInstance = null;
    }
}

function handleMapClick(e) {
    const lat = e.latlng.lat;
    const lon = e.latlng.lng;
    
    if (clickedMarker) {
        clickedMarker.setLatLng(e.latlng);
    } else {
        clickedMarker = L.marker(e.latlng).addTo(map);
    }
    
    showTimeseriesChart(lat, lon);
}

// App Entry Point
window.addEventListener('DOMContentLoaded', () => {
    initMap();
    loadApp();
    startMetadataPolling();

    // Attach map event listeners
    map.on('mousemove', handleMapMouseMove);
    map.on('mouseout', handleMapMouseOut);
    map.on('click', handleMapClick);
    chartCloseBtn.addEventListener('click', closeTimeseriesChart);
});
