content = open('src/handlers.rs').read()

# Fix get_value PMM recovery
old_pmm_lookup = """    if q.ens == "pmm" {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
        let raw_slice = if let Some(slice) = state.grid_cache.get(&key) {
            q.ens = key.0.into_owned();
            slice.value().clone()
        } else {
            q.ens = key.0.into_owned();"""

new_pmm_lookup = """    if q.ens == "pmm" {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
        let cached_res = state.grid_cache.get(&key);
        q.ens = key.0.into_owned();
        let raw_slice = if let Some(slice) = cached_res {
            slice.value().clone()
        } else {"""

content = content.replace(old_pmm_lookup, new_pmm_lookup)

# Fix get_timeseries PMM loop recovery
content = content.replace(
    '''            } else {
                q.ens = key.0.into_owned();
                let file_path_clone = file_path.clone();''',
    '''            } else {
                let file_path_clone = file_path.clone();'''
)

content = content.replace(
    '''                let state_clone = state.clone();
                tasks.push(TaskResult::Spawned(tokio::spawn(async move {''',
    '''                let state_clone = state.clone();
                tasks.push(TaskResult::Spawned(tokio::spawn(async move {'''
)

# Actually I need to fix the loop recovery more carefully.
# The PMM loop was:
# for &time_val in &meta.times {
#     let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), time_val);
#     let cached_slice = state.grid_cache.get(&key);
#     q.ens = key.0.into_owned();
#     if let Some(slice) = cached_slice {
#         tasks.push(TaskResult::Cached(slice.value().clone()));
#     } else {
#         q.ens = key.0.into_owned(); // ERROR: already moved and recovered above!

content = content.replace(
    '''            if let Some(slice) = cached_slice {
                tasks.push(TaskResult::Cached(slice.value().clone()));
            } else {
                q.ens = key.0.into_owned();''',
    '''            if let Some(slice) = cached_slice {
                tasks.push(TaskResult::Cached(slice.value().clone()));
            } else {'''
)

with open('src/handlers.rs', 'w') as f:
    f.write(content)
