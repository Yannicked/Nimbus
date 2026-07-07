import re

content = open('src/handlers.rs').read()

# Fix get_value second lookup
old_get_value_cache = """    // Try reading from cache first
    let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
    if let Some(slice) = state.grid_cache.get(&key) {
        q.ens = key.0.into_owned();
        let val_raw = slice[iy as usize * KNMI_GRID_W + ix as usize];
        let (status_out, value_out) = if q.ens == "prob" {
            ("probability".to_string(), val_raw as f64)
        } else {
            let val_mmh = raw_to_value(val_raw);
            let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
            (status.to_string(), val_mmh)
        };
        return Ok(axum::Json(ValueResponse {
            status: status_out,
            value: Some(value_out),
        }));
    }"""

new_get_value_cache = """    // Try reading from cache first
    let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
    let cached_slice = state.grid_cache.get(&key);
    q.ens = key.0.into_owned();
    if let Some(slice) = cached_slice {
        let val_raw = slice[iy as usize * KNMI_GRID_W + ix as usize];
        let (status_out, value_out) = if q.ens == "prob" {
            ("probability".to_string(), val_raw as f64)
        } else {
            let val_mmh = raw_to_value(val_raw);
            let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
            (status.to_string(), val_mmh)
        };
        return Ok(axum::Json(ValueResponse {
            status: status_out,
            value: Some(value_out),
        }));
    }"""

content = content.replace(old_get_value_cache, new_get_value_cache)

# Fix get_timeseries loops
# First the all_cached check loop
content = re.sub(
    r'let mut all_cached = true;\s+for &time_val in &meta\.times \{\s+if !state\s+\.grid_cache\s+\.contains_key\(&\(Cow::Borrowed\(q\.ens\.as_str\(\)\), time_val\)\)\s+\{\s+all_cached = false;\s+break;\s+\}\s+\}',
    r'''let mut all_cached = true;
    let mut ens_str = std::mem::take(&mut q.ens);
    for &time_val in &meta.times {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time_val);
        let found = state.grid_cache.contains_key(&key);
        ens_str = key.0.into_owned();
        if !found {
            all_cached = false;
            break;
        }
    }
    q.ens = ens_str;''',
    content
)

# Fix cached_ts lookup
content = re.sub(
    r'if all_cached \{\s+let key: \(Cow<\'static, str>, i32, i32\) = \(Cow::Owned\(q\.ens\), ix, iy\);\s+if let Some\(cached_ts\) = state\.timeseries_cache\.get\(&key\) \{\s+q\.ens = key\.0\.into_owned\(\);\s+values\.extend_from_slice\(&cached_ts\);\s+\} else \{\s+q\.ens = key\.0\.into_owned\(\);',
    r'''if all_cached {
        let key: (Cow<'static, str>, i32, i32) = (Cow::Owned(q.ens), ix, iy);
        let cached_res = state.timeseries_cache.get(&key);
        q.ens = key.0.into_owned();
        if let Some(cached_ts) = cached_res {
            values.extend_from_slice(&cached_ts);
        } else {''',
    content
)

# Fix ts_values loop
content = re.sub(
    r'for &time_val in &meta\.times \{\s+let key = \(Cow::Owned\(q\.ens\), time_val\);\s+if let Some\(slice\) = state\.grid_cache\.get\(&key\) \{\s+q\.ens = key\.0\.into_owned\(\);',
    r'''for &time_val in &meta.times {
                let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), time_val);
                let cached_slice = state.grid_cache.get(&key);
                q.ens = key.0.into_owned();
                if let Some(slice) = cached_slice {''',
    content
)

# Fix pmm loop
content = re.sub(
    r'for &time_val in &meta\.times \{\s+let key: \(Cow<\'static, str>, i64\) = \(Cow::Owned\(q\.ens\), time_val\);\s+if let Some\(slice\) = state\.grid_cache\.get\(&key\) \{\s+q\.ens = key\.0\.into_owned\(\);\s+tasks\.push\(TaskResult::Cached\(slice\.value\(\)\.clone\(\)\)\);',
    r'''for &time_val in &meta.times {
            let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), time_val);
            let cached_slice = state.grid_cache.get(&key);
            q.ens = key.0.into_owned();
            if let Some(slice) = cached_slice {
                tasks.push(TaskResult::Cached(slice.value().clone()));''',
    content
)

# Fix q_ens_clone before it is used
content = content.replace('let q_ens_clone = q.ens.clone();', 'let q_ens_clone = q.ens.clone(); // Re-clone for moved/recovered value')

with open('src/handlers.rs', 'w') as f:
    f.write(content)
