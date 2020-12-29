use clru::CLruCache;
use std::sync::Arc;
use wasmer::{Artifact, Module, Store};

use crate::{Checksum, Size, VmResult};

const ESTIMATED_MODULE_SIZE: Size = Size::mebi(10);

/// An in-memory module cache
pub struct InMemoryCache {
    // Keep the original store in cache due to https://github.com/wasmerio/wasmer/issues/1943.
    // We never re-use it.
    artifacts: CLruCache<Checksum, (Arc<dyn Artifact>, Store)>,
}

impl InMemoryCache {
    /// Creates a new cache with the given size (in bytes)
    pub fn new(size: Size) -> Self {
        let max_entries = size.0 / ESTIMATED_MODULE_SIZE.0;
        InMemoryCache {
            artifacts: CLruCache::new(max_entries),
        }
    }

    pub fn store(&mut self, checksum: &Checksum, module: Module) -> VmResult<()> {
        let artifact = Arc::clone(module.artifact());
        let store = module.store().clone();
        self.artifacts.put(*checksum, (artifact, store));
        Ok(())
    }

    /// Looks up a module in the cache and takes its artifact and
    /// creates a new module from store and artifact.
    pub fn load(&mut self, checksum: &Checksum, store: &Store) -> VmResult<Option<Module>> {
        match self.artifacts.get(checksum) {
            Some((artifact, _store)) => {
                let new_module = Module::from_artifact(store, Arc::clone(artifact));
                Ok(Some(new_module))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::Size;
    use crate::wasm_backend::{compile_only, make_runtime_store};
    use wasmer::{imports, Instance as WasmerInstance};
    use wasmer_middlewares::metering::set_remaining_points;

    const TESTING_MEMORY_LIMIT: Size = Size::mebi(16);
    const TESTING_GAS_LIMIT: u64 = 5_000;

    #[test]
    fn in_memory_cache_run() {
        let mut cache = InMemoryCache::new(Size::mebi(200));

        // Create module
        let wasm = wat::parse_str(
            r#"(module
            (type $t0 (func (param i32) (result i32)))
            (func $add_one (export "add_one") (type $t0) (param $p0 i32) (result i32)
                get_local $p0
                i32.const 1
                i32.add)
            )"#,
        )
        .unwrap();
        let checksum = Checksum::generate(&wasm);

        // Module does not exist
        let store = make_runtime_store(TESTING_MEMORY_LIMIT);
        let cache_entry = cache.load(&checksum, &store).unwrap();
        assert!(cache_entry.is_none());

        // Compile module
        let original = compile_only(&wasm).unwrap();

        // Ensure original module can be executed
        {
            let instance = WasmerInstance::new(&original, &imports! {}).unwrap();
            set_remaining_points(&instance, TESTING_GAS_LIMIT);
            let add_one = instance.exports.get_function("add_one").unwrap();
            let result = add_one.call(&[42.into()]).unwrap();
            assert_eq!(result[0].unwrap_i32(), 43);
        }

        // Store module
        cache.store(&checksum, original).unwrap();

        // Load module
        let store = make_runtime_store(TESTING_MEMORY_LIMIT);
        let cached = cache.load(&checksum, &store).unwrap().unwrap();

        // Ensure cached module can be executed
        {
            let instance = WasmerInstance::new(&cached, &imports! {}).unwrap();
            set_remaining_points(&instance, TESTING_GAS_LIMIT);
            let add_one = instance.exports.get_function("add_one").unwrap();
            let result = add_one.call(&[42.into()]).unwrap();
            assert_eq!(result[0].unwrap_i32(), 43);
        }
    }
}
