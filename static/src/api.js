import { state } from './state.js';

// In-flight active abort controllers set
const activeAbortControllers = new Set();
let errorBannerTimeout = null;

export function registerAbortController(controller) {
    if (controller) activeAbortControllers.add(controller);
    return controller;
}

export function unregisterAbortController(controller) {
    if (controller) activeAbortControllers.delete(controller);
}

/**
 * Abort all active in-flight requests (used when switching layer modes or scrubbing)
 */
export function abortAllPendingRequests() {
    for (const controller of activeAbortControllers) {
        try {
            controller.abort();
        } catch (e) {
            // Ignore abort errors
        }
    }
    activeAbortControllers.clear();
}

/**
 * Visual loading spinner and network progress bar manager
 */
export function startNetworkRequest() {
    state.activeRequests = (state.activeRequests || 0) + 1;
    state.isLoading = true;
    updateLoadingIndicators(true);
}

export function endNetworkRequest() {
    state.activeRequests = Math.max(0, (state.activeRequests || 1) - 1);
    if (state.activeRequests === 0) {
        state.isLoading = false;
        updateLoadingIndicators(false);
    }
}

function updateLoadingIndicators(isLoading) {
    const progressBar = document.getElementById('network-progress-bar');
    const headerSpinner = document.getElementById('header-spinner');

    if (progressBar) {
        if (isLoading) {
            progressBar.classList.remove('hidden');
        } else {
            progressBar.classList.add('hidden');
        }
    }

    if (headerSpinner) {
        if (isLoading) {
            headerSpinner.classList.remove('hidden');
        } else {
            headerSpinner.classList.add('hidden');
        }
    }
}

/**
 * Global friendly error banner notification
 */
export function showErrorBanner(message, duration = 6000) {
    state.currentError = message;
    const banner = document.getElementById('error-banner');
    const textEl = document.getElementById('error-banner-text');
    const closeBtn = document.getElementById('error-banner-close');

    if (banner && textEl) {
        textEl.textContent = message;
        banner.classList.remove('hidden');

        if (closeBtn) {
            closeBtn.onclick = () => hideErrorBanner();
        }

        if (errorBannerTimeout) {
            clearTimeout(errorBannerTimeout);
            errorBannerTimeout = null;
        }

        if (duration > 0) {
            errorBannerTimeout = setTimeout(() => {
                hideErrorBanner();
            }, duration);
        }
    }
}

export function hideErrorBanner() {
    state.currentError = null;
    const banner = document.getElementById('error-banner');
    if (banner) {
        banner.classList.add('hidden');
    }
    if (errorBannerTimeout) {
        clearTimeout(errorBannerTimeout);
        errorBannerTimeout = null;
    }
}

export async function fetchMetadata(layerMode, signal) {
    const endpoints = {
        'temp': '/api/metadata/temp',
        'wind': '/api/metadata/wind',
        'solar': '/api/metadata/solar',
        'rain': '/api/metadata'
    };
    
    startNetworkRequest();
    try {
        const response = await fetch(endpoints[layerMode] || endpoints['rain'], { signal });
        if (!response.ok) {
            let errorDetail = `${layerMode} metadata request failed (HTTP ${response.status})`;
            try {
                const errData = await response.json();
                if (errData.error) errorDetail = errData.error;
            } catch (_) {}
            throw new Error(errorDetail);
        }
        return await response.json();
    } catch (err) {
        if (err.name !== 'AbortError') {
            console.error(`Failed to fetch ${layerMode} metadata:`, err);
        }
        throw err;
    } finally {
        endNetworkRequest();
    }
}

export async function fetchHoverValue(layerMode, ens, time, lat, lon, signal) {
    let url = `/api/value?ens=${ens}&time=${time}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'temp') url = `/api/value/temp?time=${time}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'solar') url = `/api/value/solar?time=${time}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'wind') {
        const h = state.selectedWindHeight || 10;
        url = `/api/value/wind?time=${time}&lat=${lat}&lon=${lon}&height=${h}`;
    }

    try {
        const response = await fetch(url, { signal });
        if (!response.ok) {
            let errorDetail = `Hover value query failed (HTTP ${response.status})`;
            try {
                const errData = await response.json();
                if (errData.error) errorDetail = errData.error;
            } catch (_) {}
            throw new Error(errorDetail);
        }
        return await response.json();
    } catch (err) {
        if (err.name !== 'AbortError') {
            console.error("Hover value fetch failed:", err);
        }
        throw err;
    }
}

export async function fetchTimeseries(layerMode, ens, lat, lon, signal) {
    let url = `/api/timeseries?ens=${ens}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'temp') url = `/api/timeseries/temp?lat=${lat}&lon=${lon}`;
    if (layerMode === 'solar') url = `/api/timeseries/solar?lat=${lat}&lon=${lon}`;
    if (layerMode === 'wind') {
        const h = state.selectedWindHeight || 10;
        url = `/api/timeseries/wind?lat=${lat}&lon=${lon}&height=${h}`;
    }

    startNetworkRequest();
    try {
        const response = await fetch(url, { signal });
        if (!response.ok) {
            let errorDetail = `Timeseries query failed (HTTP ${response.status})`;
            try {
                const errData = await response.json();
                if (errData.error) errorDetail = errData.error;
            } catch (_) {}
            throw new Error(errorDetail);
        }
        return await response.json();
    } catch (err) {
        if (err.name !== 'AbortError') {
            console.error("Timeseries fetch failed:", err);
        }
        throw err;
    } finally {
        endNetworkRequest();
    }
}


