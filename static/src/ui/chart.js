import { CONFIG } from '../config.js';
import { state, syncStateToURL } from '../state.js';
import { DOM } from './dom.js';
import { fetchTimeseries, registerAbortController, unregisterAbortController, showErrorBanner } from '../api.js';
import { formatAbsoluteTime } from './controls.js';

// Convert wind speed to Beaufort scale
export function mpsToBeaufort(mps) {
    if (typeof mps !== 'number' || isNaN(mps) || mps < 0.3) return 0;
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

// Convert wind direction in degrees to cardinal direction
export function degreesToCardinal(deg) {
    if (typeof deg !== 'number' || isNaN(deg)) return "--";
    const index = Math.round(deg / 45) % 8;
    const cardinals = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    return cardinals[index] || "--";
}

function setChartHeaderTitle(iconClass, text) {
    if (!DOM.chartHeaderTitle) return;
    const icon = document.createElement('i');
    icon.className = `fa-solid ${iconClass} chart-header-icon`;
    DOM.chartHeaderTitle.replaceChildren(icon, document.createTextNode(text));
}

let timeseriesAbortController = null;

export function abortTimeseriesRequest() {
    if (timeseriesAbortController) {
        timeseriesAbortController.abort();
        unregisterAbortController(timeseriesAbortController);
        timeseriesAbortController = null;
    }
}

function showChartLoading() {
    const loadingEl = document.getElementById('chart-loading');
    const emptyEl = document.getElementById('chart-empty-state');
    const canvas = DOM.rainfallChart;

    if (loadingEl) loadingEl.classList.remove('hidden');
    if (emptyEl) emptyEl.classList.add('hidden');
    if (canvas) canvas.style.opacity = '0.3';
}

function hideChartLoading() {
    const loadingEl = document.getElementById('chart-loading');
    const canvas = DOM.rainfallChart;

    if (loadingEl) loadingEl.classList.add('hidden');
    if (canvas) canvas.style.opacity = '1';
}

function showChartEmptyState(message) {
    const emptyEl = document.getElementById('chart-empty-state');
    const emptyText = document.getElementById('chart-empty-text');
    const canvas = DOM.rainfallChart;

    if (emptyText) emptyText.textContent = message || "No forecast data available for this location";
    if (emptyEl) emptyEl.classList.remove('hidden');
    if (canvas) canvas.style.display = 'none';

    if (state.chartInstance) {
        state.chartInstance.destroy();
        state.chartInstance = null;
    }

    if (DOM.chartStatPeak) DOM.chartStatPeak.textContent = "--";
    if (DOM.chartStatTotal) DOM.chartStatTotal.textContent = "--";
}

function hideChartEmptyState() {
    const emptyEl = document.getElementById('chart-empty-state');
    const canvas = DOM.rainfallChart;

    if (emptyEl) emptyEl.classList.add('hidden');
    if (canvas) canvas.style.display = 'block';
}

// Renders the interactive timeseries chart using Chart.js
export async function showTimeseriesChart(lat, lon) {
    if (!state.metadata) return;
    state.activeCoords = { lat, lon };
    syncStateToURL();

    abortTimeseriesRequest();
    timeseriesAbortController = new AbortController();
    registerAbortController(timeseriesAbortController);
    const signal = timeseriesAbortController.signal;
    
    // Show the panel
    DOM.chartPanel.classList.remove('hidden');
    DOM.chartCoords.textContent = `lat: ${lat.toFixed(4)}, lon: ${lon.toFixed(4)}`;
    if (DOM.btnStandaloneLink) {
        DOM.btnStandaloneLink.href = `/graphs.html?lat=${lat.toFixed(4)}&lon=${lon.toFixed(4)}`;
    }
    
    showChartLoading();

    try {
        const data = await fetchTimeseries(state.currentLayerMode, state.currentEns, lat, lon, signal);
        hideChartLoading();
        
        const rawValues = state.currentLayerMode === 'wind' ? (data.speeds || []) : (data.values || []);
        const validValues = rawValues.filter(v => typeof v === 'number' && !isNaN(v) && isFinite(v));

        if (data.status === "out_of_bounds" || rawValues.length === 0 || validValues.length === 0) {
            const outOfBoundsMsg = state.currentLayerMode === 'temp'
                ? "Coordinates are outside temperature coverage grid"
                : (state.currentLayerMode === 'wind' 
                    ? "Coordinates are outside wind coverage grid" 
                    : (state.currentLayerMode === 'solar'
                        ? "Coordinates are outside solar coverage grid"
                        : "Coordinates are outside radar coverage grid"));
            DOM.chartCoords.textContent = `lat: ${lat.toFixed(4)}, lon: ${lon.toFixed(4)} (Out of Grid)`;
            showChartEmptyState(outOfBoundsMsg);
            return;
        }

        hideChartEmptyState();
        
        const peakVal = Math.max(...validValues);
        let totalVal = 0.0;
        
        if (state.currentLayerMode === 'temp') {
            const minVal = Math.min(...validValues);
            DOM.chartStatPeak.textContent = `${peakVal.toFixed(1)} °C`;
            DOM.chartStatTotal.textContent = `${minVal.toFixed(1)} °C`;
            
            DOM.statBox1Label.textContent = "Max Temp";
            DOM.statBox2Label.textContent = "Min Temp";
            setChartHeaderTitle('fa-temperature-half', ' Temperature Forecast Trend');
        } else if (state.currentLayerMode === 'solar') {
            const avgVal = validValues.reduce((a, b) => a + b, 0) / validValues.length;
            DOM.chartStatPeak.textContent = `${Math.round(peakVal)} W/m²`;
            DOM.chartStatTotal.textContent = `${Math.round(avgVal)} W/m² (avg)`;
            
            DOM.statBox1Label.textContent = "Peak Radiation";
            DOM.statBox2Label.textContent = "Avg Radiation";
            setChartHeaderTitle('fa-sun', ' Solar Forecast Trend');
        } else if (state.currentLayerMode === 'wind') {
            const avgVal = validValues.reduce((a, b) => a + b, 0) / validValues.length;
            DOM.chartStatPeak.textContent = `${peakVal.toFixed(1)} m/s`;
            DOM.chartStatTotal.textContent = `${avgVal.toFixed(1)} m/s`;
            
            DOM.statBox1Label.textContent = "Max Wind";
            DOM.statBox2Label.textContent = "Avg Wind";
            setChartHeaderTitle('fa-wind', ' Wind Speed Forecast Trend');
        } else if (state.currentEns === 'prob') {
            DOM.chartStatPeak.textContent = `${Math.round(peakVal)}%`;
            const avgVal = validValues.reduce((a, b) => a + b, 0) / validValues.length;
            DOM.chartStatTotal.textContent = `${Math.round(avgVal)}% (avg)`;
            
            DOM.statBox1Label.textContent = "Peak Probability";
            DOM.statBox2Label.textContent = "Avg Probability";
            setChartHeaderTitle('fa-chart-line', ' Rainfall Forecast Trend');
        } else if (state.currentEns === 'spread') {
            const avgVal = validValues.reduce((a, b) => a + b, 0) / validValues.length;
            DOM.chartStatPeak.textContent = `${peakVal.toFixed(2)} mm/h`;
            DOM.chartStatTotal.textContent = `${avgVal.toFixed(2)} mm/h (avg)`;
            
            DOM.statBox1Label.textContent = "Max Uncertainty";
            DOM.statBox2Label.textContent = "Avg Uncertainty";
            setChartHeaderTitle('fa-chart-line', ' Forecast Uncertainty Trend');
        } else {
            // total_mm = sum(rates) / 12 (5 mins intervals)
            totalVal = validValues.reduce((a, b) => a + b, 0) / 12.0;
            DOM.chartStatPeak.textContent = `${peakVal.toFixed(2)} mm/h`;
            DOM.chartStatTotal.textContent = `${totalVal.toFixed(2)} mm`;
            
            DOM.statBox1Label.textContent = "Peak Intensity";
            DOM.statBox2Label.textContent = "Total Accumulation";
            setChartHeaderTitle('fa-chart-line', ' Rainfall Forecast Trend');
        }
        
        const times = data.times || [];
        const labels = times.map(secs => {
            const timeStr = formatAbsoluteTime(state.metadata.reference_time_str, secs);
            if (state.currentLayerMode === 'temp' || state.currentLayerMode === 'wind' || state.currentLayerMode === 'solar') {
                // Include day for multi-day forecasts, e.g. "Mon 08:00"
                const match = timeStr.match(/(\d{2})\s+(\w+).*?(\d{2}:\d{2})/);
                if (match) {
                    const refMatch = state.metadata.reference_time_str.match(/(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})/);
                    if (refMatch) {
                        const refDate = new Date(`${refMatch[1]}T${refMatch[2]}Z`);
                        const targetDate = new Date(refDate.getTime() + secs * 1000);
                        const dayName = targetDate.toLocaleDateString('en-GB', { timeZone: 'Europe/Amsterdam', weekday: 'short' });
                        return `${dayName} ${match[3]}`;
                    }
                    return `${match[1]} ${match[2]} ${match[3]}`;
                }
            }
            const match = timeStr.match(/(\d{2}:\d{2})/);
            return match ? match[1] : `+${Math.round(secs/60)}m`;
        });
        
        const isProb = state.currentEns === 'prob';
        const isSpread = state.currentEns === 'spread';
        let labelText, borderColor, backgroundColor;
        
        if (state.currentLayerMode === 'temp') {
            labelText = "2m Temperature (°C)";
            borderColor = "#f87171"; // Warm red
            backgroundColor = "rgba(248, 113, 113, 0.15)";
        } else if (state.currentLayerMode === 'solar') {
            labelText = "Solar Radiation (W/m²)";
            borderColor = CONFIG.chart.colors.solar.border;
            backgroundColor = CONFIG.chart.colors.solar.background;
        } else if (state.currentLayerMode === 'wind') {
            labelText = `${state.selectedWindHeight}m Wind Speed (m/s)`;
            borderColor = "#22d3ee"; // Neon cyan
            backgroundColor = "rgba(34, 211, 238, 0.15)";
        } else if (isSpread) {
            labelText = "Rain Uncertainty (mm/h)";
            borderColor = CONFIG.chart.colors.spread.border;
            backgroundColor = CONFIG.chart.colors.spread.background;
        } else {
            labelText = isProb ? CONFIG.radarVisualization.prob.title + " (%)" : CONFIG.radarVisualization.rate.title;
            const chartColors = isProb ? CONFIG.chart.colors.prob : CONFIG.chart.colors.rate;
            borderColor = chartColors.border;
            backgroundColor = chartColors.background;
        }
        
        const ctx = DOM.rainfallChart.getContext('2d');
        
        if (state.chartInstance) {
            state.chartInstance.data.labels = labels;
            state.chartInstance.data.datasets[0].label = labelText;
            state.chartInstance.data.datasets[0].data = rawValues;
            state.chartInstance.data.datasets[0].borderColor = borderColor;
            state.chartInstance.data.datasets[0].backgroundColor = backgroundColor;
            state.chartInstance.data.datasets[0].spanGaps = true;
            state.chartInstance.options.scales.y.title.text = labelText;
            state.chartInstance.options.scales.y.max = (state.currentLayerMode === 'temp' || state.currentLayerMode === 'wind' || state.currentLayerMode === 'solar') ? undefined : (isProb ? 100 : undefined);
            state.chartInstance.options.scales.y.min = (state.currentLayerMode === 'temp' || state.currentLayerMode === 'wind' || state.currentLayerMode === 'solar') ? undefined : 0;
            state.chartInstance.update();
        } else {
            state.chartInstance = new Chart(ctx, {
                type: 'line',
                data: {
                    labels: labels,
                    datasets: [{
                        label: labelText,
                        data: rawValues,
                        borderColor: borderColor,
                        backgroundColor: backgroundColor,
                        borderWidth: CONFIG.chart.borderWidth,
                        fill: true,
                        tension: CONFIG.chart.tension,
                        pointRadius: CONFIG.chart.pointRadius,
                        pointHoverRadius: CONFIG.chart.pointHoverRadius,
                        spanGaps: true
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
                                    const y = context.parsed ? context.parsed.y : null;
                                    if (y === null || y === undefined || isNaN(y)) {
                                        return ' No Data';
                                    }
                                    if (state.currentLayerMode === 'temp') {
                                        return ` ${y.toFixed(1)} °C`;
                                    } else if (state.currentLayerMode === 'solar') {
                                        return ` ${Math.round(y)} W/m²`;
                                    } else if (state.currentLayerMode === 'wind') {
                                        const bft = mpsToBeaufort(y);
                                        return ` ${y.toFixed(1)} m/s (${bft} Bft)`;
                                    } else if (state.currentEns === 'spread') {
                                        return ` ±${y.toFixed(2)} mm/h`;
                                    }
                                    return ` ${y.toFixed(2)}${isProb ? '%' : ' mm/h'}`;
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
                                maxTicksLimit: CONFIG.chart.maxTicksLimit
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
                            min: (state.currentLayerMode === 'temp' || state.currentLayerMode === 'wind') ? undefined : 0,
                            max: (state.currentLayerMode === 'temp' || state.currentLayerMode === 'wind') ? undefined : (isProb ? 100 : undefined)
                        }
                    }
                }
            });
        }
    } catch (e) {
        if (e.name === 'AbortError') return;
        hideChartLoading();
        console.error("Timeseries error:", e);
        DOM.chartCoords.textContent = `lat: ${lat.toFixed(4)}, lon: ${lon.toFixed(4)} (Error)`;
        showChartEmptyState("Failed to load forecast trend for this location");
        showErrorBanner("Could not load location forecast timeseries. Please check your connection.");
    }
}

// Close chart, destroy chart instance and remove MapLibre pin marker
export function closeTimeseriesChart() {
    abortTimeseriesRequest();
    DOM.chartPanel.classList.add('hidden');
    state.activeCoords = null;
    syncStateToURL();
    
    if (state.clickedMarker) {
        state.clickedMarker.remove();
        state.clickedMarker = null;
    }
    
    if (state.chartInstance) {
        state.chartInstance.destroy();
        state.chartInstance = null;
    }
}

