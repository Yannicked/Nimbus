import { fetchTimeseries } from './api.js';

// Time formatting helpers to make the notification text clean and readable
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

/**
 * Requests notification permission from the browser.
 * Returns true if permission is granted, false otherwise.
 */
export async function requestNotificationPermission() {
    if (!("Notification" in window)) {
        console.warn("Notifications not supported in this browser.");
        return false;
    }
    if (Notification.permission === "granted") {
        return true;
    }
    if (Notification.permission !== "denied") {
        const permission = await Notification.requestPermission();
        return permission === "granted";
    }
    return false;
}

/**
 * Saves alert settings to localStorage.
 */
export function saveNotificationPrefs(lat, lon, threshold, emailEnabled, emailAddr, pushEnabled) {
    localStorage.setItem('nimbus_notify_lat', lat);
    localStorage.setItem('nimbus_notify_lon', lon);
    localStorage.setItem('nimbus_rain_threshold', threshold);
    localStorage.setItem('nimbus_notify_email', emailEnabled);
    localStorage.setItem('nimbus_email', emailAddr);
    localStorage.setItem('nimbus_notify_push', pushEnabled);
}

/**
 * Main routine to query the forecast and trigger a native notification if rain starts.
 * Utilizes a version-based deduplication key to notify once per forecast run.
 */
export async function checkRainAndNotify(rainMeta) {
    // 1. Check if notifications are enabled
    const pushEnabled = localStorage.getItem('nimbus_notify_push') === 'true';
    if (!pushEnabled) return;
    
    if (Notification.permission !== "granted") {
        console.warn("Rain alerts enabled, but browser Notification permission not granted.");
        return;
    }
    
    // 2. Retrieve saved coordinates and threshold
    const latStr = localStorage.getItem('nimbus_notify_lat');
    const lonStr = localStorage.getItem('nimbus_notify_lon');
    if (!latStr || !lonStr) {
        console.warn("Rain alerts enabled, but no target coordinates saved.");
        return;
    }
    
    const lat = parseFloat(latStr);
    const lon = parseFloat(lonStr);
    const threshold = parseFloat(localStorage.getItem('nimbus_rain_threshold') || '1.0');
    
    // 3. Check deduplication key (so we only notify once per forecast run version)
    const lastNotifiedVersion = localStorage.getItem('nimbus_last_notified_version');
    const currentVersion = rainMeta ? rainMeta.version : null;
    
    if (currentVersion && lastNotifiedVersion === currentVersion) {
        // Already notified the user about this forecast run
        return;
    }
    
    try {
        // 4. Fetch rain timeseries
        const ts = await fetchTimeseries('rain', 'pmm', lat, lon);
        if (ts.status === 'out_of_bounds' || !ts.values || !ts.values.length) {
            return;
        }
        
        const times = ts.times || [];
        const vals = (ts.values || []).map(v => (typeof v === 'number' && !isNaN(v)) ? v : 0);
        
        // 5. Check if rain exceeds threshold
        let rainStartIndex = -1;
        for (let i = 0; i < vals.length; i++) {
            if (vals[i] >= threshold) {
                rainStartIndex = i;
                break;
            }
        }
        
        if (rainStartIndex !== -1) {
            // Find duration of rain event
            let rainEndIndex = rainStartIndex;
            for (let j = rainStartIndex; j < vals.length; j++) {
                if (vals[j] >= threshold) {
                    rainEndIndex = j;
                } else {
                    // Allow brief dips of up to 10 minutes (2 indices) before declaring event ended
                    let stillRaining = false;
                    for (let k = j; k < Math.min(j + 3, vals.length); k++) {
                        if (vals[k] >= threshold) {
                            stillRaining = true;
                            break;
                        }
                    }
                    if (!stillRaining) {
                        break;
                    }
                }
            }
            
            const startSecs = times[rainStartIndex] !== undefined ? times[rainStartIndex] : 0;
            const endSecs = (times[rainEndIndex] !== undefined ? times[rainEndIndex] : startSecs) + 300; // include the full step
            const durationSecs = endSecs - startSecs;
            const durationMins = Math.max(5, Math.round(durationSecs / 60));
            
            const sub = vals.slice(rainStartIndex, rainEndIndex + 1).filter(v => typeof v === 'number' && !isNaN(v));
            const peak = sub.length > 0 ? Math.max(...sub) : threshold;
            const startRel = formatRelativeTime(startSecs);
            
            let refTimeStr = rainMeta ? rainMeta.reference_time_str : null;
            if (!refTimeStr) {
                refTimeStr = new Date().toISOString().replace('T', ' ').substring(0, 19);
            }
            const startAbs = formatAbsoluteTime(refTimeStr, startSecs);
            
            // 6. Trigger native notification
            new Notification(`Rain Alert: In ${startRel}`, {
                body: `Rain starting at ${startAbs} for ${durationMins} minutes. Peak intensity: ${peak.toFixed(2)} mm/h.`,
                tag: 'nimbus-rain-alert'
            });
            
            // 7. Save version to avoid duplicate notification for the same forecast run
            if (currentVersion) {
                localStorage.setItem('nimbus_last_notified_version', currentVersion);
            }
        }
    } catch (e) {
        console.error("Failed to execute rain notification check:", e);
    }
}
