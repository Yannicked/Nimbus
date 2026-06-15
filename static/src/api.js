import { state } from './state.js';

export async function fetchMetadata(layerMode) {
    const endpoints = {
        'temp': '/api/metadata/temp',
        'wind': '/api/metadata/wind',
        'rain': '/api/metadata'
    };
    
    const response = await fetch(endpoints[layerMode] || endpoints['rain']);
    if (!response.ok) throw new Error(`${layerMode} metadata request failed`);
    return response.json();
}

export async function fetchHoverValue(layerMode, ens, time, lat, lon) {
    let url = `/api/value?ens=${ens}&time=${time}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'temp') url = `/api/value/temp?time=${time}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'wind') {
        const h = state.selectedWindHeight || 10;
        url = `/api/value/wind?time=${time}&lat=${lat}&lon=${lon}&height=${h}`;
    }

    const response = await fetch(url);
    if (!response.ok) throw new Error("Hover value query failed");
    return response.json();
}

export async function fetchTimeseries(layerMode, ens, lat, lon) {
    let url = `/api/timeseries?ens=${ens}&lat=${lat}&lon=${lon}`;
    if (layerMode === 'temp') url = `/api/timeseries/temp?lat=${lat}&lon=${lon}`;
    if (layerMode === 'wind') {
        const h = state.selectedWindHeight || 10;
        url = `/api/timeseries/wind?lat=${lat}&lon=${lon}&height=${h}`;
    }

    const response = await fetch(url);
    if (!response.ok) throw new Error("Timeseries query failed");
    return response.json();
}
