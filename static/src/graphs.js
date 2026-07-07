import { fetchMetadata, fetchTimeseries } from './api.js';
import { state } from './state.js';
import { requestNotificationPermission, saveNotificationPrefs, checkRainAndNotify } from './notifications.js';

// Local cache for fetched data to avoid re-fetching when alert threshold changes
const currentData = {
    rainPmm: null,
    rainMed: null,
    rainMax: null,
    temp: null,
    wind: null,
    solar: null,
    rainMetadata: null,
    tempMetadata: null,
    windMetadata: null,
    solarMetadata: null
};

// Store chart instances to destroy them before re-rendering
const chartInstances = {
    rain: null,
    temp: null,
    wind: null,
    solar: null
};

// Formatting helpers
function formatAbsoluteTime(refTimeStr, secondsOffset) {
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

function formatRelativeTime(secondsOffset) {
    const mins = Math.round(secondsOffset / 60);
    if (mins < 60) {
        return `${mins}m`;
    }
    const hrs = Math.floor(mins / 60);
    const remainingMins = mins % 60;
    return remainingMins > 0 ? `${hrs}h ${remainingMins}m` : `${hrs}h`;
}

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

function degreesToCardinal(deg) {
    const index = Math.round(deg / 45) % 8;
    const cardinals = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    return cardinals[index];
}

// Generate labels array for charts
function getLabels(times, refTimeStr, isShort = false) {
    return times.map(secs => {
        const timeStr = formatAbsoluteTime(refTimeStr, secs);
        if (isShort) {
            const match = timeStr.match(/(\d{2}:\d{2})/);
            return match ? match[1] : `+${Math.round(secs/60)}m`;
        } else {
            const match = timeStr.match(/(\d{2})\s+(\w+).*?(\d{2}:\d{2})/);
            if (match) {
                const refMatch = refTimeStr.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
                if (refMatch) {
                    const refDate = new Date(`${refMatch[1]}T${refMatch[2]}Z`);
                    const targetDate = new Date(refDate.getTime() + secs * 1000);
                    const dayName = targetDate.toLocaleDateString('en-GB', { timeZone: 'Europe/Amsterdam', weekday: 'short' });
                    return `${dayName} ${match[3]}`;
                }
                return `${match[1]} ${match[2]} ${match[3]}`;
            }
            const matchTime = timeStr.match(/(\d{2}:\d{2})/);
            return matchTime ? matchTime[1] : `+${Math.round(secs/60)}m`;
        }
    });
}

/**
 * Common configuration builder for Chart.js line charts used in the dashboard.
 * @param {Array} labels - X-axis labels.
 * @param {Array} datasets - Array of dataset objects.
 * @param {string} yAxisTitle - Label for the Y-axis.
 * @param {Object} options - Optional overrides (showLegend, yMin, tooltipCallbacks).
 * @returns {Object} Chart.js configuration object.
 */
function createChartConfig(labels, datasets, yAxisTitle, options = {}) {
    return {
        type: 'line',
        data: {
            labels: labels,
            datasets: datasets
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            plugins: {
                legend: {
                    display: options.showLegend || false,
                    labels: {
                        color: '#94a3b8',
                        font: { size: 9 },
                        boxWidth: 12
                    }
                },
                tooltip: {
                    mode: 'index',
                    intersect: false,
                    backgroundColor: '#1e1e24',
                    titleColor: '#f8fafc',
                    bodyColor: '#f8fafc',
                    borderColor: 'rgba(255,255,255,0.1)',
                    borderWidth: 1,
                    callbacks: options.tooltipCallbacks || {}
                }
            },
            scales: {
                x: {
                    grid: { color: 'rgba(255, 255, 255, 0.05)' },
                    ticks: { color: '#94a3b8', font: { size: 9 }, maxTicksLimit: 6 }
                },
                y: {
                    grid: { color: 'rgba(255, 255, 255, 0.05)' },
                    ticks: { color: '#94a3b8', font: { size: 9 } },
                    title: {
                        display: true,
                        text: yAxisTitle,
                        color: '#94a3b8',
                        font: { size: 9, weight: 'bold' }
                    },
                    min: options.yMin !== undefined ? options.yMin : undefined
                }
            }
        }
    };
}

// Show temporary toast notification
function showToast(message) {
    const toast = document.getElementById('toast');
    toast.textContent = message;
    toast.classList.remove('hidden');
    setTimeout(() => {
        toast.classList.add('hidden');
    }, 3000);
}

// Read alert preferences from local storage and update form
function loadNotificationPrefs() {
    const emailNotify = localStorage.getItem('nimbus_notify_email') === 'true';
    const emailAddr = localStorage.getItem('nimbus_email') || '';
    const pushNotify = localStorage.getItem('nimbus_notify_push') === 'true';
    const threshold = localStorage.getItem('nimbus_rain_threshold') || '1.0';
    
    document.getElementById('notify-email').checked = emailNotify;
    document.getElementById('input-email').value = emailAddr;
    document.getElementById('notify-push').checked = pushNotify;
    document.getElementById('alert-threshold').value = threshold;
    
    const emailGroup = document.getElementById('group-email');
    if (emailNotify) {
        emailGroup.classList.remove('hidden');
        document.getElementById('input-email').required = true;
    } else {
        emailGroup.classList.add('hidden');
        document.getElementById('input-email').required = false;
    }
}

// Analyze the rain forecast timeseries for starting time and duration
function updateRainAlertBox() {
    const statusBox = document.getElementById('rain-status-box');
    const statusTitle = document.getElementById('rain-status-title');
    const statusDesc = document.getElementById('rain-status-desc');
    
    const threshold = parseFloat(document.getElementById('alert-threshold').value);
    const emailEnabled = document.getElementById('notify-email').checked;
    const pushEnabled = document.getElementById('notify-push').checked;
    
    // Clear styling classes
    statusBox.className = 'alert-status-box';
    
    if (!currentData.rainPmm || currentData.rainPmm.status === 'out_of_bounds' || !currentData.rainPmm.values.length) {
        statusBox.classList.add('info');
        statusTitle.textContent = "Out of Bounds";
        statusDesc.textContent = "Coordinates are outside the weather radar coverage area.";
        return;
    }
    
    const times = currentData.rainPmm.times;
    const pmmVals = currentData.rainPmm.values;
    
    let rainStartIndex = -1;
    for (let i = 0; i < pmmVals.length; i++) {
        if (pmmVals[i] >= threshold) {
            rainStartIndex = i;
            break;
        }
    }
    
    if (rainStartIndex !== -1) {
        // Find duration of rain event
        let rainEndIndex = rainStartIndex;
        for (let j = rainStartIndex; j < pmmVals.length; j++) {
            if (pmmVals[j] >= threshold) {
                rainEndIndex = j;
            } else {
                // Allow brief dips of up to 10 minutes (2 indices) before declaring event ended
                let stillRaining = false;
                for (let k = j; k < Math.min(j + 3, pmmVals.length); k++) {
                    if (pmmVals[k] >= threshold) {
                        stillRaining = true;
                        break;
                    }
                }
                if (!stillRaining) {
                    break;
                }
            }
        }
        
        const startSecs = times[rainStartIndex];
        const endSecs = times[rainEndIndex] + 300; // include the full step
        const durationSecs = endSecs - startSecs;
        const durationMins = Math.round(durationSecs / 60);
        
        const peak = Math.max(...pmmVals.slice(rainStartIndex, rainEndIndex + 1));
        const startRel = formatRelativeTime(startSecs);
        const startAbs = formatAbsoluteTime(currentData.rainMetadata.reference_time_str, startSecs);
        
        // Alert styling based on threshold severity
        if (threshold >= 5.0) {
            statusBox.classList.add('danger');
        } else {
            statusBox.classList.add('warning');
        }
        
        statusTitle.textContent = `Rain Alert: In ${startRel}`;
        
        statusDesc.replaceChildren();
        statusDesc.appendChild(document.createTextNode("Rain is forecasted to start at "));
        const startStrong = document.createElement('strong');
        startStrong.textContent = startAbs;
        statusDesc.appendChild(startStrong);
        statusDesc.appendChild(document.createTextNode(" and continue for about "));
        const durationStrong = document.createElement('strong');
        durationStrong.textContent = `${durationMins} minutes`;
        statusDesc.appendChild(durationStrong);
        statusDesc.appendChild(document.createTextNode(". Peak intensity: "));
        const peakStrong = document.createElement('strong');
        peakStrong.textContent = `${peak.toFixed(2)} mm/h`;
        statusDesc.appendChild(peakStrong);
        statusDesc.appendChild(document.createTextNode("."));

        if (emailEnabled || pushEnabled) {
            statusDesc.appendChild(document.createElement('br'));
            const alertSpan = document.createElement('span');
            alertSpan.style.marginTop = '4px';
            alertSpan.style.display = 'inline-block';
            alertSpan.style.fontSize = '0.7rem';
            alertSpan.style.opacity = '0.85';

            const bellIcon = document.createElement('i');
            bellIcon.className = 'fa-solid fa-bell';
            alertSpan.appendChild(bellIcon);

            alertSpan.appendChild(document.createTextNode(` Alerts active (email: ${emailEnabled ? 'yes' : 'no'}, push: ${pushEnabled ? 'yes' : 'no'}).`));
            statusDesc.appendChild(alertSpan);
        }
    } else {
        statusBox.classList.add('success');
        statusTitle.textContent = "No Rain Expected";
        statusDesc.replaceChildren();
        statusDesc.appendChild(document.createTextNode("Forecast indicates precipitation will remain below your threshold of "));
        const thresholdStrong = document.createElement('strong');
        thresholdStrong.textContent = `${threshold.toFixed(2)} mm/h`;
        statusDesc.appendChild(thresholdStrong);
        statusDesc.appendChild(document.createTextNode(" for the next 6 hours."));
    }
}

// Render Rainfall Chart (PMM, Median, Max)
function renderRainChart(tsPmm, tsMed, tsMax, metadata) {
    if (chartInstances.rain) {
        chartInstances.rain.destroy();
    }
    
    if (tsPmm.status === 'out_of_bounds' || !tsPmm.values.length) {
        document.getElementById('stat-rain-peak').textContent = '--';
        document.getElementById('stat-rain-total').textContent = '--';
        return;
    }
    
    const pPeak = Math.max(...tsPmm.values);
    const mPeak = Math.max(...tsMax.values);
    const accumulation = tsPmm.values.reduce((a, b) => a + b, 0) / 12.0; // PMM accumulation
    
    document.getElementById('stat-rain-peak').textContent = `${pPeak.toFixed(2)} / ${mPeak.toFixed(2)} mm/h`;
    document.getElementById('stat-rain-total').textContent = `${accumulation.toFixed(1)} mm`;
    
    const labels = getLabels(tsPmm.times, metadata.reference_time_str, true);
    
    const datasets = [
        {
            label: 'PMM (Mean)',
            data: tsPmm.values,
            borderColor: '#38bdf8',
            backgroundColor: 'rgba(56, 189, 248, 0.05)',
            borderWidth: 2,
            fill: true,
            tension: 0.3,
            pointRadius: 0,
            pointHoverRadius: 4
        },
        {
            label: 'Median',
            data: tsMed.values,
            borderColor: '#a855f7',
            backgroundColor: 'transparent',
            borderWidth: 1.5,
            borderDash: [4, 4],
            fill: false,
            tension: 0.3,
            pointRadius: 0,
            pointHoverRadius: 3
        },
        {
            label: 'Maximum',
            data: tsMax.values,
            borderColor: '#f59e0b',
            backgroundColor: 'transparent',
            borderWidth: 1.5,
            fill: false,
            tension: 0.3,
            pointRadius: 0,
            pointHoverRadius: 3
        }
    ];

    const ctx = document.getElementById('chart-rain').getContext('2d');
    chartInstances.rain = new Chart(ctx, createChartConfig(labels, datasets, 'Rainfall Rate (mm/h)', {
        showLegend: true,
        yMin: 0
    }));
}

// Render Temperature Chart
function renderTempChart(tsTemp, metadata) {
    if (chartInstances.temp) {
        chartInstances.temp.destroy();
    }
    
    if (tsTemp.status === 'out_of_bounds' || !tsTemp.values.length) {
        document.getElementById('stat-temp-max').textContent = '--';
        document.getElementById('stat-temp-min').textContent = '--';
        return;
    }
    
    const maxVal = Math.max(...tsTemp.values);
    const minVal = Math.min(...tsTemp.values);
    
    document.getElementById('stat-temp-max').textContent = `${maxVal.toFixed(1)} °C`;
    document.getElementById('stat-temp-min').textContent = `${minVal.toFixed(1)} °C`;
    
    const labels = getLabels(tsTemp.times, metadata.reference_time_str, false);
    
    const datasets = [{
        label: '2m Temperature',
        data: tsTemp.values,
        borderColor: '#ef4444', // Reddish warm line
        backgroundColor: 'rgba(239, 68, 68, 0.05)',
        borderWidth: 2,
        fill: true,
        tension: 0.3,
        pointRadius: 0,
        pointHoverRadius: 4
    }];

    const ctx = document.getElementById('chart-temp').getContext('2d');
    chartInstances.temp = new Chart(ctx, createChartConfig(labels, datasets, 'Temperature (°C)', {
        tooltipCallbacks: {
            label: function(context) {
                return ` Temp: ${context.parsed.y.toFixed(1)} °C`;
            }
        }
    }));
}

// Render Wind Chart
function renderWindChart(tsWind, metadata) {
    if (chartInstances.wind) {
        chartInstances.wind.destroy();
    }
    
    if (tsWind.status === 'out_of_bounds' || !tsWind.speeds.length) {
        document.getElementById('stat-wind-peak').textContent = '--';
        document.getElementById('stat-wind-avg').textContent = '--';
        return;
    }
    
    const peakVal = Math.max(...tsWind.speeds);
    const avgVal = tsWind.speeds.reduce((a, b) => a + b, 0) / tsWind.speeds.length;
    
    document.getElementById('stat-wind-peak').textContent = `${peakVal.toFixed(1)} m/s`;
    document.getElementById('stat-wind-avg').textContent = `${avgVal.toFixed(1)} m/s`;
    
    const labels = getLabels(tsWind.times, metadata.reference_time_str, false);
    
    const datasets = [{
        label: 'Wind Speed',
        data: tsWind.speeds,
        borderColor: '#10b981', // green line
        backgroundColor: 'rgba(16, 185, 129, 0.05)',
        borderWidth: 2,
        fill: true,
        tension: 0.3,
        pointRadius: 0,
        pointHoverRadius: 4
    }];

    const ctx = document.getElementById('chart-wind').getContext('2d');
    chartInstances.wind = new Chart(ctx, createChartConfig(labels, datasets, 'Wind Speed (m/s)', {
        yMin: 0,
        tooltipCallbacks: {
            label: function(context) {
                const index = context.dataIndex;
                const speed = context.parsed.y;
                const dir = tsWind.directions[index];
                const cardinal = degreesToCardinal(dir);
                return ` Wind: ${speed.toFixed(1)} m/s (${mpsToBeaufort(speed)} Bft) | Dir: ${dir.toFixed(0)}° (${cardinal})`;
            }
        }
    }));
}

// Render Solar Radiation Chart
function renderSolarChart(tsSolar, metadata) {
    if (chartInstances.solar) {
        chartInstances.solar.destroy();
    }
    
    if (tsSolar.status === 'out_of_bounds' || !tsSolar.values.length) {
        document.getElementById('stat-solar-peak').textContent = '--';
        document.getElementById('stat-solar-avg').textContent = '--';
        return;
    }
    
    const peakVal = Math.max(...tsSolar.values);
    const avgVal = tsSolar.values.reduce((a, b) => a + b, 0) / tsSolar.values.length;
    
    document.getElementById('stat-solar-peak').textContent = `${Math.round(peakVal)} W/m²`;
    document.getElementById('stat-solar-avg').textContent = `${Math.round(avgVal)} W/m²`;
    
    const labels = getLabels(tsSolar.times, metadata.reference_time_str, false);
    
    const datasets = [{
        label: 'Solar Radiation',
        data: tsSolar.values,
        borderColor: '#eab308', // Yellow
        backgroundColor: 'rgba(234, 179, 8, 0.05)',
        borderWidth: 2,
        fill: true,
        tension: 0.3,
        pointRadius: 0,
        pointHoverRadius: 4
    }];

    const ctx = document.getElementById('chart-solar').getContext('2d');
    chartInstances.solar = new Chart(ctx, createChartConfig(labels, datasets, 'Solar Radiation (W/m²)', {
        yMin: 0,
        tooltipCallbacks: {
            label: function(context) {
                return ` Solar: ${Math.round(context.parsed.y)} W/m²`;
            }
        }
    }));
}

// Update "Interactive Map" link in header to zoom/center on current coordinates
function updateBackToMapLink(lat, lon) {
    const backBtn = document.getElementById('btn-back-to-map');
    if (backBtn) {
        backBtn.href = `/?lat=${lat.toFixed(4)}&lon=${lon.toFixed(4)}&zoom=11&slat=${lat.toFixed(4)}&slon=${lon.toFixed(4)}`;
    }
}

// Fetch all forecasts and render
async function refreshData(lat, lon) {
    updateBackToMapLink(lat, lon);
    
    // Show loading text in badges
    const loadLabels = ['stat-rain-peak', 'stat-rain-total', 'stat-temp-max', 'stat-temp-min', 'stat-wind-peak', 'stat-wind-avg', 'stat-solar-peak', 'stat-solar-avg'];
    loadLabels.forEach(id => {
        document.getElementById(id).textContent = 'Loading...';
    });
    
    try {
        // Fetch metadatas in parallel
        const [rainMeta, tempMeta, windMeta, solarMeta] = await Promise.all([
            fetchMetadata('rain'),
            fetchMetadata('temp'),
            fetchMetadata('wind'),
            fetchMetadata('solar')
        ]);
        
        currentData.rainMetadata = rainMeta;
        currentData.tempMetadata = tempMeta;
        currentData.windMetadata = windMeta;
        currentData.solarMetadata = solarMeta;
        
        // Update header time
        document.getElementById('ref-time-value').textContent = formatAbsoluteTime(rainMeta.reference_time_str, 0);
        
        // Fetch all timeseries in parallel
        const [tsRainPmm, tsRainMed, tsRainMax, tsTemp, tsWind, tsSolar] = await Promise.all([
            fetchTimeseries('rain', 'pmm', lat, lon),
            fetchTimeseries('rain', 'med', lat, lon),
            fetchTimeseries('rain', 'max', lat, lon),
            fetchTimeseries('temp', 'med', lat, lon),
            fetchTimeseries('wind', 'med', lat, lon),
            fetchTimeseries('solar', 'med', lat, lon)
        ]);
        
        currentData.rainPmm = tsRainPmm;
        currentData.rainMed = tsRainMed;
        currentData.rainMax = tsRainMax;
        currentData.temp = tsTemp;
        currentData.wind = tsWind;
        currentData.solar = tsSolar;
        
        // Draw all charts
        renderRainChart(tsRainPmm, tsRainMed, tsRainMax, rainMeta);
        renderTempChart(tsTemp, tempMeta);
        renderWindChart(tsWind, windMeta);
        renderSolarChart(tsSolar, solarMeta);
        
        // Update Calculated Alert
        updateRainAlertBox();
        
        // Trigger background rain alert notification check
        checkRainAndNotify(currentData.rainMetadata);
        
    } catch (e) {
        console.error("Failed to load forecast data:", e);
        showToast("Error loading weather forecast!");
        loadLabels.forEach(id => {
            document.getElementById(id).textContent = 'Error';
        });
    }
}

// Set up event listeners
function setupListeners() {
    // Coords update form
    document.getElementById('coords-form').addEventListener('submit', (e) => {
        e.preventDefault();
        const lat = parseFloat(document.getElementById('input-lat').value);
        const lon = parseFloat(document.getElementById('input-lon').value);
        
        if (isNaN(lat) || lat < 49.0 || lat > 56.0 || isNaN(lon) || lon < 2.0 || lon > 8.0) {
            showToast("Coordinates are outside valid coverage (Netherlands bounds: Lat 49-56, Lon 2-8)");
            return;
        }
        
        document.getElementById('display-lat').textContent = lat.toFixed(4);
        document.getElementById('display-lon').textContent = lon.toFixed(4);
        
        // Sync URL query params without full page reload
        const url = new URL(window.location.href);
        url.searchParams.set('lat', lat.toFixed(4));
        url.searchParams.set('lon', lon.toFixed(4));
        window.history.pushState({}, '', url.pathname + url.search);
        
        refreshData(lat, lon);
    });
    
    // GPS Geolocation button
    document.getElementById('btn-geolocation').addEventListener('click', () => {
        if (!navigator.geolocation) {
            showToast("Geolocation is not supported by your browser");
            return;
        }
        
        showToast("Retrieving GPS location...");
        navigator.geolocation.getCurrentPosition(
            (pos) => {
                const lat = pos.coords.latitude;
                const lon = pos.coords.longitude;
                
                document.getElementById('input-lat').value = lat.toFixed(4);
                document.getElementById('input-lon').value = lon.toFixed(4);
                document.getElementById('display-lat').textContent = lat.toFixed(4);
                document.getElementById('display-lon').textContent = lon.toFixed(4);
                
                // Sync URL query params
                const url = new URL(window.location.href);
                url.searchParams.set('lat', lat.toFixed(4));
                url.searchParams.set('lon', lon.toFixed(4));
                window.history.pushState({}, '', url.pathname + url.search);
                
                refreshData(lat, lon);
                showToast("Location updated successfully!");
            },
            (err) => {
                console.error("GPS Error:", err);
                showToast(`Unable to retrieve location: ${err.message}`);
            }
        );
    });
    
    // Alert configuration form toggle
    document.getElementById('notify-email').addEventListener('change', (e) => {
        const emailGroup = document.getElementById('group-email');
        if (e.target.checked) {
            emailGroup.classList.remove('hidden');
            document.getElementById('input-email').required = true;
        } else {
            emailGroup.classList.add('hidden');
            document.getElementById('input-email').required = false;
        }
    });
    
    // Alert form submit
    document.getElementById('notification-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        const emailNotify = document.getElementById('notify-email').checked;
        const emailAddr = document.getElementById('input-email').value;
        let pushNotify = document.getElementById('notify-push').checked;
        const threshold = parseFloat(document.getElementById('alert-threshold').value);
        
        const params = new URLSearchParams(window.location.search);
        let lat = parseFloat(params.get('lat')) || 52.1;
        let lon = parseFloat(params.get('lon')) || 5.2;
        
        if (pushNotify) {
            const granted = await requestNotificationPermission();
            if (!granted) {
                showToast("Notification permission denied by browser. Please enable notifications in your settings.");
                pushNotify = false;
                document.getElementById('notify-push').checked = false;
            }
        }
        
        saveNotificationPrefs(lat.toFixed(4), lon.toFixed(4), threshold, emailNotify, emailAddr, pushNotify);
        
        showToast("Alert preferences saved successfully!");
        
        // Re-analyze forecast using new parameters
        updateRainAlertBox();
        
        // Run check immediately if push notifications are active
        if (pushNotify && currentData.rainMetadata) {
            checkRainAndNotify(currentData.rainMetadata);
        }
    });
    
    // Wind height selector
    document.getElementById('wind-height-select').addEventListener('change', async (e) => {
        const height = parseInt(e.target.value);
        state.selectedWindHeight = height;
        
        const params = new URLSearchParams(window.location.search);
        let lat = parseFloat(params.get('lat')) || 52.1;
        let lon = parseFloat(params.get('lon')) || 5.2;
        
        document.getElementById('stat-wind-peak').textContent = 'Loading...';
        document.getElementById('stat-wind-avg').textContent = 'Loading...';
        
        try {
            const tsWind = await fetchTimeseries('wind', 'med', lat, lon);
            currentData.wind = tsWind;
            renderWindChart(tsWind, currentData.windMetadata);
        } catch (err) {
            console.error("Failed to load new wind height timeseries:", err);
            showToast("Error updating wind forecast height!");
        }
    });
    
    // Listen for back/forward browser navigation
    window.addEventListener('popstate', () => {
        const params = new URLSearchParams(window.location.search);
        let lat = parseFloat(params.get('lat')) || 52.1;
        let lon = parseFloat(params.get('lon')) || 5.2;
        
        document.getElementById('input-lat').value = lat.toFixed(4);
        document.getElementById('input-lon').value = lon.toFixed(4);
        document.getElementById('display-lat').textContent = lat.toFixed(4);
        document.getElementById('display-lon').textContent = lon.toFixed(4);
        
        refreshData(lat, lon);
    });
}

// Poll for rain metadata updates to detect new NetCDF file
function startMetadataPolling() {
    setInterval(async () => {
        try {
            const response = await fetch('/api/metadata');
            if (!response.ok) return;
            const newMetadata = await response.json();
            
            if (currentData.rainMetadata && newMetadata.version !== currentData.rainMetadata.version) {
                console.log("New rain forecast run detected in graphs! Reloading dashboard...");
                
                const params = new URLSearchParams(window.location.search);
                let lat = parseFloat(params.get('lat')) || 52.1;
                let lon = parseFloat(params.get('lon')) || 5.2;
                
                // Reload dashboard data
                await refreshData(lat, lon);
                
                // Trigger background rain alert notification check
                checkRainAndNotify(newMetadata);
            }
        } catch (e) {
            console.error("Failed to check for rain metadata update:", e);
        }
    }, 5000); // 5 seconds
}

// Bootstrap standalone dashboard
function bootstrap() {
    const params = new URLSearchParams(window.location.search);
    let lat = parseFloat(params.get('lat'));
    let lon = parseFloat(params.get('lon'));
    
    if (isNaN(lat) || lat < 49.0 || lat > 56.0) lat = 52.1;
    if (isNaN(lon) || lon < 2.0 || lon > 8.0) lon = 5.2;
    
    document.getElementById('input-lat').value = lat.toFixed(4);
    document.getElementById('input-lon').value = lon.toFixed(4);
    document.getElementById('display-lat').textContent = lat.toFixed(4);
    document.getElementById('display-lon').textContent = lon.toFixed(4);
    
    loadNotificationPrefs();
    setupListeners();
    refreshData(lat, lon);
    startMetadataPolling();
}

document.addEventListener('DOMContentLoaded', bootstrap);
