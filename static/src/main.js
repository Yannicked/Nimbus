import { CONFIG } from './config.js';
import { state, parseURLState } from './state.js';
import { DOM } from './ui/dom.js';
import { fetchMetadata } from './api.js';
import { initMap, updateRadarOverlay, clearRadarLayers } from './map/index.js';
import { initControls, drawSliderTicks, updateTimeStepDisplay, updateLegend, formatAbsoluteTime, selectEnsemble, selectLayerMode, selectWindHeight, updateTimelineSlider } from './ui/controls.js';
import { showTimeseriesChart, closeTimeseriesChart } from './ui/chart.js';
import { checkRainAndNotify } from './notifications.js';

// Fetch Metadata and Load App
async function loadApp() {
    try {
        // Fetch metadata in parallel
        const [rainMetadata, tempMetadata, windMetadata, solarMetadata] = await Promise.all([
            fetchMetadata('rain'),
            fetchMetadata('temp'),
            fetchMetadata('wind'),
            fetchMetadata('solar')
        ]);
        state.rainMetadata = rainMetadata;
        state.tempMetadata = tempMetadata;
        state.windMetadata = windMetadata;
        state.solarMetadata = solarMetadata;
        
        // Default active metadata
        if (state.currentLayerMode === 'temp') {
            state.metadata = state.tempMetadata;
        } else if (state.currentLayerMode === 'wind') {
            state.metadata = state.windMetadata;
        } else if (state.currentLayerMode === 'solar') {
            state.metadata = state.solarMetadata;
        } else {
            state.metadata = state.rainMetadata;
        }
        
        // Find index closest to current system time
        const refMatch = state.metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
        let refTimeMs = Date.now();
        if (refMatch) {
            refTimeMs = new Date(`${refMatch[1]}T${refMatch[2]}Z`).getTime();
        }
        const targetOffset = (Date.now() - refTimeMs) / 1000;
        let closestIndex = 0;
        let minDiff = Infinity;
        for (let i = 0; i < state.metadata.times.length; i++) {
            const diff = Math.abs(state.metadata.times[i] - targetOffset);
            if (diff < minDiff) {
                minDiff = diff;
                closestIndex = i;
            }
        }
        state.currentTimeIndex = closestIndex;
        
        // Display reference time
        DOM.refTimeVal.textContent = formatAbsoluteTime(state.metadata.reference_time_str, 0);

        // Create Ensemble Selector Options Grouped by Category
        DOM.ensembleSelect.replaceChildren();
        
        // Add statistics first (separate / at the beginning)
        const statsGroup = document.createElement('optgroup');
        statsGroup.label = 'Statistics / Summary';
        
        const stats = ['pmm', 'med', 'max', 'prob', 'spread'];
        const statLabels = { 
            'pmm': 'Probability Matched Mean (PMM)',
            'med': 'Median Forecast (MED)', 
            'max': 'Maximum Forecast (MAX)', 
            'prob': 'Neighborhood Probability (NEP)',
            'spread': 'Forecast Uncertainty (SPREAD)'
        };
        stats.forEach(stat => {
            const opt = document.createElement('option');
            opt.value = stat;
            opt.textContent = statLabels[stat];
            if (stat === state.currentEns) opt.selected = true;
            statsGroup.appendChild(opt);
        });
        DOM.ensembleSelect.appendChild(statsGroup);

        // Add individual ensemble members
        const membersGroup = document.createElement('optgroup');
        membersGroup.label = 'Ensemble Members';
        
        state.rainMetadata.ensembles.forEach(ens => {
            const opt = document.createElement('option');
            opt.value = ens.toString();
            opt.textContent = `Ensemble Member E${ens}`;
            if (ens === state.currentEns) opt.selected = true;
            membersGroup.appendChild(opt);
        });
        DOM.ensembleSelect.appendChild(membersGroup);

        // Initialize Timeline Slider
        DOM.timeSlider.min = 0;
        updateTimelineSlider();

        // Load initial overlay
        updateRadarOverlay();
        updateTimeStepDisplay();
        updateLegend();

        // Trigger background rain alert notification check
        checkRainAndNotify(state.rainMetadata);

    } catch (e) {
        console.error(e);
        DOM.refTimeVal.textContent = "Error loading data!";
    }
}

// Poll for metadata updates to detect new NetCDF file
function startMetadataPolling() {
    setInterval(async () => {
        try {
            const endpoint = state.currentLayerMode === 'temp' 
                ? '/api/metadata/temp' 
                : (state.currentLayerMode === 'wind' ? '/api/metadata/wind' : (state.currentLayerMode === 'solar' ? '/api/metadata/solar' : '/api/metadata'));
            const response = await fetch(endpoint);
            if (!response.ok) return;
            const newMetadata = await response.json();

            let targetMetadata = state.currentLayerMode === 'temp' 
                ? state.tempMetadata 
                : (state.currentLayerMode === 'wind' ? state.windMetadata : (state.currentLayerMode === 'solar' ? state.solarMetadata : state.rainMetadata));
                
            if (targetMetadata && newMetadata.version !== targetMetadata.version) {
                console.log(`New ${state.currentLayerMode} forecast run detected! Reloading...`);
                clearRadarLayers();
                
                if (state.currentLayerMode === 'temp') {
                    state.tempMetadata = newMetadata;
                    state.metadata = state.tempMetadata;
                } else if (state.currentLayerMode === 'wind') {
                    state.windMetadata = newMetadata;
                    state.metadata = state.windMetadata;
                    state.windPixelData = null;
                } else if (state.currentLayerMode === 'solar') {
                    state.solarMetadata = newMetadata;
                    state.metadata = state.solarMetadata;
                } else {
                    state.rainMetadata = newMetadata;
                    state.metadata = state.rainMetadata;
                    
                    // Trigger notification check when rain data updates
                    checkRainAndNotify(newMetadata);
                }

                // Re-render timeline slider
                updateTimelineSlider();
                updateRadarOverlay();

                // Update reference time display
                DOM.refTimeVal.textContent = formatAbsoluteTime(state.metadata.reference_time_str, 0);
            }

            // If we are not on the rain layer, we still need to poll rain metadata separately to trigger notifications
            if (state.currentLayerMode !== 'rain') {
                try {
                    const rainResponse = await fetch('/api/metadata');
                    if (rainResponse.ok) {
                        const newRainMetadata = await rainResponse.json();
                        if (state.rainMetadata && newRainMetadata.version !== state.rainMetadata.version) {
                            state.rainMetadata = newRainMetadata;
                            checkRainAndNotify(newRainMetadata);
                        }
                    }
                } catch (err) {
                    console.error("Failed to check rain metadata for notifications:", err);
                }
            }
        } catch (e) {
            console.error("Failed to check for metadata update:", e);
        }
    }, CONFIG.intervals.metadataPollingMs);
}

// Orchestrator tying it all together
async function bootstrap() {
    parseURLState();
    initMap();
    await loadApp();
    initControls();
    startMetadataPolling();

    // Restore layer mode, wind height, and selected location on load
    if (state.currentLayerMode !== 'rain') {
        const targetMode = state.currentLayerMode;
        state.currentLayerMode = 'rain'; // temporarily reset to trigger setup code
        selectLayerMode(targetMode);
    }
    if (state.currentLayerMode === 'wind') {
        selectWindHeight(state.selectedWindHeight);
    }

    const urlParams = new URLSearchParams(window.location.search);
    const sLat = parseFloat(urlParams.get('slat'));
    const sLon = parseFloat(urlParams.get('slon'));
    if (!isNaN(sLat) && !isNaN(sLon)) {
        if (state.clickedMarker) {
            state.clickedMarker.setLngLat([sLon, sLat]);
        } else {
            state.clickedMarker = new maplibregl.Marker()
                .setLngLat([sLon, sLat])
                .addTo(state.map);
        }
        showTimeseriesChart(sLat, sLon);
    }

    // Additional event listeners
    DOM.chartCloseBtn.addEventListener('click', closeTimeseriesChart);
}

window.addEventListener('DOMContentLoaded', bootstrap);
