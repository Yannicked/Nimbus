import { CONFIG } from '../config.js';
import { state } from '../state.js';
import { DOM } from './dom.js';
import { updateRadarOverlay, triggerHoverQuery, clearRadarLayers, setupRadarSourceAndLayer } from '../map/index.js';
import { showTimeseriesChart } from './chart.js';

// Format relative time step
export function formatRelativeTime(seconds) {
    const mins = Math.round(seconds / 60);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    if (h > 0) {
        return `+${h}h ${m.toString().padStart(2, '0')}m`;
    }
    return `+${m}`;
}

// Format absolute forecast time
export function formatAbsoluteTime(refTimeStr, secondsOffset) {
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

// Render ticks on the timeline slider
export function drawSliderTicks() {
    if (!state.metadata) return;
    DOM.sliderTicks.innerHTML = '';
    
    const stepCount = state.metadata.times.length;
    for (let i = 0; i < stepCount; i++) {
        const span = document.createElement('span');
        const secs = state.metadata.times[i];
        
        // Mark every hour as a larger tick
        if (secs % 3600 === 0) {
            span.classList.add('hour-tick');
        }
        DOM.sliderTicks.appendChild(span);
    }
}

// Update time text displays
export function updateTimeStepDisplay() {
    if (!state.metadata) return;
    const timeVal = state.metadata.times[state.currentTimeIndex];
    DOM.currentTimeStep.textContent = formatAbsoluteTime(state.metadata.reference_time_str, timeVal);
    DOM.timeStepRelative.textContent = formatRelativeTime(timeVal);
}

// Update the legend colors and labels dynamically
export function updateLegend() {
    if (!DOM.legendTitle || !DOM.legendBar || !DOM.legendLabels) return;

    const visConfig = (state.currentEns === 'prob') ? CONFIG.radarVisualization.prob : CONFIG.radarVisualization.rate;
    
    DOM.legendTitle.textContent = visConfig.title;
    
    DOM.legendBar.innerHTML = visConfig.colors
        .map(color => `<span style="background: ${color};"></span>`)
        .join('');
        
    DOM.legendLabels.innerHTML = visConfig.labels
        .map(label => `<span>${label}</span>`)
        .join('');
}

// Start playback animation
export function startPlayer() {
    state.isPlaying = true;
    DOM.btnPlay.innerHTML = '<i class="fa-solid fa-pause"></i>';
    DOM.btnPlay.classList.add('btn-active');
    
    const fps = parseInt(DOM.speedSlider.value);
    const intervalMs = 1000 / fps;
    state.playInterval = setInterval(stepForward, intervalMs);
}

// Stop playback animation
export function stopPlayer() {
    if (!state.isPlaying) return;
    state.isPlaying = false;
    DOM.btnPlay.innerHTML = '<i class="fa-solid fa-play"></i>';
    DOM.btnPlay.classList.remove('btn-active');
    clearInterval(state.playInterval);
}

// Advance one step forward in timeline
export function stepForward() {
    if (!state.metadata) return;
    if (state.currentTimeIndex < state.metadata.times.length - 1) {
        state.currentTimeIndex++;
    } else {
        state.currentTimeIndex = 0; // loop
    }
    DOM.timeSlider.value = state.currentTimeIndex;
    updateRadarOverlay();
    updateTimeStepDisplay();
    triggerHoverQuery();
}

// Select ensemble member or stat mode
export function selectEnsemble(ens) {
    state.currentEns = ens;
    clearRadarLayers();
    
    // Update dropdown value if it differs
    if (DOM.ensembleSelect.value !== ens.toString()) {
        DOM.ensembleSelect.value = ens.toString();
    }

    // Update quick selector buttons active state and sliding indicator
    const viewMap = { 'med': '0', 'max': '1', 'prob': '2' };
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === ens.toString());
    });
    const selector = DOM.viewSelector;
    if (selector && viewMap[ens] !== undefined) {
        selector.dataset.active = viewMap[ens];
    }
    
    updateRadarOverlay();
    updateLegend();
    triggerHoverQuery(); // update hover panel if mouse is over map
    
    // If timeseries chart is open, reload it for the new ensemble selection
    if (state.activeCoords) {
        showTimeseriesChart(state.activeCoords.lat, state.activeCoords.lon);
    }
}

// Toggle layer mode between rain, temperature, and wind
export function selectLayerMode(mode) {
    if (mode === state.currentLayerMode) return;
    state.currentLayerMode = mode;
    
    // Update button active state
    document.querySelectorAll('.layer-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.mode === mode);
    });
    
    const selector = DOM.layerSelector;
    if (selector) {
        selector.dataset.active = mode === 'rain' ? '0' : (mode === 'temp' ? '1' : '2');
    }
    
    // Toggle UI visibility depending on layer mode
    const viewSelector = DOM.rainViewSelector;
    const ensembleContainer = DOM.ensembleContainer;
    const legendRain = DOM.legendRain;
    const legendTemp = DOM.legendTemp;
    const legendWind = DOM.legendWind;
    
    if (mode === 'temp') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        if (legendTemp) legendTemp.classList.remove('hidden');
        
        state.metadata = state.tempMetadata;
    } else if (mode === 'wind') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.remove('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.remove('hidden');
        
        state.metadata = state.windMetadata;
    } else {
        if (viewSelector) viewSelector.classList.remove('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.remove('hidden');
        if (legendRain) legendRain.classList.remove('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        
        state.metadata = state.rainMetadata;
    }
    
    // Re-initialize slider and select index closest to current time
    if (state.metadata) {
        DOM.timeSlider.max = state.metadata.times.length - 1;
        
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
        
        DOM.timeSlider.value = state.currentTimeIndex;
        drawSliderTicks();
        updateTimeStepDisplay();
    }
    
    clearRadarLayers();
    setupRadarSourceAndLayer();
    updateRadarOverlay();
    
    // Update hover panel label
    const hoverLabel = DOM.hoverLabel;
    if (hoverLabel) {
        hoverLabel.textContent = mode === 'temp' ? 'TEMPERATURE' : (mode === 'wind' ? `${state.selectedWindHeight}M WIND` : 'PRECIPITATION');
    }

    // Update legend title
    const legendWindTitle = document.querySelector('#legend-wind .section-label');
    if (legendWindTitle) {
        legendWindTitle.textContent = `${state.selectedWindHeight}m Wind Speed (m/s / Bft)`;
    }
    
    // Update hover panel & trend chart if open
    triggerHoverQuery();
    if (state.activeCoords) {
        showTimeseriesChart(state.activeCoords.lat, state.activeCoords.lon);
    }
}

// Switch between map vector style layers
export function switchMapStyle(styleKey) {
    if (state.map && CONFIG.map.styles[styleKey]) {
        state.map.setStyle(CONFIG.map.styles[styleKey]);
    }
}

// Attach all dashboard interaction event listeners
export function initControls() {
    // Sync default UI control values from CONFIG
    DOM.speedSlider.value = CONFIG.defaults.speed;
    DOM.speedValue.textContent = `${CONFIG.defaults.speed} fps`;
    DOM.opacitySlider.value = CONFIG.defaults.opacity;
    DOM.opacityValue.textContent = `${CONFIG.defaults.opacity}%`;

    // Restore settings drawer state
    const savedExpanded = localStorage.getItem('nimbus_settings_expanded');
    const isMobile = window.innerWidth <= 768;
    if (savedExpanded === 'true' && !isMobile) {
        DOM.settingsContent.classList.add('expanded');
        DOM.btnSettingsToggle.classList.add('active');
    } else {
        DOM.settingsContent.classList.remove('expanded');
        DOM.btnSettingsToggle.classList.remove('active');
    }

    // Set initial quick view selector state
    const viewMap = { 'med': '0', 'max': '1', 'prob': '2' };
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === state.currentEns.toString());
    });
    const viewSelector = DOM.viewSelector;
    if (viewSelector && viewMap[state.currentEns] !== undefined) {
        viewSelector.dataset.active = viewMap[state.currentEns];
    }

    // Attach local controls listeners
    DOM.timeSlider.addEventListener('input', (e) => {
        state.currentTimeIndex = parseInt(e.target.value);
        updateRadarOverlay();
        updateTimeStepDisplay();
        triggerHoverQuery();
    });

    DOM.opacitySlider.addEventListener('input', (e) => {
        const val = e.target.value;
        DOM.opacityValue.textContent = `${val}%`;
        if (state.map) {
            state.map.triggerRepaint();
        }
    });

    DOM.speedSlider.addEventListener('input', (e) => {
        const val = e.target.value;
        DOM.speedValue.textContent = `${val} fps`;
        if (state.isPlaying) {
            stopPlayer();
            startPlayer();
        }
    });

    DOM.btnPrev.addEventListener('click', () => {
        stopPlayer();
        if (state.currentTimeIndex > 0) {
            state.currentTimeIndex--;
        } else if (state.metadata) {
            state.currentTimeIndex = state.metadata.times.length - 1; // loop
        }
        DOM.timeSlider.value = state.currentTimeIndex;
        updateRadarOverlay();
        updateTimeStepDisplay();
        triggerHoverQuery();
    });

    DOM.btnNext.addEventListener('click', () => {
        stopPlayer();
        stepForward();
    });

    DOM.btnPlay.addEventListener('click', () => {
        if (state.isPlaying) {
            stopPlayer();
        } else {
            startPlayer();
        }
    });

    DOM.themeSelect.addEventListener('change', (e) => {
        switchMapStyle(e.target.value);
    });

    DOM.btnSettingsToggle.addEventListener('click', () => {
        const isExpanded = DOM.settingsContent.classList.toggle('expanded');
        DOM.btnSettingsToggle.classList.toggle('active', isExpanded);
        localStorage.setItem('nimbus_settings_expanded', isExpanded);
    });

    const toggleInfoModal = (show) => {
        if (show === undefined) {
            const isHidden = DOM.infoModal.classList.toggle('hidden');
            DOM.infoModalBackdrop.classList.toggle('hidden', isHidden);
            DOM.btnInfoToggle.classList.toggle('active', !isHidden);
        } else {
            DOM.infoModal.classList.toggle('hidden', !show);
            DOM.infoModalBackdrop.classList.toggle('hidden', !show);
            DOM.btnInfoToggle.classList.toggle('active', show);
        }
    };

    DOM.btnInfoToggle.addEventListener('click', () => {
        toggleInfoModal();
    });

    DOM.infoCloseBtn.addEventListener('click', () => {
        toggleInfoModal(false);
    });

    DOM.infoModalBackdrop.addEventListener('click', () => {
        toggleInfoModal(false);
    });

    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            toggleInfoModal(false);
        }
    });

    // Attach height buttons event listeners
    document.querySelectorAll('.height-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            selectWindHeight(btn.dataset.height);
        });
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

    // Attach ensemble dropdown selector change listener
    DOM.ensembleSelect.addEventListener('change', (e) => {
        let val = e.target.value;
        if (!isNaN(val)) {
            val = parseInt(val);
        }
        selectEnsemble(val);
    });
}

// Select wind height
export function selectWindHeight(height) {
    state.selectedWindHeight = parseInt(height);
    clearRadarLayers();
    
    const heightMap = { '10': '0', '50': '1', '100': '2', '200': '3', '300': '4' };
    
    document.querySelectorAll('.height-btn').forEach(btn => {
        btn.classList.toggle('active', parseInt(btn.dataset.height) === state.selectedWindHeight);
    });
    
    const selector = DOM.windHeightSelector;
    if (selector && heightMap[height] !== undefined) {
        selector.dataset.active = heightMap[height];
    }
    
    // Update hover panel label
    const hoverLabel = DOM.hoverLabel;
    if (hoverLabel) {
        hoverLabel.textContent = `${state.selectedWindHeight}M WIND`;
    }

    // Update legend title
    const legendWindTitle = document.querySelector('#legend-wind .section-label');
    if (legendWindTitle) {
        legendWindTitle.textContent = `${state.selectedWindHeight}m Wind Speed (m/s / Bft)`;
    }
    
    updateRadarOverlay();
    triggerHoverQuery();
    
    if (state.activeCoords) {
        showTimeseriesChart(state.activeCoords.lat, state.activeCoords.lon);
    }
}
