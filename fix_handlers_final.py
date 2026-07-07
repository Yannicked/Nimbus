import re

def fix_file():
    with open('src/handlers.rs', 'r') as f:
        content = f.read()

    # 1. Ensure mutability in signatures
    content = content.replace('Path((ens_str, time))', 'Path((mut ens_str, time))')
    content = content.replace('Path((mut ens_str, time))', 'Path((mut ens_str, time))') # No-op if already fixed
    content = content.replace('Query(q): Query<ValueQuery>', 'Query(mut q): Query<ValueQuery>')
    content = content.replace('Query(q): Query<TimeseriesQuery>', 'Query(mut q): Query<TimeseriesQuery>')

    # 2. Fix get_data_image
    # First lookup
    content = re.sub(
        r'if let Some\(cached_data\) = state\.data_cache\.get\(&\(Cow::Borrowed\(ens_str\.as_str\(\)\), time\)\) \{',
        r'''let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time);
    let cached_res = state.data_cache.get(&key);
    ens_str = key.0.into_owned();
    if let Some(cached_data) = cached_res {''',
        content
    )
    # Second lookup (grid_cache)
    content = re.sub(
        r'let raw_slice = if let Some\(cached\) = state\.grid_cache\.get\(&\(Cow::Borrowed\(ens_str\.as_str\(\)\), time\)\) \{',
        r'''let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time);
    let cached_res = state.grid_cache.get(&key);
    ens_str = key.0.into_owned();
    let raw_slice = if let Some(cached) = cached_res {''',
        content
    )

    # 3. Fix get_value
    # PMM lookup
    content = re.sub(
        r'if q\.ens == "pmm" \{\s+let key: \(Cow<\'static, str>, i64\) = \(Cow::Owned\(q\.ens\), q\.time\);\s+let raw_slice = if let Some\(slice\) = state\.grid_cache\.get\(&key\) \{\s+q\.ens = key\.0\.into_owned\(\);\s+slice\.value\(\)\.clone\(\)\s+\} else \{\s+q\.ens = key\.0\.into_owned\(\);',
        r'''if q.ens == "pmm" {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
        let cached_res = state.grid_cache.get(&key);
        q.ens = key.0.into_owned();
        let raw_slice = if let Some(slice) = cached_res {
            slice.value().clone()
        } else {''',
        content
    )
    # General lookup
    content = re.sub(
        r'// Try reading from cache first\s+let key: \(Cow<\'static, str>, i64\) = \(Cow::Owned\(q\.ens\), q\.time\);\s+if let Some\(slice\) = state\.grid_cache\.get\(&key\) \{\s+q\.ens = key\.0\.into_owned\(\);',
        r'''// Try reading from cache first
    let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
    let cached_res = state.grid_cache.get(&key);
    q.ens = key.0.into_owned();
    if let Some(slice) = cached_res {''',
        content
    )

    # 4. Fix get_timeseries
    # all_cached loop
    # We already have a fix in the file from previous run, let's make sure it's clean.

    # 5. General cleanup of double declarations or leftovers
    content = content.replace('let key: (Cow<\'static, str>, i64) = (Cow::Owned(q.ens), time_val);\n            let key: (Cow<\'static, str>, i64) = (Cow::Owned(q.ens), time_val);',
                              'let key: (Cow<\'static, str>, i64) = (Cow::Owned(q.ens), time_val);')

    # Ensure all insert calls use Cow::Owned(q.ens.clone()) if needed again, or just Cow::Owned(q.ens)
    # In get_timeseries, it's used at the very end to populate the response.

    # Let's check get_timeseries response part
    content = re.sub(
        r'Ok\(axum::Json\(TimeseriesResponse \{\s+status: "ok"\.to_string\(\),\s+lat: q\.lat,\s+lon: q\.lon,\s+ens: q\.ens,',
        r'''Ok(axum::Json(TimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        ens: q.ens.clone(),''',
        content
    )

    with open('src/handlers.rs', 'w') as f:
        f.write(content)

fix_file()
