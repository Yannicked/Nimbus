// App Configuration
export const CONFIG = {
    // Map settings
    map: {
        defaultCenter: [5.2, 52.1], // Center on Netherlands (lon, lat for MapLibre!)
        defaultZoom: 7, // MapLibre zoom 7 roughly matches Leaflet zoom 8
        minZoom: 0,
        maxZoom: 14,
        // Standard CartoDB vector basemaps (Dark Matter and Voyager)
        styles: {
            dark: 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json',
            osm: 'https://basemaps.cartocdn.com/gl/voyager-gl-style/style.json'
        }
    },
    // Application defaults & behavior
    defaults: {
        ensemble: 'med', // Default selected ensemble/statistic ('med', 'max', 'prob', or ensemble member number)
        opacity: 70, // Default radar layer opacity (%)
        speed: 3, // Default playback speed (fps)
        timeIndex: 0 // Default starting time index
    },
    // Timing and performance
    intervals: {
        metadataPollingMs: 5000, // Metadata check interval
        hoverThrottleMs: 100, // Hover values API query throttle
    },
    // Sliding-window layer cache config
    cache: {
        preloadAhead: 2, // Number of future frames to pre-load
    },
    // Legend and visualization colors
    radarVisualization: {
        prob: {
            title: "Rain Probability",
            colors: [
                "rgba(180, 200, 220, 0.0)",
                "rgba(100, 160, 255, 0.5)",
                "rgba(0, 100, 255, 0.65)",
                "rgba(0, 200, 100, 0.75)",
                "rgba(220, 0, 220, 0.85)",
                "rgba(255, 255, 255, 0.95)"
            ],
            labels: ["10%", "30%", "50%", "70%", "90%", "100%"]
        },
        rate: {
            title: "Rainfall Rate (mm/h)",
            colors: [
                "rgba(120, 200, 255, 0.0)",
                "rgba(0, 100, 255, 0.7)",
                "rgba(0, 200, 0, 0.7)",
                "rgba(255, 230, 0, 0.8)",
                "rgba(255, 120, 0, 0.9)",
                "rgba(255, 0, 0, 0.95)",
                "rgba(200, 0, 200, 1.0)",
                "rgba(255, 255, 255, 1.0)"
            ],
            labels: ["0.05", "0.2", "1", "5", "15", "30", "100", "250+"]
        },
        spread: {
            title: "Rain Uncertainty (mm/h)",
            colors: [
                "rgba(99, 102, 241, 0.0)",
                "rgba(99, 102, 241, 0.4)",
                "rgba(168, 85, 247, 0.6)",
                "rgba(236, 72, 153, 0.75)",
                "rgba(244, 63, 94, 0.9)",
                "rgba(255, 255, 255, 0.95)"
            ],
            labels: ["0.05", "0.2", "1.0", "5.0", "15.0", "30.0+"]
        },
        solar: {
            title: "Solar Radiation (W/m²)",
            colors: [
                "rgba(0, 0, 0, 0.0)",
                "rgba(253, 224, 71, 0.3)",
                "rgba(250, 204, 21, 0.5)",
                "rgba(234, 179, 8, 0.7)",
                "rgba(249, 115, 22, 0.85)",
                "rgba(239, 68, 68, 0.95)"
            ],
            labels: ["10", "100", "250", "500", "750", "1000+"]
        }
    },
    // Chart options
    chart: {
        tension: 0.3,
        borderWidth: 2,
        pointRadius: 0,
        pointHoverRadius: 4,
        maxTicksLimit: 6,
        colors: {
            prob: {
                border: "#a855f7",
                background: "rgba(168, 85, 247, 0.15)"
            },
            rate: {
                border: "#38bdf8",
                background: "rgba(56, 189, 248, 0.15)"
            },
            spread: {
                border: "#ec4899",
                background: "rgba(236, 72, 153, 0.15)"
            },
            temp: {
                border: "#10b981",
                background: "rgba(16, 185, 129, 0.15)"
            },
            solar: {
                border: "#f59e0b",
                background: "rgba(245, 158, 11, 0.15)"
            }
        }
    }
};
