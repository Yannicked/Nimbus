content = open('src/handlers.rs').read()

# The ens_clone = q.ens.clone() is fine because q.ens was recovered just before the if cached_slice block.
# However, if we move it into tokio::spawn, we need it to be cloned anyway.
# The current code:
# q.ens = key.0.into_owned();
# if let Some(slice) = cached_slice { ... } else { ... ens_clone = q.ens.clone(); ... }
# is correct regarding moves, but q.ens must be mutable.

content = content.replace(
    'Query(mut q): Query<ValueQuery>',
    'Query(mut q): Query<ValueQuery>'
)
# Handled by previous sed.

with open('src/handlers.rs', 'w') as f:
    f.write(content)
