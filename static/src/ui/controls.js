import { CONFIG } from '../config.js';
import { state, syncStateToURL } from '../state.js';
import { DOM } from './dom.js';
import { updateRadarOverlay, triggerHoverQuery, clearRadarLayers, setupRadarSourceAndLayer, setupRadarSourceAndLayerRight, initMapRight, enableMapSync, disableMapSync } from '../map/index.js';
import { showTimeseriesChart } from './chart.js';

// Format relative time step
export function formatRelativeTime(seconds) {
    if (seconds === 0) return 'Now';
    const isPast = seconds < 0;
    const absSecs = Math.abs(seconds);
    const mins = Math.round(absSecs / 60);
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    const prefix = isPast ? '-' : '+';
    if (h > 0) {
        return `${prefix}${h}h ${m.toString().padStart(2, '0')}m`;
    }
    return `${prefix}${m}m`;
}

// Format absolute forecast time
export function formatAbsoluteTime(refTimeStr, secondsOffset) {
    const match = refTimeStr.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
    if (!match) return `+${Math.round(secondsOffset / 60)}m`;
    
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
    DOM.sliderTicks.replaceChildren();
    
    const stepCount = parseInt(DOM.timeSlider.max) + 1;
    for (let i = 0; i < stepCount; i++) {
        const span = document.createElement('span');
        const secs = state.metadata.times[i];
        
        // Mark every hour as a larger tick
        if (secs !== undefined && secs % 3600 === 0) {
            span.classList.add('hour-tick');
        }
        DOM.sliderTicks.appendChild(span);
    }
}

// Dynamically adjust the timeline slider max and clamp current index based on the selected layer/ensemble
export function updateTimelineSlider() {
    if (!state.metadata) return;

    let maxIndex = state.metadata.times.length - 1;
    if (state.currentLayerMode === 'rain') {
        if (state.currentEns !== 'pmm') {
            maxIndex = (state.metadata.radar_times_len || state.metadata.times.length) - 1;
        }
    }
    
    DOM.timeSlider.max = maxIndex;
    
    // Clamp currentTimeIndex if it exceeds the new max
    if (state.currentTimeIndex > maxIndex) {
        state.currentTimeIndex = maxIndex;
    }
    DOM.timeSlider.value = state.currentTimeIndex;
    
    drawSliderTicks();
    updateTimeStepDisplay();
}

// Update time text displays
export function updateTimeStepDisplay() {
    if (!state.metadata) return;
    const index = Math.round(state.currentTimeIndex);
    const timeVal = state.metadata.times[index];
    if (timeVal === undefined) return;
    
    DOM.currentTimeStep.textContent = formatAbsoluteTime(state.metadata.reference_time_str, timeVal);
    DOM.timeStepRelative.textContent = formatRelativeTime(timeVal);
}

// Update the legend colors and labels dynamically
export function updateLegend() {
    if (!DOM.legendTitle || !DOM.legendBar || !DOM.legendLabels) return;

    let visConfig;
    if (state.currentEns === 'prob') {
        visConfig = CONFIG.radarVisualization.prob;
    } else if (state.currentEns === 'spread') {
        visConfig = CONFIG.radarVisualization.spread;
    } else {
        visConfig = CONFIG.radarVisualization.rate;
    }
    
    DOM.legendTitle.textContent = visConfig.title;
    
    DOM.legendBar.replaceChildren(...visConfig.colors.map(color => {
        const span = document.createElement('span');
        span.style.background = color;
        return span;
    }));
        
    DOM.legendLabels.replaceChildren(...visConfig.labels.map(label => {
        const span = document.createElement('span');
        span.textContent = label;
        return span;
    }));
}

// Start playback animation
export function startPlayer() {
    if (state.isPlaying) return;
    state.isPlaying = true;
    const icon = document.createElement('i');
    icon.className = 'fa-solid fa-pause';
    DOM.btnPlay.replaceChildren(icon);
    DOM.btnPlay.classList.add('btn-active');
    
    const fps = parseFloat(DOM.speedSlider.value) || 2;
    state.playInterval = setInterval(() => {
        stepForward();
    }, 1000 / fps);
}

// Stop playback animation
export function stopPlayer() {
    if (!state.isPlaying) return;
    state.isPlaying = false;
    const icon = document.createElement('i');
    icon.className = 'fa-solid fa-play';
    DOM.btnPlay.replaceChildren(icon);
    DOM.btnPlay.classList.remove('btn-active');
    
    if (state.playInterval) {
        clearInterval(state.playInterval);
        state.playInterval = null;
    }
    triggerHoverQuery();
}

// Advance one step forward in timeline
export function stepForward() {
    if (!state.metadata) return;
    const layerMode = state.currentLayerMode;
    const ens = state.currentEns;
    const maxIndex = (layerMode === 'rain' && ens !== 'pmm') 
        ? (state.metadata.radar_times_len || state.metadata.times.length) - 1 
        : state.metadata.times.length - 1;
        
    if (state.currentTimeIndex < maxIndex) {
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
    const viewMap = { 'pmm': '0', 'med': '1', 'max': '2', 'prob': '3', 'spread': '4' };
    document.querySelectorAll('.view-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === ens.toString());
    });
    const selector = DOM.viewSelector;
    if (selector && viewMap[ens] !== undefined) {
        selector.dataset.active = viewMap[ens];
    }
    
    updateTimelineSlider();
    updateRadarOverlay();
    updateLegend();
    triggerHoverQuery(); // update hover panel if mouse is over map
    
    // If timeseries chart is open, reload it for the new ensemble selection
    if (state.activeCoords) {
        showTimeseriesChart(state.activeCoords.lat, state.activeCoords.lon);
    }
    syncStateToURL();
}

// Toggle layer mode between rain, temperature, solar, and wind
export function selectLayerMode(mode) {
    if (mode === state.currentLayerMode) return;
    state.currentLayerMode = mode;
    
    // Update button active state
    document.querySelectorAll('.layer-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.mode === mode);
    });
    
    const selector = DOM.layerSelector;
    if (selector) {
        selector.dataset.active = mode === 'rain' ? '0' : (mode === 'temp' ? '1' : (mode === 'solar' ? '2' : '3'));
    }
    
    // Toggle UI visibility depending on layer mode
    const viewSelector = DOM.rainViewSelector;
    const ensembleContainer = DOM.ensembleContainer;
    const legendRain = DOM.legendRain;
    const legendTemp = DOM.legendTemp;
    const legendWind = DOM.legendWind;
    const legendSolar = DOM.legendSolar;
    
    if (mode === 'temp') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        if (legendSolar) legendSolar.classList.add('hidden');
        if (legendTemp) legendTemp.classList.remove('hidden');
        
        state.metadata = state.tempMetadata;
    } else if (mode === 'wind') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.remove('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendSolar) legendSolar.classList.add('hidden');
        if (legendWind) legendWind.classList.remove('hidden');
        
        state.metadata = state.windMetadata;
    } else if (mode === 'solar') {
        if (viewSelector) viewSelector.classList.add('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.add('hidden');
        if (legendRain) legendRain.classList.add('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        if (legendSolar) legendSolar.classList.remove('hidden');
        
        state.metadata = state.solarMetadata;
    } else {
        if (viewSelector) viewSelector.classList.remove('hidden');
        if (DOM.windHeightSelector) DOM.windHeightSelector.classList.add('hidden');
        if (ensembleContainer) ensembleContainer.classList.remove('hidden');
        if (legendRain) legendRain.classList.remove('hidden');
        if (legendTemp) legendTemp.classList.add('hidden');
        if (legendWind) legendWind.classList.add('hidden');
        if (legendSolar) legendSolar.classList.add('hidden');
        
        state.metadata = state.rainMetadata;
    }
    
    // Re-initialize slider and select index closest to current time
    if (state.metadata) {
        let maxIndex = state.metadata.times.length - 1;
        if (state.currentLayerMode === 'rain') {
            if (state.currentEns !== 'pmm') {
                maxIndex = (state.metadata.radar_times_len || state.metadata.times.length) - 1;
            }
        }
        
        const refMatch = state.metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
        let refTimeMs = Date.now();
        if (refMatch) {
            refTimeMs = new Date(`${refMatch[1]}T${refMatch[2]}Z`).getTime();
        }
        const targetOffset = (Date.now() - refTimeMs) / 1000;
        let closestIndex = 0;
        let minDiff = Infinity;
        for (let i = 0; i <= maxIndex; i++) {
            const diff = Math.abs(state.metadata.times[i] - targetOffset);
            if (diff < minDiff) {
                minDiff = diff;
                closestIndex = i;
            }
        }
        state.currentTimeIndex = closestIndex;
        
        updateTimelineSlider();
    }
    
    clearRadarLayers();
    setupRadarSourceAndLayer();
    updateRadarOverlay();
    
    // Update hover panel label
    const hoverLabel = DOM.hoverLabel;
    if (hoverLabel) {
        hoverLabel.textContent = mode === 'temp' ? 'TEMPERATURE' : (mode === 'solar' ? 'SOLAR RADIATION' : (mode === 'wind' ? `${state.selectedWindHeight}M WIND` : 'PRECIPITATION'));
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
    syncStateToURL();
}

// Switch between map vector style layers
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

// Helper to update compare map clipping inset path
export function updateClipPath() {
    const parent = DOM.swipeDivider.parentElement;
    if (!parent) return;
    const rect = parent.getBoundingClientRect();
    const dividerX = (state.dividerPosition / 100) * rect.width;
    
    DOM.swipeDivider.style.left = `${dividerX}px`;
    
    const mapRightEl = document.getElementById('map-right');
    if (mapRightEl) {
        mapRightEl.style.clipPath = `inset(0px 0px 0px ${dividerX}px)`;
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
    const viewMap = { 'pmm': '0', 'med': '1', 'max': '2', 'prob': '3', 'spread': '4' };
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
        if (state.mapRight) {
            state.mapRight.triggerRepaint();
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
        const layerMode = state.currentLayerMode;
        const ens = state.currentEns;
        const maxIndex = (layerMode === 'rain' && ens !== 'pmm') 
            ? (state.metadata.radar_times_len || state.metadata.times.length) - 1 
            : state.metadata.times.length - 1;
            
        if (state.currentTimeIndex > 0) {
            state.currentTimeIndex--;
        } else if (state.metadata) {
            state.currentTimeIndex = maxIndex;
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

    // -------------------------------------------------------------
    // Compare Mode (Split Screen) Toggling & Dragging
    // -------------------------------------------------------------
    if (DOM.btnCompareToggle) {
        DOM.btnCompareToggle.addEventListener('click', () => {
            state.isCompareModeActive = !state.isCompareModeActive;
            DOM.btnCompareToggle.classList.toggle('active', state.isCompareModeActive);
            
            const mapRightEl = document.getElementById('map-right');
            
            if (state.isCompareModeActive) {
                mapRightEl.classList.remove('hidden');
                DOM.compareMenu.classList.remove('hidden');
                DOM.swipeDivider.classList.remove('hidden');
                
                // Initialize right map
                if (!state.mapRight) {
                    initMapRight();
                } else {
                    // Sync viewport to main map
                    state.mapRight.jumpTo({
                        center: state.map.getCenter(),
                        zoom: state.map.getZoom(),
                        bearing: state.map.getBearing(),
                        pitch: state.map.getPitch()
                    });
                }
                
                enableMapSync();
                updateClipPath();
                setupRadarSourceAndLayerRight();
                updateRadarOverlay();
            } else {
                disableMapSync();
                mapRightEl.classList.add('hidden');
                DOM.compareMenu.classList.add('hidden');
                DOM.swipeDivider.classList.add('hidden');
                
                // Redraw main map
                updateRadarOverlay();
            }
        });
    }

    // Drag divider logic
    if (DOM.swipeDivider) {
        let isDragging = false;
        
        const onStart = (e) => {
            isDragging = true;
            document.body.style.userSelect = 'none';
            document.body.style.cursor = 'ew-resize';
        };
        
        const onMove = (e) => {
            if (!isDragging) return;
            
            const parent = DOM.swipeDivider.parentElement;
            if (!parent) return;
            
            const rect = parent.getBoundingClientRect();
            const clientX = e.touches ? e.touches[0].clientX : e.clientX;
            const relativeX = clientX - rect.left;
            
            let percentage = (relativeX / rect.width) * 100;
            percentage = Math.max(0, Math.min(100, percentage));
            
            state.dividerPosition = percentage;
            updateClipPath();
            
            if (state.map) state.map.triggerRepaint();
            if (state.mapRight) state.mapRight.triggerRepaint();
        };
        
        const onEnd = () => {
            if (isDragging) {
                isDragging = false;
                document.body.style.userSelect = '';
                document.body.style.cursor = '';
            }
        };
        
        DOM.swipeDivider.addEventListener('mousedown', onStart);
        DOM.swipeDivider.addEventListener('touchstart', onStart, { passive: true });
        
        window.addEventListener('mousemove', onMove);
        window.addEventListener('touchmove', onMove, { passive: true });
        
        window.addEventListener('mouseup', onEnd);
        window.addEventListener('touchend', onEnd);
        
        window.addEventListener('resize', () => {
            if (state.isCompareModeActive) {
                updateClipPath();
            }
        });
    }

    // Bind Compare Menu select change listener
    if (DOM.compareLayerSelect) {
        DOM.compareLayerSelect.addEventListener('change', (e) => {
            const val = e.target.value;
            // Options: rain-med, rain-max, rain-prob, rain-spread, temp, solar, wind-10, wind-50, etc.
            if (val.startsWith('rain-')) {
                state.compareLayerMode = 'rain';
                state.compareEns = val.substring(5); // med, max, prob, spread
            } else if (val === 'temp') {
                state.compareLayerMode = 'temp';
                state.compareEns = 'med';
            } else if (val === 'solar') {
                state.compareLayerMode = 'solar';
                state.compareEns = 'med';
            } else if (val.startsWith('wind-')) {
                state.compareLayerMode = 'wind';
                state.compareEns = 'med';
                state.compareSelectedWindHeight = parseInt(val.substring(5));
            }
            
            clearRadarLayers();
            setupRadarSourceAndLayerRight();
            updateRadarOverlay();
            triggerHoverQuery();
        });
    }
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
    syncStateToURL();
}
